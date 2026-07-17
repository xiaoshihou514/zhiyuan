use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;
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
    cache: Mutex<HashMap<String, String>>,
}

impl WebExtractor {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .build()
            .expect("Failed to create HTTP client");
        Self {
            client,
            max_text_length: 100_000,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn extract_main_content(&self, html: &str, url: &str) -> String {
        self.dom_smoothie_extract(html, url)
            .map(|text| self.clean_text(&text))
            .unwrap_or_else(|| self.legacy_extract(html))
    }

    fn dom_smoothie_extract(&self, html: &str, url: &str) -> Option<String> {
        use dom_smoothie::{Config, Readability, TextMode};

        let cfg = Config {
            text_mode: TextMode::Markdown,
            ..Default::default()
        };
        let mut readable = Readability::new(html, Some(url), Some(cfg)).ok()?;
        let article = readable.parse().ok()?;
        let text = article.text_content.to_string();
        if text.trim().is_empty() { None } else { Some(text) }
    }

    fn legacy_extract(&self, html: &str) -> String {
        let re_block = regex_lite::Regex::new(r"(?is)<(script|style|noscript|iframe)[^>]*>.*?</(?:script|style|noscript|iframe)>").unwrap();
        let html = re_block.replace_all(html, "");
        let re_tag = regex_lite::Regex::new(r"<[^>]*>").unwrap();
        let text = re_tag.replace_all(&html, " ");

        let garbage = [
            ".css-", "g-recaptcha", "elementor", "grecaptcha", "recaptcha",
            "Skip to main content", "document.", "window.",
        ];
        let sentences: String = text
            .split(|c: char| c == '。' || c == '.' || c == '！' || c == '?')
            .filter(|s| {
                let t = s.trim();
                t.len() > 15 && !garbage.iter().any(|p| t.contains(p))
            })
            .collect::<Vec<_>>()
            .join("。");
        self.clean_text(&sentences)
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

    async fn extract_pdf(&self, url: &str) -> Result<String> {
        let bytes = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| Error::Extract(format!("PDF 下载失败 {}: {e}", url)))?
            .bytes()
            .await
            .map_err(|e| Error::Extract(format!("PDF 读取失败 {}: {e}", url)))?;

        let result = tokio::time::timeout(Duration::from_secs(120), async {
            tokio::task::spawn_blocking(move || {
                use pdf_oxide::PdfDocument;
                let doc = PdfDocument::from_bytes(bytes.to_vec())
                    .map_err(|e| Error::Extract(format!("PDF 打开失败: {e}")))?;
                doc.extract_all_text()
                    .map_err(|e| Error::Extract(format!("PDF 文本提取失败: {e}")))
            })
            .await
            .map_err(|e| Error::Extract(format!("PDF 提取线程失败: {e}")))?
        })
        .await
        .map_err(|_| Error::Extract("PDF 提取超时（2分钟）".into()))?;

        result
    }
}

#[async_trait]
impl ContentExtractor for WebExtractor {
    fn name(&self) -> &'static str {
        "web"
    }

    async fn extract(&self, result: &SearchResult, context: &str) -> Result<ExtractedContent> {
        // 缓存检查：同一 URL 在本次会话中只提取一次
        if let Ok(cache) = self.cache.lock() {
            if let Some(text) = cache.get(&result.url) {
                let relevance = self.relevance_score(text, context);
                return Ok(ExtractedContent {
                    url: result.url.clone(),
                    title: result.title.clone(),
                    text: text.clone(),
                    relevance_score: relevance,
                });
            }
        }

        let url_lower = result.url.to_lowercase();
        let is_pdf = url_lower.ends_with(".pdf");

        let text = if is_pdf {
            self.extract_pdf(&result.url).await?
        } else {
            let resp = self.client.get(&result.url).send().await
                .map_err(|e| Error::Extract(format!("Failed to fetch {}: {e}", result.url)))?;
            let status = resp.status();
            if !status.is_success() {
                return Err(Error::Extract(format!("HTTP {status} for {}", result.url)));
            }
            let content_type = resp.headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");

            if content_type.contains("pdf") && !is_pdf {
                drop(resp);
                self.extract_pdf(&result.url).await?
            } else if is_pdf {
                drop(resp);
                self.extract_pdf(&result.url).await?
            } else {
                let html = resp.text().await
                    .map_err(|e| Error::Extract(format!("Failed to read body: {e}")))?;
                self.extract_main_content(&html, &result.url)
            }
        };

        // 写入缓存
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(result.url.clone(), text.clone());
        }

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
        let html = "<html><body>这是一段足够长的测试文字Hello世界用于验证提取功能是否正常工作没有问题</body></html>";
        let result = e.extract_main_content(html, "https://example.com");
        assert!(result.contains("Hello"), "结果应为: {result}");
        assert!(result.contains("世界"), "结果应为: {result}");
    }

    #[test]
    fn test_extract_main_content_filters_css() {
        let e = WebExtractor::new();
        let html = "\
<html><body>
<p>这是一篇真实文章的内容，包含重要的技术信息</p>
<style>.css-abc123{color:red}</style>
<script>var x=function(){return 1}</script>
<noscript>您的浏览器不支持JavaScript</noscript>
<p>更多有用的正文文字这里继续扩展长度确保超过十五字阈值</p>
</body></html>";
        let result = e.extract_main_content(html, "https://example.com");
        assert!(result.contains("真实文章"), "正文应保留，结果: {result}");
        assert!(!result.contains("function"), "JS 应被完整移除，结果: {result}");
        assert!(!result.contains("您的浏览器"), "noscript 应被移除，结果: {result}");
    }
}
