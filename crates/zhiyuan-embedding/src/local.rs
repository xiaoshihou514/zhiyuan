use async_trait::async_trait;
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing;

use crate::{EmbeddingError, EmbeddingErrorKind, EmbeddingProvider};

/// sha256 十六进制字符串
fn sha256_hex(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

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
        // 默认使用 Hugging Face 镜像站；用户可设 HF_ENDPOINT 覆盖
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
            Err(e) => {
                tracing::warn!("embedding 模型加载失败: {e}，清理缓存后重试...");
                // 删除不完整的缓存文件
                if cache_dir.exists() {
                    if let Err(rm_err) = std::fs::remove_dir_all(&cache_dir) {
                        tracing::warn!("清理缓存目录失败: {rm_err}");
                    }
                }
                Self::try_load(model_name, cache_dir)
            }
        }
    }

    /// 用 ureq 预下载模型到 hf-hub 缓存目录
    ///
    /// 纯同步实现，可在 tokio 上下文中安全调用（不创建也不阻塞 runtime）。
    fn predownload_model(model_name: Option<&str>, cache_dir: &std::path::Path) -> Result<(), EmbeddingError> {
        let (repo_id, model_file, _dim) = match model_name {
            Some("bge-large-zh") | None => ("Xenova/bge-large-zh-v1.5", "onnx/model.onnx", 1024),
            Some("bge-small-zh") => ("Xenova/bge-small-zh-v1.5", "onnx/model.onnx", 512),
            Some("multilingual-e5-base") => ("Xenova/multilingual-e5-base", "onnx/model.onnx", 768),
            Some(other) => return Err(EmbeddingError {
                message: format!("不支持的模型: {other}"),
                kind: EmbeddingErrorKind::ModelLoad,
            }),
        };

        let repo_slug = repo_id.replace('/', "--").replace('-', "--");
        let repo_dir = cache_dir.join(format!("models--{}", repo_slug));
        let blob_dir = repo_dir.join("blobs");
        let blob_hash = sha256_hex(model_file);
        let blob_path = blob_dir.join(&blob_hash);

        if blob_path.exists() {
            return Ok(());
        }

        let endpoint = std::env::var("HF_ENDPOINT").unwrap_or_else(|_| "https://huggingface.co".to_string());
        let model_url = format!("{endpoint}/{repo_id}/resolve/main/{model_file}");
        tracing::info!("正在下载 embedding 模型...");

        // 用 ureq 同步下载（不涉及 tokio runtime）
        let agent = ureq::AgentBuilder::new()
            .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
            .timeout_connect(Duration::from_secs(30))
            .timeout(Duration::from_secs(300))
            .build();

        // HEAD 获取元数据
        let head_resp = agent.head(&model_url).call().map_err(|e| EmbeddingError {
            message: format!("请求元数据失败: {e}，请检查网络/代理"),
            kind: EmbeddingErrorKind::ModelLoad,
        })?;
        let commit_hash = head_resp.header("x-repo-commit").unwrap_or("main").to_string();

        // 下载文件
        std::fs::create_dir_all(&blob_dir).map_err(|e| EmbeddingError {
            message: format!("创建缓存目录失败: {e}"),
            kind: EmbeddingErrorKind::ModelLoad,
        })?;

        let response = agent.get(&model_url).call().map_err(|e| EmbeddingError {
            message: format!("下载失败: {e}，请检查网络/代理"),
            kind: EmbeddingErrorKind::ModelLoad,
        })?;

        let mut file = std::fs::File::create(&blob_path).map_err(|e| EmbeddingError {
            message: format!("创建文件失败: {e}"),
            kind: EmbeddingErrorKind::ModelLoad,
        })?;
        std::io::copy(&mut response.into_reader(), &mut file).map_err(|e| EmbeddingError {
            message: format!("写入文件失败: {e}"),
            kind: EmbeddingErrorKind::ModelLoad,
        })?;
        drop(file);

        // 创建 refs/main + snapshot symlink
        let refs_dir = repo_dir.join("refs");
        std::fs::create_dir_all(&refs_dir).ok();
        let _ = std::fs::write(refs_dir.join("main"), &commit_hash);

        let pointer_path = repo_dir.join("snapshots").join(&commit_hash).join(model_file);
        std::fs::create_dir_all(pointer_path.parent().unwrap()).ok();
        let rel = pathdiff::diff_paths(&blob_path, pointer_path.parent().unwrap());
        if let Some(rel) = rel {
            #[cfg(unix)]
            std::os::unix::fs::symlink(&rel, &pointer_path).ok();
        }
        if !pointer_path.exists() {
            std::fs::copy(&blob_path, &pointer_path).ok();
        }

        tracing::info!("embedding 模型下载完成");
        Ok(())
    }

    fn try_load(model_name: Option<&str>, cache_dir: std::path::PathBuf) -> Result<Self, EmbeddingError> {
        // 先用 reqwest 预下载模型文件到 hf-hub 缓存（解决 ureq UA 被 CDN 拦截的问题）
        if let Err(e) = Self::predownload_model(model_name, &cache_dir) {
            tracing::warn!("预下载模型文件失败（将尝试 fastembed 内置下载）: {e}");
        }

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
