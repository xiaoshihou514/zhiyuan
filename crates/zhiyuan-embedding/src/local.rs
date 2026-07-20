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
    ///
    /// 模型下载优先使用 `HF_ENDPOINT` 环境变量（若未设置则默认 `https://hf-mirror.com`）。
    pub fn new(model_name: Option<&str>) -> Result<Self, EmbeddingError> {
        // Hugging Face 国内镜像（用户可覆盖）
        if std::env::var("HF_ENDPOINT").is_err() {
            std::env::set_var("HF_ENDPOINT", "https://hf-mirror.com");
        }

        // 统一缓存到 ~/.cache/zhiyuan/embedding/
        let cache_dir = std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join(".cache").join("zhiyuan").join("embedding"))
            .unwrap_or_else(|_| std::path::PathBuf::from(".fastembed_cache"));

        let result = Self::try_load(model_name, cache_dir.clone());
        match result {
            Ok(embedder) => return Ok(embedder),
            Err(_) => {
                // 加载失败，清理不完整缓存后重试一次
                tracing::warn!("embedding 模型加载失败，清理缓存后重试...");
                let _ = std::fs::remove_dir_all(&cache_dir);
                Self::try_load(model_name, cache_dir)
            }
        }
    }

    fn try_load(model_name: Option<&str>, cache_dir: std::path::PathBuf) -> Result<Self, EmbeddingError> {
        let (model, dim, label) = match model_name {
            Some("bge-large-zh") | None => {
                let m = TextEmbedding::try_new(
                    InitOptions::new(EmbeddingModel::BGELargeZHV15)
                        .with_cache_dir(cache_dir.clone()),
                )
                    .map_err(|e| EmbeddingError {
                        message: format!("fastembed 模型加载失败: {e}"),
                        kind: EmbeddingErrorKind::ModelLoad,
                    })?;
                (m, 1024, "bge-large-zh-v1.5")
            }
            Some("bge-small-zh") => {
                let m = TextEmbedding::try_new(
                    InitOptions::new(EmbeddingModel::BGESmallZHV15)
                        .with_cache_dir(cache_dir.clone()),
                )
                    .map_err(|e| EmbeddingError {
                        message: format!("fastembed 模型加载失败: {e}"),
                        kind: EmbeddingErrorKind::ModelLoad,
                    })?;
                (m, 512, "bge-small-zh-v1.5")
            }
            Some("multilingual-e5-base") => {
                let m = TextEmbedding::try_new(
                    InitOptions::new(EmbeddingModel::MultilingualE5Base)
                        .with_cache_dir(cache_dir.clone()),
                )
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
