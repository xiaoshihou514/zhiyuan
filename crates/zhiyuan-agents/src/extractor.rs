use std::sync::Arc;
use zhiyuan_core::{ExtractedContent, Result, SearchResult};
use zhiyuan_extract::ContentExtractor;

pub struct ExtractorAgent {
    extractor: Arc<dyn ContentExtractor + Send + Sync>,
}

impl ExtractorAgent {
    pub fn new(extractor: Arc<dyn ContentExtractor + Send + Sync>) -> Self {
        Self { extractor }
    }

    pub async fn extract_content(&self, results: &[SearchResult], context: &str) -> Result<Vec<ExtractedContent>> {
        let mut extracted = Vec::new();

        tracing::info!("总数" = %results.len(), "提取器选定URL");

        for result in results {
            tracing::debug!(url = %result.url, "正在提取内容");
            match self.extractor.extract(result, context).await {
                Ok(content) => extracted.push(content),
                Err(e) => tracing::warn!("内容提取失败 {}: {e}", result.url),
            }
        }

        if extracted.is_empty() {
            tracing::warn!("所有 URL 提取失败");
        }

        let titles: Vec<&str> = extracted.iter().map(|c| c.title.as_str()).collect();
        tracing::info!("已提取" = %extracted.len(), "标题" = ?titles, "内容提取完成");

        Ok(extracted)
    }
}
