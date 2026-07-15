use clap::Parser;
use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::Arc;
use zhiyuan_core::{LlmClient, ResearchConfig, ResearchQuery};
use zhiyuan_orchestrator::ResearchOrchestrator;
use zhiyuan_search::EnginePool;

mod llm;
use llm::OpenaiLlm;

#[derive(Parser)]
#[command(name = "zhiyuan", version, about = "致远 - 深度研究框架")]
struct Cli {
    /// 研究问题
    query: String,

    /// 研究前 LLM 生成澄清问题并等待用户回答
    #[arg(long, default_value_t = true)]
    clarify: bool,

    /// 启用长报告模式（多章节结构报告）
    #[arg(long)]
    long: bool,

    /// 任务并发数
    #[arg(long, default_value_t = 4)]
    concurrency: usize,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    dotenvy::dotenv().ok();

    let hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        cli.query.hash(&mut hasher);
        hasher.finish()
    };

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let log_dir = Path::new(&home).join(".local/share/zhiyuan");
    std::fs::create_dir_all(&log_dir).ok();
    let log_path = log_dir.join(format!("{:016x}.log", hash));
    let log_file = std::fs::File::create(&log_path)
        .map_err(|e| anyhow::anyhow!("创建日志文件失败 {}: {e}", log_path.display()))?;

    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(log_file))
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    tracing::info!(query = %cli.query, hash = %format!("{:016x}", hash), "session started");

    let data_dir = format!("{home}/.cache/zhiyuan/{:016x}", hash);

    let mut config = load_config()?;
    config.research.long_report = cli.long;
    config.research.concurrency = cli.concurrency;
    if cli.long {
        config.research.cross_validate = true;
    }

    let engine_pool = Arc::new(EnginePool::from_config(&config.search));

    if config.llm.api_key.is_empty() {
        anyhow::bail!(
            "未配置 LLM API 密钥。请创建配置文件，在 [llm] 中设置 api_key。\n\
             参考：https://platform.openai.com/api-keys"
        );
    }
    let llm: Box<dyn LlmClient> = Box::new(OpenaiLlm::new(
        config.llm.api_key.clone(),
        config.llm.base_url.clone(),
        config.llm.main_model.clone(),
    ));

    let clarification = if cli.clarify {
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
    };

    tracing::info!("Starting research: {}", query.full_query());
    let report = orchestrator.research(query).await?;

    if cli.long {
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

fn load_config() -> anyhow::Result<ResearchConfig> {
    let home = std::env::var("HOME").unwrap_or_default();
    let user_config = Path::new(&home).join(".config/zhiyuan.toml");
    let config_path = if user_config.exists() {
        user_config
    } else {
        let local = Path::new("zhiyuan.toml");
        if local.exists() {
            local.to_path_buf()
        } else {
            anyhow::bail!(
                "未找到配置文件。请创建 ~/.config/zhiyuan.toml 或 ./zhiyuan.toml。\n\
                 参考项目中的 zhiyuan.toml.example"
            );
        }
    };

    let config_str = std::fs::read_to_string(&config_path)
        .map_err(|e| anyhow::anyhow!("读取配置文件失败 {}: {e}", config_path.display()))?;

    let config: ResearchConfig = toml::from_str(&config_str)
        .map_err(|e| anyhow::anyhow!("解析配置文件失败: {e}"))?;

    Ok(config)
}
