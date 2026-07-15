use std::sync::Arc;
use crate::util::extract_json;
use zhiyuan_core::{ExtractedContent, LlmClient, Result, SearchResult};
use zhiyuan_extract::ContentExtractor;

pub struct ExtractorAgent {
    llm: Box<dyn LlmClient>,
    extractor: Arc<dyn ContentExtractor + Send + Sync>,
}

impl ExtractorAgent {
    pub fn new(llm: Box<dyn LlmClient>, extractor: Arc<dyn ContentExtractor + Send + Sync>) -> Self {
        Self { llm, extractor }
    }

    pub async fn extract_content(&self, results: &[SearchResult], context: &str) -> Result<Vec<ExtractedContent>> {
        let mut extracted = Vec::new();

        let system = "你是一个信息提取专家。你的任务是从搜索结果中选择最有价值的网页进行抓取，\
并提取与研究目标相关的关键信息。\
只输出纯 JSON，不要 markdown 格式、不要代码块、不要其他文字。";

        let results_list: String = results
            .iter()
            .map(|r| format!("- [{}]({}): {}", r.title, r.url, r.snippet))
            .collect::<Vec<_>>()
            .join("\n");

        let user = format!(
            "研究上下文：{context}
搜索结果列表：
{results_list}
请从以上结果中选出最相关的 3-5 个网页 URL，输出 JSON 数组格式：{{\"urls\": [\"url1\", \"url2\"]}}"
        );

        let response = self.llm.prompt(system, &user).await?;
        let cleaned = extract_json(&response);
        let parsed: serde_json::Value = serde_json::from_str(cleaned)
            .unwrap_or(serde_json::json!({"urls": []}));

        let selected_urls: Vec<String> = parsed["urls"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        let selected: Vec<&SearchResult> = results.iter().filter(|r| selected_urls.contains(&r.url)).take(5).collect();

        for result in selected {
            match self.extractor.extract(result, context).await {
                Ok(content) => extracted.push(content),
                Err(e) => tracing::warn!("Extraction failed for {}: {e}", result.url),
            }
        }

        if extracted.is_empty() {
            for result in results.iter().take(3) {
                if let Ok(content) = self.extractor.extract(result, context).await {
                    extracted.push(content);
                }
            }
        }

        Ok(extracted)
    }
}
