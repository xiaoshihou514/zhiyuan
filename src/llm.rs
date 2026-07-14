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
    pub fn from_env() -> anyhow::Result<Self> {
        let api_key = std::env::var("OPENAI_API_KEY")
            .map_err(|_| anyhow::anyhow!("OPENAI_API_KEY not set"))?;

        let model = std::env::var("MAIN_MODEL").unwrap_or_else(|_| "gpt-4o".into());

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()?;

        Ok(Self {
            client,
            api_key,
            model,
            endpoint: "https://api.openai.com/v1/chat/completions".into(),
        })
    }

    #[allow(dead_code)]
    pub fn new(api_key: String, model: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            client,
            api_key,
            model,
            endpoint: "https://api.openai.com/v1/chat/completions".into(),
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

        let resp = self
            .client
            .post(&self.endpoint)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Llm(format!("OpenAI request failed: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(Error::Llm(format!("OpenAI returned {status}: {text}")));
        }

        let chat_resp: ChatResponse = resp
            .json()
            .await
            .map_err(|e| Error::Llm(format!("OpenAI parse failed: {e}")))?;

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
