mod llm;
pub use llm::*;

pub use uuid::Uuid;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Search error: {0}")]
    Search(String),
    #[error("Extract error: {0}")]
    Extract(String),
    #[error("Memory error: {0}")]
    Memory(String),
    #[error("Orchestration error: {0}")]
    Orchestration(String),
    #[error("Agent error: {0}")]
    Agent(String),
    #[error("LLM error: {0}")]
    Llm(String),
    #[error("Robust execution error: {0}")]
    Robust(String),
    #[error("Config error: {0}")]
    Config(String),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchQuery {
    pub id: Uuid,
    pub query: String,
    pub clarification: Option<String>,
}

impl ResearchQuery {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            query: query.into(),
            clarification: None,
        }
    }

    pub fn full_query(&self) -> String {
        match &self.clarification {
            Some(c) => format!("{}\n\nAdditional context: {}", self.query, c),
            None => self.query.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchPlan {
    pub query_id: Uuid,
    pub sub_tasks: Vec<SubTask>,
    pub outline: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubTask {
    pub id: Uuid,
    pub description: String,
    pub status: TaskStatus,
    pub dependencies: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub max_results: usize,
    pub region: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: String,
    pub fetch_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedContent {
    pub url: String,
    pub title: String,
    pub text: String,
    pub relevance_score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Finding {
    pub id: Uuid,
    pub content: String,
    pub sources: Vec<String>,
    pub sub_task_id: Option<Uuid>,
    pub iteration: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeBase {
    pub query_id: Uuid,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityScore {
    pub coverage: f64,
    pub reliability: f64,
    pub freshness: f64,
    pub depth: f64,
    pub overall: f64,
}

impl QualityScore {
    pub fn new(coverage: f64, reliability: f64, freshness: f64, depth: f64) -> Self {
        let overall = coverage * 0.3 + reliability * 0.3 + freshness * 0.2 + depth * 0.2;
        Self {
            coverage,
            reliability,
            freshness,
            depth,
            overall,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CitationGraph {
    pub claims: Vec<Claim>,
    pub sources: Vec<SourceNode>,
    pub edges: Vec<CitationEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Claim {
    pub id: Uuid,
    pub text: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceNode {
    pub id: Uuid,
    pub url: String,
    pub title: String,
    pub reliability: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CitationEdge {
    Supports { claim_id: Uuid, source_id: Uuid },
    Contradicts { claim_id: Uuid, source_id: Uuid },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchReport {
    pub query_id: Uuid,
    pub title: String,
    pub sections: Vec<ReportSection>,
    pub citation_graph: CitationGraph,
    pub quality_score: QualityScore,
    pub generated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportSection {
    pub heading: String,
    pub content: String,
    pub citations: Vec<String>,
}

#[derive(Debug, Clone)]
pub enum ProgressUpdate {
    Started {
        max_iterations: usize,
        tasks: Vec<String>,
    },
    Phase {
        name: String,
        message: String,
    },
    Iteration {
        iteration: usize,
        max_iterations: usize,
        quality: Option<QualityScore>,
        findings_count: usize,
        sources_count: usize,
    },
    TaskPhase {
        task_desc: String,
        phase: String,
    },
    Report(ResearchReport),
    Error(String),
}

pub trait ProgressReporter: Send + Sync {
    fn report(&self, update: ProgressUpdate);
}

pub struct NullProgressReporter;
impl ProgressReporter for NullProgressReporter {
    fn report(&self, _update: ProgressUpdate) {}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchConfig {
    pub search: SearchConfig,
    pub llm: LlmConfig,
    pub research: ResearchSettings,
    pub pdf: PdfConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfConfig {
    pub font: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchConfig {
    pub max_results: usize,
    #[serde(default = "default_searxng_url")]
    pub searxng_url: String,
    #[serde(default)]
    pub blocked_domains: Vec<String>,
}

fn default_searxng_url() -> String {
    "http://localhost:8888".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub api_key: String,
    pub base_url: String,
    pub main_model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchSettings {
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    #[serde(default = "default_quality_threshold")]
    pub quality_threshold: f64,
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default)]
    pub long_report: bool,
    #[serde(default)]
    pub cross_validate: bool,
}

fn default_max_iterations() -> usize { 4 }
fn default_quality_threshold() -> f64 { 0.7 }
fn default_concurrency() -> usize { 4 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressReport {
    pub iteration: usize,
    pub quality_score: Option<QualityScore>,
    pub findings_count: usize,
    pub sources_count: usize,
    pub cost_usd: f64,
    pub status: String,
}

pub fn sub_task_from_value(parsed: &serde_json::Value) -> Vec<SubTask> {
    parsed["sub_tasks"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|v| SubTask {
                    id: Uuid::new_v4(),
                    description: v["description"].as_str().unwrap_or("").to_string(),
                    status: TaskStatus::Pending,
                    dependencies: v["dependencies"]
                        .as_array()
                        .map(|d| {
                            d.iter()
                                .filter_map(|x| x.as_str().and_then(|s| Uuid::parse_str(s).ok()))
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchDirection {
    pub description: String,
    pub rationale: String,
    pub priority: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReportChapter {
    pub title: String,
    pub description: String,
    pub findings: Vec<Finding>,
}

pub trait ModelProvider: Send + Sync {
    fn name(&self) -> &str;
    fn tier(&self) -> ModelTier;
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum ModelTier {
    Reasoning,
    Main,
    Fast,
}
