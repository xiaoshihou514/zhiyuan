use crate::Result;
use async_trait::async_trait;

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn prompt(&self, system: &str, user: &str) -> Result<String>;
    fn clone_box(&self) -> Box<dyn LlmClient>;
}
