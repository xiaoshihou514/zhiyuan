use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zhiyuan_core::{LlmClient, Result};

pub struct OpenaiLlm {
    client: reqwest::Client,
    api_key: String,
    model: String,
    endpoint: String,
    log_file: Option<Arc<Mutex<std::fs::File>>>,
    prompt_cache: Mutex<HashMap<String, String>>,
    token_tx: Option<tokio::sync::mpsc::UnboundedSender<(usize, usize)>>,
}

impl OpenaiLlm {
    pub fn new(
        api_key: String,
        base_url: String,
        model: String,
        log_path: Option<String>,
        token_tx: Option<tokio::sync::mpsc::UnboundedSender<(usize, usize)>>,
    ) -> Self {
        let endpoint = if base_url.ends_with('/') {
            format!("{}chat/completions", base_url)
        } else {
            format!("{}/chat/completions", base_url)
        };
        let endpoint = if base_url.contains("chat/completions") {
            base_url
        } else {
            endpoint
        };

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("Failed to create HTTP client");

        let log_file = log_path
            .and_then(|p| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&p)
                    .ok()
            })
            .map(|f| Arc::new(Mutex::new(f)));

        Self {
            client,
            api_key,
            model,
            endpoint,
            log_file,
            prompt_cache: Mutex::new(HashMap::new()),
            token_tx,
        }
    }

    fn cache_key(system: &str, user: &str) -> String {
        format!("{}\x00{}", system, user)
    }

    fn cache_get(&self, key: &str) -> Option<String> {
        self.prompt_cache.lock().ok()?.get(key).cloned()
    }

    fn cache_set(&self, key: String, value: String) {
        if let Ok(mut cache) = self.prompt_cache.lock() {
            cache.insert(key, value);
        }
    }

    fn log(&self, s: &str) {
        if let Some(ref f) = self.log_file {
            let mut f = f.lock().unwrap();
            writeln!(f, "{}", s).ok();
        }
    }
}

#[async_trait]
impl LlmClient for OpenaiLlm {
    async fn prompt(&self, system: &str, user: &str) -> Result<String> {
        let cache_key = Self::cache_key(system, user);
        if let Some(cached) = self.cache_get(&cache_key) {
            tracing::info!("LLM 缓存命中 ({})", cached.chars().take(50).collect::<String>());
            return Ok(cached);
        }

        let now = chrono::Utc::now().format("%H:%M:%S");
        self.log(&format!("[{now}] SYSTEM: {system}"));
        self.log(&format!("[{now}] USER: {user}"));

        let body = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: system.to_string(),
                },
                Message {
                    role: "user".into(),
                    content: user.to_string(),
                },
            ],
            temperature: Some(0.3),
        };

        let max_retries = 5u32;
        for attempt in 1..=max_retries {
            let mut req = self.client.post(&self.endpoint).json(&body);
            if !self.api_key.is_empty() {
                req = req.header("Authorization", format!("Bearer {}", self.api_key));
            }

            match req.send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if !status.is_success() {
                        let text = resp.text().await.unwrap_or_default();
                        self.log(&format!("[{now}] 尝试 {attempt}/{max_retries}: HTTP {status}: {text}"));
                        tracing::warn!("LLM 返回 {status}: {text}，{attempt}/{max_retries} 次重试...");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        continue;
                    }

                    match resp.json::<ChatResponse>().await {
                        Ok(chat_resp) => {
                            let (prompt_tok, completion_tok) = chat_resp
                                .usage
                                .as_ref()
                                .map(|u| (u.prompt_tokens, u.completion_tokens))
                                .unwrap_or((0, 0));
                            if let Some(content) = chat_resp
                                .choices
                                .into_iter()
                                .next()
                                .and_then(|c| c.message.content)
                            {
                                self.cache_set(cache_key, content.clone());
                                let truncated: String = content.chars().take(500).collect();
                                self.log(&format!("[{now}] RESPONSE({} chars): {truncated}", content.len()));
                                if prompt_tok > 0 || completion_tok > 0 {
                                    if let Some(ref tx) = self.token_tx {
                                        let _ = tx.send((prompt_tok, completion_tok));
                                    }
                                }
                                return Ok(content);
                            }
                            self.log(&format!("[{now}] 尝试 {attempt}/{max_retries}: 空响应"));
                            tracing::warn!("LLM 返回空响应，{attempt}/{max_retries} 次重试...");
                        }
                        Err(e) => {
                            self.log(&format!("[{now}] 尝试 {attempt}/{max_retries}: JSON 解析失败: {e}"));
                            tracing::warn!("LLM JSON 解析失败: {e}，{attempt}/{max_retries} 次重试...");
                        }
                    }
                }
                Err(e) => {
                    self.log(&format!("[{now}] 尝试 {attempt}/{max_retries}: 请求失败: {e}"));
                    tracing::warn!("LLM 请求失败: {e}，{attempt}/{max_retries} 次重试...");
                }
            }
            tokio::time::sleep(Duration::from_secs(2u64 * attempt as u64)).await;
        }

        let err = format!("LLM 请求失败（重试 {max_retries} 次后放弃）");
        self.log(&format!("[{now}] {err}"));
        Err(zhiyuan_core::Error::Llm(err))
    }

    fn clone_box(&self) -> Box<dyn LlmClient> {
        Box::new(Self {
            client: self.client.clone(),
            api_key: self.api_key.clone(),
            model: self.model.clone(),
            endpoint: self.endpoint.clone(),
            log_file: self.log_file.clone(),
            prompt_cache: Mutex::new(HashMap::new()),
            token_tx: self.token_tx.clone(),
        })
    }
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
}

#[derive(Serialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    prompt_tokens: usize,
    completion_tokens: usize,
}

#[derive(Deserialize)]
struct Choice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: Option<String>,
}
