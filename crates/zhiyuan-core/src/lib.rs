mod llm;
mod argument;
pub use llm::*;
pub use argument::*;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
pub use uuid::Uuid;

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
    /// 核心论点（由 PlannerAgent 生成）
    #[serde(default)]
    pub core_thesis: Option<String>,
    /// 预期推理链（由 PlannerAgent 生成）
    #[serde(default)]
    pub reasoning_chain: Option<Vec<String>>,
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
    #[serde(default = "default_search_categories")]
    pub categories: String,
}

fn default_search_categories() -> String {
    "general".to_string()
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
    /// 对论证骨架的影响类型（由 SynthesizerAgent 标注）
    #[serde(default)]
    pub epistemic_status: Option<EpistemicStatus>,
}

/// 论证影响类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum EpistemicStatus {
    /// 强化现有论点
    Supports,
    /// 削弱现有论点（触发修正）
    Undermines,
    /// 扩展论点边界（可能新增子论点）
    Extends,
    /// 与当前论证骨架无关（暂存或丢弃）
    Irrelevant,
}

// ─── 论证骨架（Argument Skeleton）────────────────────────────────────

/// 论点类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ArgumentNodeType {
    /// 前提：基础假设或已知事实
    Premise,
    /// 证据：支撑论点的数据或引用
    Evidence,
    /// 结论：由前提和证据推导出的论断
    Conclusion,
}

/// 论证骨架中的单个节点
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgumentNode {
    pub id: Uuid,
    /// 论点表述
    pub claim: String,
    /// 节点类型
    pub node_type: ArgumentNodeType,
    /// 在推理链中的层级（0=最基础）
    pub layer: usize,
    /// 支撑该论点的来源 URL
    pub sources: Vec<String>,
}

/// 论证节点间的边
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ArgumentEdge {
    /// 支撑关系
    Supports,
    /// 削弱关系
    Undermines,
    /// 拓展关系
    Extends,
}

/// 论证骨架：研究的推理结构表示
///
/// 由 PlannerAgent 初始化（core_thesis + reasoning_chain），
/// 在迭代过程中被 SynthesizerAgent 增量更新，
/// 供 QualityEvaluator 评分和 WriterAgent 组织报告。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgumentSkeleton {
    /// 所有论证节点
    pub nodes: Vec<ArgumentNode>,
    /// 节点间的边（(from_id, to_id, relation)）
    pub edges: Vec<(Uuid, Uuid, ArgumentEdge)>,
    /// 章节到骨架节点的映射：chapter_index → node_ids
    #[serde(default)]
    pub chapter_mapping: Vec<(usize, Uuid)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeBase {
    pub query_id: Uuid,
    pub findings: Vec<Finding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityScore {
    pub coverage: f64,
    pub freshness: f64,
    pub depth: f64,
    pub overall: f64,
}

impl QualityScore {
    pub fn new(coverage: f64, freshness: f64, depth: f64) -> Self {
        let overall = coverage * 0.4 + freshness * 0.3 + depth * 0.3;
        Self {
            coverage,
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
    pub argument_skeleton: Option<ArgumentSkeleton>,
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
    #[serde(default)]
    pub embedding: EmbeddingConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    /// 模型名称：bge-large-zh / bge-small-zh / multilingual-e5-base
    #[serde(default = "default_embedding_model")]
    pub model: String,
}

fn default_embedding_model() -> String {
    "bge-large-zh".to_string()
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            model: "bge-large-zh".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PdfConfig {
    pub font_paths: Vec<String>,
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
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    #[serde(default)]
    pub long_report: bool,
    #[serde(default)]
    pub cross_validate: bool,
}

fn default_max_iterations() -> usize {
    4
}
fn default_concurrency() -> usize {
    4
}

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
