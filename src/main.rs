use clap::Parser;
use std::io::{BufRead, Write};
use std::sync::Arc;
use zhiyuan_core::{LlmClient, ResearchConfig, ResearchQuery, ResearchSettings};
use zhiyuan_orchestrator::ResearchOrchestrator;
use zhiyuan_search::EnginePool;

mod llm;
use llm::OpenaiLlm;

#[derive(Parser)]
#[command(name = "zhiyuan", version, about = "致远 - 深度研究框架")]
struct Cli {
    /// 研究问题
    #[arg(short, long)]
    query: String,

    /// 交互模式：研究前通过提问澄清问题
    #[arg(long, default_value = "true")]
    interactive: bool,

    /// 长报告模式：多章节结构报告
    #[arg(long)]
    long_report: bool,

    /// 长报告最大章节数
    #[arg(long, default_value = "6")]
    max_chapters: usize,

    /// 质量阈值 (0.0 - 1.0)
    #[arg(long, default_value = "0.7")]
    quality_threshold: f64,

    /// 最大迭代次数
    #[arg(long, default_value = "10")]
    max_iterations: usize,

    /// 搜索广度（每层查询数）
    #[arg(long, default_value = "4")]
    breadth: usize,

    /// 搜索深度（递归层数）
    #[arg(long, default_value = "3")]
    depth: usize,

    /// 并发数
    #[arg(long, default_value = "4")]
    concurrency: usize,

    /// 交叉搜索验证：多引擎并行搜索，自动去重合并
    #[arg(long)]
    cross_validate: bool,

    /// 多语言搜索：自动补充英文查询以覆盖技术术语
    #[arg(long)]
    search_in_english: bool,

    /// 配置文件路径
    #[arg(short, long)]
    config: Option<String>,

    /// 数据目录（记忆存储，默认 ~/.cache/zhiyuan/<query_hash>）
    #[arg(short, long)]
    data_dir: Option<String>,

    /// 输出文件
    #[arg(short, long)]
    output: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();
    dotenvy::dotenv().ok();

    let data_dir = cli.data_dir.clone().unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        cli.query.hash(&mut hasher);
        let hash = hasher.finish();
        format!("{home}/.cache/zhiyuan/{:016x}", hash)
    });

    let config = load_config(&cli, &data_dir)?;

    let engine_pool = Arc::new(EnginePool::from_config(&config.search));

    let llm: Box<dyn LlmClient> = Box::new(OpenaiLlm::from_env()?);

    let clarification = if cli.interactive {
        let planner = zhiyuan_agents::PlannerAgent::new(llm.clone_box());
        match planner.generate_clarifying_questions(&cli.query).await {
            Ok(questions) if !questions.is_empty() => {
                println!("\n=== 研究问题澄清 ===\n");
                println!("您的研究问题：{}\n", cli.query);
                println!("请回答以下问题以精炼研究方向（直接回车跳过）：\n");

                let stdin = std::io::stdin();
                let mut answers = Vec::new();
                for (i, question) in questions.iter().enumerate() {
                    print!("{}. {}: ", i + 1, question);
                    std::io::stdout().flush().ok();
                    let mut input = String::new();
                    stdin.lock().read_line(&mut input).ok();
                    let answer = input.trim().to_string();
                    if !answer.is_empty() {
                        answers.push(format!("{question} {answer}"));
                    }
                }

                if answers.is_empty() {
                    None
                } else {
                    Some(answers.join("\n"))
                }
            }
            _ => None,
        }
    } else {
        None
    };

    let orchestrator = ResearchOrchestrator::new(
        llm,
        engine_pool,
        config.research,
        Some(data_dir),
    ).await;

    let query = ResearchQuery {
        id: zhiyuan_core::Uuid::new_v4(),
        query: cli.query.clone(),
        clarification,
        breadth: cli.breadth,
        depth: cli.depth,
        max_iterations: cli.max_iterations,
        quality_threshold: cli.quality_threshold,
        cost_budget_usd: 1.0,
    };

    tracing::info!("Starting research: {}", query.full_query());
    let report = orchestrator.research(query).await?;

    let report_json = serde_json::to_string_pretty(&report)?;

    if let Some(path) = cli.output {
        std::fs::write(&path, &report_json)?;
        tracing::info!("Report written to {path}");
    } else if cli.long_report {
        println!("# {}\n", report.title);
        for section in &report.sections {
            if !section.content.is_empty() {
                println!("{}", section.content);
            }
        }
        println!("\n---\n");
        println!(
            "质量评分: {:.2} (覆盖率: {:.2}, 可靠性: {:.2}, 时效性: {:.2}, 深度: {:.2})",
            report.quality_score.overall,
            report.quality_score.coverage,
            report.quality_score.reliability,
            report.quality_score.freshness,
            report.quality_score.depth,
        );
    } else {
        for section in &report.sections {
            println!("# {}\n", section.heading);
            println!("{}\n", section.content);
        }
        println!("---\n");
        println!(
            "质量评分: {:.2} (覆盖率: {:.2}, 可靠性: {:.2}, 时效性: {:.2}, 深度: {:.2})",
            report.quality_score.overall,
            report.quality_score.coverage,
            report.quality_score.reliability,
            report.quality_score.freshness,
            report.quality_score.depth,
        );
    }

    Ok(())
}

fn load_config(cli: &Cli, data_dir: &str) -> anyhow::Result<ResearchConfig> {
    let config_path = cli
        .config
        .clone()
        .unwrap_or_else(|| "config/default.toml".into());

    let config_str = std::fs::read_to_string(&config_path).unwrap_or_default();

    let mut config: ResearchConfig = toml::from_str(&config_str).unwrap_or_else(|_| ResearchConfig {
        search: zhiyuan_core::SearchConfig {
            bing_api_key: std::env::var("BING_API_KEY").unwrap_or_default(),
            bing_endpoint: std::env::var("BING_ENDPOINT")
                .unwrap_or_else(|_| "https://api.bing.microsoft.com/v7.0/search".into()),
            google_api_key: std::env::var("GOOGLE_API_KEY").unwrap_or_default(),
            google_cse_id: std::env::var("GOOGLE_CSE_ID").unwrap_or_default(),
            ddg_max_results: 10,
            request_timeout_secs: 10,
            cross_validate: cli.cross_validate,
            search_in_english: cli.search_in_english,
        },
        llm: zhiyuan_core::LlmConfig {
            reasoning_model: std::env::var("REASONING_MODEL")
                .unwrap_or_else(|_| "gpt-4o".into()),
            reasoning_provider: std::env::var("REASONING_PROVIDER")
                .unwrap_or_else(|_| "openai".into()),
            main_model: std::env::var("MAIN_MODEL").unwrap_or_else(|_| "gpt-4o".into()),
            main_provider: std::env::var("MAIN_PROVIDER").unwrap_or_else(|_| "openai".into()),
            fast_model: std::env::var("FAST_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
            fast_provider: std::env::var("FAST_PROVIDER").unwrap_or_else(|_| "openai".into()),
        },
        research: ResearchSettings {
            max_iterations: cli.max_iterations,
            quality_threshold: cli.quality_threshold,
            breadth: cli.breadth,
            depth: cli.depth,
            concurrency: cli.concurrency,
            cost_budget_usd: 1.0,
            long_report: cli.long_report,
            max_chapters: cli.max_chapters,
            cross_validate: cli.cross_validate,
            search_in_english: cli.search_in_english,
        },
        memory: zhiyuan_core::MemoryConfig {
            db_path: data_dir.to_string(),
        },
    });

    if let Ok(key) = std::env::var("BING_API_KEY") {
        config.search.bing_api_key = key;
    }
    if let Ok(key) = std::env::var("GOOGLE_API_KEY") {
        config.search.google_api_key = key;
    }
    if let Ok(key) = std::env::var("GOOGLE_CSE_ID") {
        config.search.google_cse_id = key;
    }

    Ok(config)
}
