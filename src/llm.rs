use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use zhiyuan_core::{Error, LlmClient, Result};

pub struct OpenaiLlm {
    client: reqwest::Client,
    api_key: String,
    model: String,
    endpoint: String,
    log_file: Option<Arc<Mutex<std::fs::File>>>,
}

impl OpenaiLlm {
    pub fn new(
        api_key: String,
        base_url: String,
        model: String,
        log_path: Option<String>,
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

        let mut req = self.client.post(&self.endpoint).json(&body);
        if !self.api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", self.api_key));
        }
        let resp = req
            .send()
            .await
            .map_err(|e| Error::Llm(format!("LLM request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            self.log(&format!("[{now}] ERROR {status}: {text}"));
            return Err(Error::Llm(format!("LLM returned {status}: {text}")));
        }

        let chat_resp: ChatResponse = resp
            .json()
            .await
            .map_err(|e| Error::Llm(format!("LLM parse failed: {e}")))?;

        let content = chat_resp
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .ok_or_else(|| Error::Llm("No response content".into()))?;

        let truncated: String = content.chars().take(500).collect();
        self.log(&format!("[{now}] RESPONSE({} chars): {truncated}", content.len()));

        Ok(content)
    }

    fn clone_box(&self) -> Box<dyn LlmClient> {
        Box::new(Self {
            client: self.client.clone(),
            api_key: self.api_key.clone(),
            model: self.model.clone(),
            endpoint: self.endpoint.clone(),
            log_file: self.log_file.clone(),
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
}

#[derive(Deserialize)]
struct Choice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    content: Option<String>,
}
