use std::io::Write as IoWrite;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::Parser;
use tuirealm::application::{Application, PollStrategy};
use tuirealm::event::NoUserEvent;
use tuirealm::listener::EventListenerCfg;
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter};
use zhiyuan_core::{
    LlmClient, ProgressReporter, ProgressUpdate, ResearchConfig, ResearchPlan, ResearchQuery,
};
use zhiyuan_orchestrator::ResearchOrchestrator;
use zhiyuan_search::EnginePool;

mod llm;
use llm::OpenaiLlm;

mod pdf;
mod tui;
use tui::{App, Id, Msg, TuiEvent};

struct DualWriter {
    file: Arc<Mutex<std::fs::File>>,
    tx: tokio::sync::mpsc::UnboundedSender<TuiEvent>,
}

impl Clone for DualWriter {
    fn clone(&self) -> Self {
        Self {
            file: self.file.clone(),
            tx: self.tx.clone(),
        }
    }
}

impl IoWrite for DualWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.file.lock().unwrap().write_all(buf).ok();
        if let Ok(s) = String::from_utf8(buf.to_vec()) {
            let _ = self.tx.send(TuiEvent::LogLine(s));
        }
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.file.lock().unwrap().flush()
    }
}

struct ChannelReporter {
    tx: tokio::sync::mpsc::UnboundedSender<TuiEvent>,
}

impl ProgressReporter for ChannelReporter {
    fn report(&self, update: ProgressUpdate) {
        let _ = self.tx.send(TuiEvent::Progress(update));
    }
}

#[derive(Parser)]
#[command(name = "zhiyuan", version, about = "致远 - 深度研究框架")]
struct Cli {
    query: String,
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    clarify: bool,
    #[arg(long)]
    long: bool,
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
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros())
            .unwrap_or(0)
            .hash(&mut hasher);
        hasher.finish()
    };

    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let base_dir = Path::new(&home).join(".local/share/zhiyuan");
    let session_dir = base_dir.join(format!("{:016x}", hash));
    std::fs::create_dir_all(&session_dir)?;
    let searches_dir = session_dir.join("searches");
    std::fs::create_dir_all(&searches_dir)?;

    let log_path = session_dir.join("session.log");
    let log_file = std::fs::File::create(&log_path)
        .map_err(|e| anyhow::anyhow!("创建日志文件失败 {}: {e}", log_path.display()))?;

    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel::<TuiEvent>();
    let (research_trigger_tx, research_trigger_rx) =
        tokio::sync::oneshot::channel::<(ResearchQuery, Option<ResearchPlan>)>();

    {
        let dual = std::sync::Mutex::new(DualWriter {
            file: Arc::new(Mutex::new(log_file)),
            tx: event_tx.clone(),
        });
        tracing_subscriber::fmt()
            .with_writer(dual)
            .with_ansi(false)
            .with_target(false)
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| "info".into()),
            )
            .init();
    }

    tracing::info!("查询" = %cli.query, "哈希" = %format!("{:016x}", hash), "会话开始");

    let data_dir = format!("{home}/.cache/zhiyuan/{:016x}", hash);
    let llm_log = session_dir.join("llm.log");

    let mut config = load_config()?;
    config.research.long_report = cli.long;
    config.research.concurrency = cli.concurrency;
    if cli.long {
        config.research.cross_validate = true;
    }

    let engine_pool = Arc::new(EnginePool::from_config(&config.search, Some(searches_dir)));

    if config.llm.api_key.is_empty() {
        tracing::warn!("LLM API 密钥为空，将不发送 Authorization 请求头");
    }
    let llm: Box<dyn LlmClient> = Box::new(OpenaiLlm::new(
        config.llm.api_key.clone(),
        config.llm.base_url.clone(),
        config.llm.main_model.clone(),
        Some(llm_log.to_string_lossy().to_string()),
    ));

    if cli.clarify {
        let tx = event_tx.clone();
        let llm_clone = llm.clone_box();

        let inner_query = ResearchQuery::new(cli.query.clone());
        let rs = config.research.clone();
        tokio::spawn(async move {
            let planner = zhiyuan_agents::PlannerAgent::new(llm_clone);
            match planner.create_plan(&inner_query, &rs).await {
                Ok(plan) => {
                    tx.send(TuiEvent::PlanReady(plan)).ok();
                }
                Err(_) => {
                    tx.send(TuiEvent::PlanReady(ResearchPlan {
                        query_id: inner_query.id,
                        sub_tasks: vec![],
                        outline: None,
                    }))
                    .ok();
                }
            }
        });
    }

    {
        let tx = event_tx.clone();
        let llm = llm.clone_box();
        let engine_pool = engine_pool.clone();
        let config_research = config.research.clone();
        let data_dir = data_dir.clone();

        tokio::spawn(async move {
            let (query, plan) = match research_trigger_rx.await {
                Ok(pair) => pair,
                Err(_) => return,
            };
            let reporter = ChannelReporter { tx: tx.clone() };
            let orchestrator = ResearchOrchestrator::new(
                llm,
                engine_pool,
                config_research,
                Some(data_dir),
                Some(Box::new(reporter)),
            )
            .await;
            match orchestrator.research(query, plan).await {
                Ok(report) => {
                    tx.send(TuiEvent::Progress(ProgressUpdate::Report(report)))
                        .ok();
                }
                Err(e) => {
                    tx.send(TuiEvent::Progress(ProgressUpdate::Error(e.to_string())))
                        .ok();
                }
            }
        });
    }

    let research_trigger = if cli.clarify {
        research_trigger_tx
    } else {
        let query = ResearchQuery::new(cli.query.clone());
        let _ = research_trigger_tx.send((query, None));
        let (dummy, _) = tokio::sync::oneshot::channel();
        dummy
    };

    let mut app: Application<Id, Msg, NoUserEvent> = Application::init(
        EventListenerCfg::default()
            .crossterm_input_listener(Duration::from_millis(50), 1)
            .tick_interval(Duration::from_millis(100)),
    );

    app.mount(
        Id::App,
        Box::new(App::new(cli.query.clone(), event_rx, research_trigger)),
        vec![],
    )?;
    app.active(&Id::App)?;

    let mut adapter = CrosstermTerminalAdapter::new()?;
    adapter.enable_raw_mode()?;
    adapter.enter_alternate_screen()?;
    adapter.enable_mouse_capture()?;

    let mut quit = false;

    while !quit {
        match app.tick(PollStrategy::Once(Duration::from_millis(100))) {
            Ok(msgs) => {
                for msg in msgs {
                    if msg == Msg::Quit {
                        quit = true;
                    }
                }
            }
            Err(_) => {}
        }
        let _ = adapter.draw(|f| {
            app.view(&Id::App, f, f.area());
        });
    }

    drop(adapter);

    if let Some(component) = app.get_component_mut(&Id::App) {
        let any = component.as_any_mut();
        if let Some(comp) = any.downcast_mut::<App>() {
            if let Some(report) = comp.report() {
            let font_paths = vec![config.pdf.font.clone()];
                let pdf_filename = report
                    .title
                    .chars()
                    .map(|c| {
                        if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' {
                            c
                        } else {
                            ' '
                        }
                    })
                    .collect::<String>()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join("_")
                    .trim_end_matches('_')
                    .to_string()
                    + ".pdf";
                if let Err(e) = pdf::compile_report(report, std::path::Path::new(&pdf_filename), &font_paths) {
                    tracing::warn!("PDF 生成失败: {e}");
                } else {
                    println!("PDF: {}", pdf_filename);
                }
            }
        }
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
