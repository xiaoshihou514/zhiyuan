use clap::Parser;
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

    /// 配置文件路径
    #[arg(short, long)]
    config: Option<String>,

    /// 数据目录（记忆存储）
    #[arg(short, long, default_value = "./zhiyuan_data")]
    data_dir: String,

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

    let config = load_config(&cli)?;

    let engine_pool = Arc::new(EnginePool::from_config(&config.search));

    let llm: Box<dyn LlmClient> = Box::new(OpenaiLlm::from_env()?);

    let orchestrator = ResearchOrchestrator::new(
        llm,
        engine_pool,
        config.research,
        Some(cli.data_dir),
    ).await;

    let query = ResearchQuery {
        id: zhiyuan_core::Uuid::new_v4(),
        query: cli.query.clone(),
        clarification: None,
        breadth: cli.breadth,
        depth: cli.depth,
        max_iterations: cli.max_iterations,
        quality_threshold: cli.quality_threshold,
        cost_budget_usd: 1.0,
    };

    tracing::info!("Starting research: {}", query.query);
    let report = orchestrator.research(query).await?;

    let report_json = serde_json::to_string_pretty(&report)?;

    if let Some(path) = cli.output {
        std::fs::write(&path, &report_json)?;
        tracing::info!("Report written to {path}");
    } else {
        for section in &report.sections {
            println!("# {}\n", section.heading);
            println!("{}\n", section.content);
        }
        println!("---");
        println!(
            "Quality score: {:.2} (coverage: {:.2}, reliability: {:.2}, freshness: {:.2}, depth: {:.2})",
            report.quality_score.overall,
            report.quality_score.coverage,
            report.quality_score.reliability,
            report.quality_score.freshness,
            report.quality_score.depth,
        );
    }

    Ok(())
}

fn load_config(cli: &Cli) -> anyhow::Result<ResearchConfig> {
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
        },
        memory: zhiyuan_core::MemoryConfig {
            db_path: cli.data_dir.clone(),
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
