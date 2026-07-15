use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use zhiyuan_core::{Error, LlmClient, Result};

pub struct OpenaiLlm {
    client: reqwest::Client,
    api_key: String,
    model: String,
    endpoint: String,
}

impl OpenaiLlm {
    pub fn new(api_key: String, base_url: String, model: String) -> Self {
        let endpoint = if base_url.ends_with('/') {
            format!("{}chat/completions", base_url)
        } else {
            format!("{}/chat/completions", base_url)
        };

        // Allow overriding endpoint directly for non-OpenAI-compatible APIs
        let endpoint = if base_url.contains("chat/completions") {
            base_url
        } else {
            endpoint
        };

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            api_key,
            model,
            endpoint,
        }
    }
}

#[async_trait]
impl LlmClient for OpenaiLlm {
    async fn prompt(&self, system: &str, user: &str) -> Result<String> {
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
        let resp = req.send().await.map_err(|e| Error::Llm(format!("LLM request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
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

        Ok(content)
    }

    fn clone_box(&self) -> Box<dyn LlmClient> {
        Box::new(Self {
            client: self.client.clone(),
            api_key: self.api_key.clone(),
            model: self.model.clone(),
            endpoint: self.endpoint.clone(),
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
