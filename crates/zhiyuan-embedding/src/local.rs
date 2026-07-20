use async_trait::async_trait;
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
use hf_hub::api::sync::ApiBuilder;
use hf_hub::{Cache, Repo, RepoType};
use std::path::Path;
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
        // 默认使用 Hugging Face 镜像站；用户可设 HF_ENDPOINT 覆盖
        if std::env::var("HF_ENDPOINT").is_err() {
            std::env::set_var("HF_ENDPOINT", "https://hf-mirror.com");
        }

        // 统一缓存到 ~/.cache/zhiyuan/embedding/
        let cache_dir = std::env::var("HOME")
            .map(|h| std::path::PathBuf::from(h).join(".cache").join("zhiyuan").join("embedding"))
            .unwrap_or_else(|_| std::path::PathBuf::from(".fastembed_cache"));

        Self::try_load(model_name, cache_dir)
    }

    fn try_load(model_name: Option<&str>, cache_dir: std::path::PathBuf) -> Result<Self, EmbeddingError> {
        let result = self::try_load_impl(model_name, &cache_dir);
        match result {
            Ok(v) => Ok(v),
            Err(e) => {
                tracing::error!(
                    "fastembed 模型加载失败: {e}\n\
                     诊断信息:\n\
                     cache_dir     = {cache_dir:?}\n\
                     HF_ENDPOINT   = {:?}\n\
                     HF_HOME       = {:?}\n\
                     FASTEMBED_CACHE_DIR = {:?}\n\
                     缓存内容:\n{}",
                    std::env::var("HF_ENDPOINT").ok(),
                    std::env::var("HF_HOME").ok(),
                    std::env::var("FASTEMBED_CACHE_DIR").ok(),
                    dump_cache_dir_detailed(&cache_dir),
                );
                Err(e)
            }
        }
    }
}

// ─── 模型名 → Repo 映射 ────────────────────────────────────────────────

fn model_repo(model_name: Option<&str>) -> Option<Repo> {
    let id = match model_name {
        None | Some("bge-large-zh") => "Xenova/bge-large-zh-v1.5",
        Some("bge-small-zh") => "Xenova/bge-small-zh-v1.5",
        Some("multilingual-e5-base") => "Xenova/multilingual-e5-base",
        Some(_) => return None,
    };
    Some(Repo::new(id.to_string(), RepoType::Model))
}

/// 所有支持的 Repo 列表（用于遍历清理等）
fn all_model_repos() -> [Repo; 3] {
    [
        Repo::new("Xenova/bge-large-zh-v1.5".into(), RepoType::Model),
        Repo::new("Xenova/bge-small-zh-v1.5".into(), RepoType::Model),
        Repo::new("Xenova/multilingual-e5-base".into(), RepoType::Model),
    ]
}

// ─── 缓存管理 ──────────────────────────────────────────────────────────

/// 清理 hf-hub 残留的 `.lock` 文件。
fn cleanup_stale_locks(cache_dir: &Path) {
    for repo in all_model_repos() {
        let blobs_dir = cache_dir.join(repo.folder_name()).join("blobs");
        let Ok(entries) = std::fs::read_dir(&blobs_dir) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.ends_with(".lock") {
                continue;
            }
            let blob_name = &name_str[..name_str.len() - 5]; // 去掉 .lock
            let blob_path = blobs_dir.join(blob_name);
            if blob_path.exists() {
                if blob_name.ends_with(".part") {
                    let _ = std::fs::remove_file(entry.path());
                    let _ = std::fs::remove_file(&blob_path);
                    tracing::warn!("清理不完整下载残留: {blob_name}");
                } else {
                    let _ = std::fs::remove_file(entry.path());
                    tracing::info!("清理残留 lock: {name_str}");
                }
            } else {
                let _ = std::fs::remove_file(entry.path());
                tracing::warn!("清理孤立 lock: {name_str}");
            }
        }
    }
}

/// 用 hf-hub 库管理缓存，补全缺失的 tokenizer 小文件。
///
/// hf-hub 的 `metadata()` 方法（内部被 `download()` 调用）依赖
/// `Range: bytes=0-0 → 206 → Content-Range`，但 hf-mirror 的 CDN
/// 返回 `200 OK` 不带 Content-Range。此处绕过 metadata()，
/// 直接用 ureq 下载到 hf-hub 的缓存结构中，使后续 fastembed 的
/// `get()` 全部命中本地缓存。
fn ensure_cache_complete(model_name: Option<&str>, cache_dir: &Path) {
    let repo = match model_repo(model_name) {
        Some(r) => r,
        None => return,
    };

    let cache = Cache::new(cache_dir.to_path_buf());
    let cache_repo = cache.repo(repo.clone());
    let folder_name = repo.folder_name();

    // 读取 commit hash
    let refs_path = cache_dir.join(&folder_name).join("refs").join(repo.revision());
    let commit_hash = match std::fs::read_to_string(&refs_path) {
        Ok(h) => h.trim().to_string(),
        Err(_) => return,
    };

    let snapshot_dir = cache_dir.join(&folder_name).join("snapshots").join(&commit_hash);
    let blobs_dir = cache_dir.join(&folder_name).join("blobs");

    // fastembed 的 load_tokenizer_hf_hub 需要的文件
    let needed_files = [
        "tokenizer.json",
        "config.json",
        "special_tokens_map.json",
        "tokenizer_config.json",
    ];

    // 用 ApiBuilder 创建 API（自动读取 HF_ENDPOINT）
    let api = match ApiBuilder::new()
        .with_cache_dir(cache_dir.to_path_buf())
        .build()
    {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("创建 HF API 失败: {e}");
            return;
        }
    };
    // api.model() 需要 repo_id，用 folder_name 反推 repo_id
    let folder_name = repo.folder_name();
    let repo_id = folder_name
        .strip_prefix("models--")
        .unwrap_or(&folder_name)
        .replace("--", "/");
    let api_repo = api.model(repo_id);

    for filename in &needed_files {
        // 先用 hf-hub 的 CacheRepo::get() 检查缓存
        if cache_repo.get(filename).is_some() {
            continue;
        }

        let _ = std::fs::create_dir_all(&snapshot_dir);
        let url = api_repo.url(filename);

        tracing::info!("缓存缺失 {filename}，下载...");

        // ureq GET（不设 Range 头，避免 Content-Range 问题）
        let resp = match ureq::get(&url).call() {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!("下载 {filename} 失败: {e}");
                continue;
            }
        };

        // 小文件直接读到内存（tokenizer 文件都 < 1MB），算 SHA256 作为 blobs 文件名
        let mut data: Vec<u8> = Vec::new();
        if std::io::Read::read_to_end(&mut resp.into_reader(), &mut data).is_err() {
            tracing::warn!("读取 {filename} 响应失败");
            continue;
        }
        use sha2::Digest;
        let hex_hash = format!("{:x}", sha2::Sha256::digest(&data));

        let blob_path = blobs_dir.join(&hex_hash);
        if let Err(e) = std::fs::create_dir_all(&blobs_dir)
            .and_then(|_| std::fs::write(&blob_path, &data))
        {
            tracing::warn!("保存 {filename} 失败: {e}");
            continue;
        }

        // 创建 snapshots 下的符号链接
        // 注意：onnx/ 下的文件需要 ../../../ 到 blobs，根目录下的文件只需要 ../../
        let _ = std::fs::remove_file(&snapshot_dir.join(filename));
        let is_in_onnx = filename.contains('/');
        let rel_blob = if is_in_onnx {
            format!("../../../blobs/{hex_hash}")
        } else {
            format!("../../blobs/{hex_hash}")
        };
        if std::os::unix::fs::symlink(&rel_blob, &snapshot_dir.join(filename)).is_err() {
            let _ = std::fs::copy(&blob_path, &snapshot_dir.join(filename));
        }
        tracing::info!("缓存完成: {filename}");
    }
}

/// 实际的加载逻辑
fn try_load_impl(
    model_name: Option<&str>,
    cache_dir: &Path,
) -> Result<LocalEmbedder, EmbeddingError> {
    cleanup_stale_locks(cache_dir);
    ensure_cache_complete(model_name, cache_dir);

    let (model, dim, label) = match model_name {
        Some("bge-large-zh") | None => {
            let m = TextEmbedding::try_new(
                InitOptions::new(EmbeddingModel::BGELargeZHV15)
                    .with_cache_dir(cache_dir.to_path_buf()),
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
                    .with_cache_dir(cache_dir.to_path_buf()),
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
                    .with_cache_dir(cache_dir.to_path_buf()),
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

    Ok(LocalEmbedder {
        model: Arc::new(Mutex::new(model)),
        model_name: label.to_string(),
        dim,
    })
}

// ─── 诊断辅助 ──────────────────────────────────────────────────────────

fn dump_cache_dir_detailed(dir: &Path) -> String {
    if !dir.exists() {
        return "  (目录不存在)".into();
    }
    let mut lines = Vec::new();
    collect_entries(dir, 0, 4, &mut lines);

    for repo in all_model_repos() {
        let folder = repo.folder_name();
        let repo_dir = dir.join(&folder);

        // refs
        let refs_path = repo_dir.join("refs").join(repo.revision());
        if refs_path.exists() {
            let content = std::fs::read_to_string(&refs_path).unwrap_or_default();
            lines.push(format!("  [{folder}] refs/{} = {:?}", repo.revision(), content.trim()));
        }

        // snapshots
        let snap_dir = repo_dir.join("snapshots");
        if snap_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&snap_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    lines.push(format!("  [{folder}] snapshots/{:?} (目录)", name));
                    // 检查关键文件
                    for check in &["onnx/model.onnx", "tokenizer.json", "config.json"] {
                        let path = entry.path().join(check);
                        if path.exists() || path.is_symlink() {
                            let meta = std::fs::symlink_metadata(&path).ok();
                            lines.push(format!("    -> {check} 存在 (symlink={})", meta.map(|m| m.len()).map_or("?".into(), |s| s.to_string())));
                        }
                    }
                }
            }
        }

        // blobs
        let blobs_dir = repo_dir.join("blobs");
        if blobs_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&blobs_dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let size = entry.metadata().ok().map(|m| m.len()).unwrap_or(0);
                    if name.ends_with(".part") {
                        lines.push(format!("  [{folder}] ⚠ blobs/{name} (部分下载, {size}B)"));
                    } else if name.ends_with(".lock") {
                        lines.push(format!("  [{folder}] 🔒 blobs/{name} (锁)"));
                    } else {
                        lines.push(format!("  [{folder}] blobs/{name} ({}B 完整)", size));
                    }
                }
            }
        }
    }

    lines.join("\n")
}

fn collect_entries(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<String>) {
    if depth > max_depth {
        return;
    }
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            let meta = entry.metadata().ok();
            let size = meta.and_then(|m| if m.is_file() { Some(m.len()) } else { None });
            let indent = "  ".repeat(depth);
            match size {
                Some(s) => out.push(format!("{indent}{} ({}B)", path.display(), s)),
                None => out.push(format!("{indent}{}/", path.display())),
            }
            if path.is_dir() {
                collect_entries(&path, depth + 1, max_depth, out);
            }
        }
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
