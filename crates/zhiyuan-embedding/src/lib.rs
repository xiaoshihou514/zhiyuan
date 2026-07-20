use serde::{Deserialize, Serialize};
use std::fmt;

/// 致远 Embedding 错误
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingError {
    pub message: String,
    pub kind: EmbeddingErrorKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EmbeddingErrorKind {
    ModelLoad,
    Inference,
    NotAvailable,
    Timeout,
}

impl fmt::Display for EmbeddingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{:?}] {}", self.kind, self.message)
    }
}

impl std::error::Error for EmbeddingError {}

// ─── EmbeddingProvider trait ──────────────────────────────────────────

/// 语义编码服务抽象
///
/// 所有实现必须满足 Send + Sync，供跨线程并发调用。
/// embed_batch 提供默认实现（逐条调用 embed），各实现可按需重写以利用批处理。
#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// 编码单条文本，返回归一化向量
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;

    /// 批量编码（默认逐条调用，可重写为真正的 batch 调用）
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let mut results = Vec::with_capacity(texts.len());
        for t in texts {
            results.push(self.embed(t).await?);
        }
        Ok(results)
    }

    /// 向量维度
    fn dimension(&self) -> usize;

    /// 模型/提供者名称
    fn name(&self) -> &str;
}

// ─── NoopEmbedder（回退桩）────────────────────────────────────────────

/// 总是返回 NotAvailable 错误的 embedding 提供者
///
/// 当本地模型加载失败且未配置远程 API 时使用。
/// 所有调用方应捕获 `EmbeddingErrorKind::NotAvailable` 并回退到关键词匹配。
pub struct NoopEmbedder;

#[async_trait::async_trait]
impl EmbeddingProvider for NoopEmbedder {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
        Err(EmbeddingError {
            message: "embedding 服务不可用（未配置）".into(),
            kind: EmbeddingErrorKind::NotAvailable,
        })
    }

    fn dimension(&self) -> usize {
        0
    }

    fn name(&self) -> &str {
        "noop"
    }
}

// ─── LocalEmbedder（fastembed）────────────────────────────────────────

#[cfg(feature = "local")]
pub mod local;

// ─── 向量索引 ─────────────────────────────────────────────────────────

pub mod index;

// ─── 便捷工厂 ─────────────────────────────────────────────────────────

/// 尝试创建本地 embedder，失败时返回 NoopEmbedder
pub fn auto_embedder(model_name: Option<&str>) -> Box<dyn EmbeddingProvider> {
    #[cfg(feature = "local")]
    {
        match local::LocalEmbedder::new(model_name) {
            Ok(embedder) => {
                tracing::info!(
                    "本地 embedding 模型已加载: {} ({} 维)",
                    embedder.name(),
                    embedder.dimension()
                );
                return Box::new(embedder);
            }
            Err(e) => {
                tracing::warn!("本地 embedding 模型加载失败: [{:?}] {}", e.kind, e.message);
            }
        }
    }

    #[cfg(not(feature = "local"))]
    {
        let _ = model_name;
        tracing::info!("embedding local feature 未启用，使用 noop 回退");
    }

    Box::new(NoopEmbedder)
}
