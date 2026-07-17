use std::sync::Arc;
use zhiyuan_core::{ExtractedContent, Result, SearchResult};
use zhiyuan_extract::ContentExtractor;

pub struct ExtractorAgent {
    extractor: Arc<dyn ContentExtractor + Send + Sync>,
    blocked_domains: Vec<String>,
}

impl ExtractorAgent {
    pub fn new(
        extractor: Arc<dyn ContentExtractor + Send + Sync>,
        blocked_domains: Vec<String>,
    ) -> Self {
        Self {
            extractor,
            blocked_domains,
        }
    }

    fn is_blocked(&self, url: &str) -> bool {
        self.blocked_domains
            .iter()
            .any(|d| url.contains(d.as_str()))
    }

    /// 从上下文提取搜索片段，用于预筛。
    /// 策略：
    /// 1. 按 CJK / ASCII 边界切分
    /// 2. 原词保留（≥2 字符）
    /// 3. 对 CJK 多字词追加重叠 bigram / trigram
    /// 4. 全小写
    fn extract_fragments(context: &str) -> Vec<String> {
        let mut frags = Vec::new();

        // 按非字母数字/非CJK边界分割
        let raw: Vec<String> = context
            .split(|c: char| {
                !c.is_alphanumeric()
                    && !(c as u32 >= 0x4E00 && c as u32 <= 0x9FFF)
                    && !(c as u32 >= 0x3400 && c as u32 <= 0x4DBF)
            })
            .filter(|s| s.len() >= 2)
            .flat_map(|s| {
                // 进一步在 CJK/ASCII 交界处拆分
                let mut parts = Vec::new();
                let mut buf = String::new();
                let mut is_cjk = false;
                for c in s.chars() {
                    let cur_cjk = (c as u32 >= 0x4E00 && c as u32 <= 0x9FFF)
                        || (c as u32 >= 0x3400 && c as u32 <= 0x4DBF);
                    if !buf.is_empty() && cur_cjk != is_cjk {
                        parts.push(std::mem::take(&mut buf));
                    }
                    buf.push(c);
                    is_cjk = cur_cjk;
                }
                if !buf.is_empty() {
                    parts.push(buf);
                }
                parts
            })
            .filter(|s| s.len() >= 2)
            .collect();

        for word in &raw {
            let lower = word.to_lowercase();
            if !frags.contains(&lower) {
                frags.push(lower);
            }
        }

        // 对 CJK 多字词生成重叠 n-gram
        for word in &raw {
            let chars: Vec<char> = word.chars().collect();
            let is_cjk_word = chars.iter().any(|c| {
                let u = *c as u32;
                (u >= 0x4E00 && u <= 0x9FFF) || (u >= 0x3400 && u <= 0x4DBF)
            });
            if !is_cjk_word || chars.len() < 3 {
                continue;
            }

            // bigram
            for w in chars.windows(2) {
                let ngram: String = w.iter().collect();
                let lower = ngram.to_lowercase();
                if !frags.contains(&lower) {
                    frags.push(lower);
                }
            }
            // trigram
            if chars.len() >= 3 {
                for w in chars.windows(3) {
                    let ngram: String = w.iter().collect();
                    let lower = ngram.to_lowercase();
                    if !frags.contains(&lower) {
                        frags.push(lower);
                    }
                }
            }
        }

        frags
    }

    fn is_relevant(&self, result: &SearchResult, context: &str) -> bool {
        let text = format!("{} {}", result.title, result.snippet).to_lowercase();
        let fragments = Self::extract_fragments(context);
        if fragments.is_empty() {
            return true;
        }
        fragments.iter().any(|f| text.contains(f.as_str()))
    }

    pub async fn extract_content(
        &self,
        results: &[SearchResult],
        context: &str,
    ) -> Result<Vec<ExtractedContent>> {
        let mut extracted = Vec::new();

        tracing::info!("总数" = %results.len(), "提取器选定URL");

        for result in results {
            if self.is_blocked(&result.url) {
                tracing::info!("⏭ 跳过黑名单域名: {}", result.url);
                continue;
            }

            if !self.is_relevant(result, context) {
                tracing::info!(
                    "⏭ 预筛跳过 [{}]: 标题/摘要不匹配",
                    result.title
                );
                continue;
            }

            tracing::debug!(url = %result.url, "正在提取内容");
            match self.extractor.extract(result, context).await {
                Ok(content) => {
                    tracing::info!(
                        "✓ 提取成功 [{}]: {} 字符",
                        content.title,
                        content.text.len()
                    );
                    extracted.push(content);
                }
                Err(e) => tracing::warn!("✗ 提取失败 {}: {e}", result.url),
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
