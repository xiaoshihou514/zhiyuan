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
        let cleaned: String = text
            .chars()
            .filter(|c| c.is_ascii_graphic() || c.is_ascii_whitespace() || c.is_ascii_punctuation() || c.is_whitespace())
            .collect();
        let re_whitespace = regex_lite::Regex::new(r"\s+").unwrap();
        let collapsed = re_whitespace.replace_all(&cleaned, " ");
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
