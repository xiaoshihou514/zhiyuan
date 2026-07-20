use async_trait::async_trait;
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing;

use crate::{EmbeddingError, EmbeddingErrorKind, EmbeddingProvider};

/// 基于 fastembed (ONNX) 的本地 embedding 模型
///
/// 默认使用 BAAI/bge-large-zh-v1.5（中文优化，1024 维）。
/// 模型文件首次使用时自动下载到 ~/.cache/fastembed/。
pub struct LocalEmbedder {
    model: Arc<Mutex<TextEmbedding>>,
    model_name: String,
    dim: usize,
}

impl LocalEmbedder {
    /// 创建本地 embedder
    ///
    /// `model_name` 可选：传入 `"bge-large-zh"`、`"multilingual-e5-base"` 等。
    /// 为 `None` 时使用默认模型（bge-large-zh-v1.5）。
    pub fn new(model_name: Option<&str>) -> Result<Self, EmbeddingError> {
        let (model, dim, label) = match model_name {
            Some("bge-large-zh") | None => {
                let m = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::BGELargeZHV15))
                    .map_err(|e| EmbeddingError {
                        message: format!("fastembed 模型加载失败: {e}"),
                        kind: EmbeddingErrorKind::ModelLoad,
                    })?;
                (m, 1024, "bge-large-zh-v1.5")
            }
            Some("bge-small-zh") => {
                let m = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::BGESmallZHV15))
                    .map_err(|e| EmbeddingError {
                        message: format!("fastembed 模型加载失败: {e}"),
                        kind: EmbeddingErrorKind::ModelLoad,
                    })?;
                (m, 512, "bge-small-zh-v1.5")
            }
            Some("multilingual-e5-base") => {
                let m = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::MultilingualE5Base))
                    .map_err(|e| EmbeddingError {
                        message: format!("fastembed 模型加载失败: {e}"),
                        kind: EmbeddingErrorKind::ModelLoad,
                    })?;
                (m, 768, "multilingual-e5-base")
            }
            Some(other) => {
                return Err(EmbeddingError {
                    message: format!("不支持的模型: {other}，支持: bge-large-zh, bge-small-zh, multilingual-e5-base"),
                    kind: EmbeddingErrorKind::ModelLoad,
                })
            }
        };

        tracing::info!("LocalEmbedder 初始化完成: {} ({} 维)", label, dim);

        Ok(Self {
            model: Arc::new(Mutex::new(model)),
            model_name: label.to_string(),
            dim,
        })
    }
}

#[async_trait]
impl EmbeddingProvider for LocalEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let model = self.model.lock().await;
        let mut embeddings = model
            .embed(vec![text], None)
            .map_err(|e| EmbeddingError {
                message: format!("embedding 推理失败: {e}"),
                kind: EmbeddingErrorKind::Inference,
            })?;

        embeddings.pop().ok_or_else(|| EmbeddingError {
            message: "embedding 返回空结果".into(),
            kind: EmbeddingErrorKind::Inference,
        })
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        let owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
        let model = self.model.lock().await;
        model
            .embed(owned, None)
            .map_err(|e| EmbeddingError {
                message: format!("batch embedding 推理失败: {e}"),
                kind: EmbeddingErrorKind::Inference,
            })
    }

    fn dimension(&self) -> usize {
        self.dim
    }

    fn name(&self) -> &str {
        &self.model_name
    }
}
