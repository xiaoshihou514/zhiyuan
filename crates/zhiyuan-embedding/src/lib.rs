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
#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let mut results = Vec::with_capacity(texts.len());
        for t in texts {
            results.push(self.embed(t).await?);
        }
        Ok(results)
    }
    fn dimension(&self) -> usize;
    fn name(&self) -> &str;
}

// ─── LocalEmbedder（fastembed）────────────────────────────────────────

#[cfg(feature = "local")]
pub mod local;

// ─── 向量索引 ─────────────────────────────────────────────────────────

pub mod index;

// ─── 便捷工厂 ─────────────────────────────────────────────────────────

/// 初始化 embedding 模型，失败时直接退出进程
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
                tracing::error!("本地 embedding 模型加载失败: [{:?}] {}", e.kind, e.message);
                tracing::error!("请检查网络连接，或手动设置 HF_ENDPOINT 指定镜像源");
                std::process::exit(1);
            }
        }
    }

    #[cfg(not(feature = "local"))]
    {
        tracing::error!("embedding 功能未编译（编译时需要 --features local）");
        std::process::exit(1);
    }
}
