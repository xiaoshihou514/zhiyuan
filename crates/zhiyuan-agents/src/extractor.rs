use std::sync::Arc;
use zhiyuan_core::{ExtractedContent, Result, SearchResult};
use zhiyuan_extract::ContentExtractor;

pub struct ExtractorAgent {
    extractor: Arc<dyn ContentExtractor + Send + Sync>,
    blocked_domains: Vec<String>,
    /// 每个子任务最多提取的 URL 数
    max_extract_per_task: usize,
}

impl ExtractorAgent {
    pub fn new(
        extractor: Arc<dyn ContentExtractor + Send + Sync>,
        blocked_domains: Vec<String>,
    ) -> Self {
        Self {
            extractor,
            blocked_domains,
            max_extract_per_task: 10,
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

    /// 综合优先级评分：搜索结果标题/摘要与任务描述片段的匹配密度
    fn result_priority_score(result: &SearchResult, fragments: &[String]) -> f64 {
        if fragments.is_empty() {
            return 0.5;
        }
        let text = format!("{} {}", result.title, result.snippet).to_lowercase();
        let match_count = fragments
            .iter()
            .filter(|f| text.contains(f.as_str()))
            .count();
        match_count as f64 / fragments.len() as f64
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
        // 1. 过滤被屏蔽域名和完全不相关的结果
        let fragments = Self::extract_fragments(context);
        let candidates: Vec<&SearchResult> = results
            .iter()
            .filter(|r| !self.is_blocked(&r.url) && self.is_relevant(r, context))
            .collect();

        // 2. 按综合优先级评分排序，取 Top-N
        let mut scored: Vec<(f64, &SearchResult)> = candidates
            .into_iter()
            .map(|r| {
                let score = Self::result_priority_score(r, &fragments);
                (score, r)
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

        let targets: Vec<&SearchResult> = scored
            .into_iter()
            .take(self.max_extract_per_task)
            .map(|(_, r)| r)
            .collect();

        tracing::info!(
            "总数" = %targets.len(),
            "上限" = %self.max_extract_per_task,
            "提取器选定URL（按优先级排序）"
        );

        if !targets.is_empty() {
            tracing::info!("开始内容提取");
        }

        let mut extracted = Vec::new();
        for chunk in targets.chunks(32) {
            let futures: Vec<_> = chunk
                .iter()
                .map(|r| {
                    let url = r.url.clone();
                    async move {
                        let result = self.extractor.extract(r, context).await;
                        (url, result)
                    }
                })
                .collect();
            for (url, result) in futures::future::join_all(futures).await {
                match result {
                    Ok(content) => {
                        tracing::info!(
                            "✓ 提取成功 [{}]: {} 字符",
                            content.title,
                            content.text.len()
                        );
                        extracted.push(content);
                    }
                    Err(e) => tracing::warn!("✗ 提取失败 {}: {e}", url),
                }
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
