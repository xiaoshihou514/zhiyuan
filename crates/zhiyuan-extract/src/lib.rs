use async_trait::async_trait;
use scraper::{Html, Selector};
use std::time::Duration;
use zhiyuan_core::{Error, ExtractedContent, Result, SearchResult};

#[async_trait]
pub trait ContentExtractor: Send + Sync {
    async fn extract(&self, result: &SearchResult, context: &str) -> Result<ExtractedContent>;
    fn name(&self) -> &'static str;
}

pub struct WebExtractor {
    client: reqwest::Client,
    max_text_length: usize,
}

impl WebExtractor {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (compatible; ZhiyuanResearch/0.1)")
            .build()
            .expect("Failed to create HTTP client");
        Self {
            client,
            max_text_length: 100_000,
        }
    }

    fn extract_main_content(&self, html: &str) -> String {
        let doc = Html::parse_document(html);
        let body_sel = Selector::parse("body").unwrap();
        let text: String = doc
            .select(&body_sel)
            .flat_map(|el| el.text())
            .collect::<Vec<_>>()
            .join(" ");
        self.clean_text(&text)
    }

    fn clean_text(&self, text: &str) -> String {
        let re_whitespace = regex_lite::Regex::new(r"\s+").unwrap();
        let collapsed = re_whitespace.replace_all(text, " ");
        let truncated: String = collapsed.chars().take(self.max_text_length).collect();
        truncated.trim().to_string()
    }

    fn relevance_score(&self, text: &str, context: &str) -> f64 {
        let text_lower = text.to_lowercase();
        let context_lower = context.to_lowercase();
        let keywords: Vec<&str> = context_lower.split_whitespace().filter(|w| w.len() > 3).collect();
        if keywords.is_empty() {
            return 0.5;
        }
        let matches = keywords
            .iter()
            .filter(|k| text_lower.contains(*k))
            .count();
        matches as f64 / keywords.len() as f64
    }
}

#[async_trait]
impl ContentExtractor for WebExtractor {
    fn name(&self) -> &'static str {
        "web"
    }

    async fn extract(&self, result: &SearchResult, context: &str) -> Result<ExtractedContent> {
        let resp = self
            .client
            .get(&result.url)
            .send()
            .await
            .map_err(|e| Error::Extract(format!("Failed to fetch {}: {e}", result.url)))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(Error::Extract(format!(
                "HTTP {status} for {}",
                result.url
            )));
        }

        let html = resp
            .text()
            .await
            .map_err(|e| Error::Extract(format!("Failed to read body: {e}")))?;

        let text = self.extract_main_content(&html);
        let relevance = self.relevance_score(&text, context);

        Ok(ExtractedContent {
            url: result.url.clone(),
            title: result.title.clone(),
            text,
            relevance_score: relevance,
        })
    }
}

impl Default for WebExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clean_text_preserves_cjk() {
        let e = WebExtractor::new();
        let result = e.clean_text("你好世界");
        assert!(result.contains("你好世界"));
    }

    #[test]
    fn test_clean_text_collapses_whitespace() {
        let e = WebExtractor::new();
        let result = e.clean_text("a   b\t\tc\n\nd");
        assert_eq!(result, "a b c d");
    }

    #[test]
    fn test_clean_text_truncates() {
        let mut e = WebExtractor::new();
        e.max_text_length = 5;
        let result = e.clean_text("abcdefghij");
        assert_eq!(result.len(), 5);
    }

    #[test]
    fn test_clean_text_trimmed() {
        let e = WebExtractor::new();
        let result = e.clean_text("  hello world  ");
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_clean_text_cjk_not_filtered() {
        let e = WebExtractor::new();
        let result = e.clean_text("国产AUTOSAR工具链现状与未来发展趋势分析报告");
        assert!(result.contains("国产AUTOSAR"));
    }

    #[test]
    fn test_relevance_score_matches() {
        let e = WebExtractor::new();
        let score = e.relevance_score("国产AUTOSAR工具链发展现状分析", "AUTOSAR工具链");
        assert!(score > 0.0);
    }

    #[test]
    fn test_relevance_score_no_match() {
        let e = WebExtractor::new();
        let score = e.relevance_score("今天天气不错", "AUTOSAR");
        assert_eq!(score, 0.0);
    }

    #[test]
    fn test_extract_main_content_gets_body_text() {
        let e = WebExtractor::new();
        let html = "<html><body>Hello 世界</body></html>";
        let result = e.extract_main_content(html);
        assert!(result.contains("Hello"));
        assert!(result.contains("世界"));
    }
}
