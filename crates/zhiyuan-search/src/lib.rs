use async_trait::async_trait;
use chrono::Utc;
use std::time::Duration;
use zhiyuan_core::{Result as CoreResult, SearchQuery, SearchResult};

#[async_trait]
pub trait SearchEngine: Send + Sync {
    async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>>;
    fn name(&self) -> &'static str;
}

pub struct SearXngEngine {
    client: reqwest::Client,
    base_url: String,
    max_results: usize,
}

impl SearXngEngine {
    pub fn new(base_url: &str, max_results: usize) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            max_results,
        }
    }
}

#[async_trait]
impl SearchEngine for SearXngEngine {
    fn name(&self) -> &'static str {
        "searxng"
    }

    async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>> {
        let resp = self
            .client
            .get(format!("{}/search", self.base_url))
            .query(&[
                ("q", &query.query),
                ("format", &"json".to_string()),
                ("categories", &"general".to_string()),
                ("language", &"all".to_string()),
                ("safesearch", &"0".to_string()),
            ])
            .send()
            .await
            .map_err(|e| zhiyuan_core::Error::Search(format!("SearXNG 请求失败: {e}")))?;

        let body = resp
            .text()
            .await
            .map_err(|e| zhiyuan_core::Error::Search(format!("SearXNG 读取响应失败: {e}")))?;

        #[derive(serde::Deserialize)]
        struct SearxngResponse {
            results: Vec<SearxngResult>,
        }

        #[derive(serde::Deserialize)]
        struct SearxngResult {
            title: String,
            url: String,
            #[serde(default)]
            content: String,
            #[serde(default)]
            engine: String,
        }

        let parsed: SearxngResponse = serde_json::from_str(&body)
            .map_err(|e| zhiyuan_core::Error::Search(format!("SearXNG 解析 JSON 失败: {e}")))?;

        let results: Vec<SearchResult> = parsed
            .results
            .into_iter()
            .take(self.max_results)
            .map(|r| SearchResult {
                title: r.title,
                url: r.url,
                snippet: r.content,
                source: format!("searxng/{}", r.engine),
                fetch_time: Utc::now(),
            })
            .collect();

        Ok(results)
    }
}

pub struct EnginePool {
    engines: Vec<Box<dyn SearchEngine>>,
    fallback_order: Vec<usize>,
    #[allow(dead_code)]
    failure_count: Vec<u32>,
}

impl EnginePool {
    pub fn new(engines: Vec<Box<dyn SearchEngine>>) -> Self {
        let len = engines.len();
        Self {
            engines,
            fallback_order: (0..len).collect(),
            failure_count: vec![0; len],
        }
    }

    pub fn from_config(config: &zhiyuan_core::SearchConfig) -> Self {
        let engines: Vec<Box<dyn SearchEngine>> = vec![Box::new(SearXngEngine::new(
            &config.searxng_url,
            config.max_results,
        ))];
        Self::new(engines)
    }

    pub async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>> {
        let mut last_err = None;
        for &idx in &self.fallback_order {
            match self.engines[idx].search(query).await {
                Ok(results) if !results.is_empty() => {
                    let deduped = dedup_results(results);
                    let titles: Vec<&str> = deduped.iter().map(|r| r.title.as_str()).collect();
                    tracing::info!("引擎" = %self.engines[idx].name(), "数量" = %deduped.len(), "标题" = ?titles, "搜索返回");
                    let results = filter_relevant(&query.query, deduped);
                    if !results.is_empty() {
                        return Ok(results);
                    }
                }
                Ok(_) => {
                    tracing::warn!("引擎" = %self.engines[idx].name(), "搜索返回空结果");
                }
                Err(e) => {
                    tracing::warn!("引擎" = %self.engines[idx].name(), "错误" = %e, "搜索失败");
                    last_err = Some(e);
                }
            }
        }
        Err(last_err
            .unwrap_or_else(|| zhiyuan_core::Error::Search("all search engines failed".into())))
    }

    pub fn engine_count(&self) -> usize {
        self.engines.len()
    }

    pub async fn search_all(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>> {
        use futures::future::join_all;
        use std::collections::HashMap;

        let futures: Vec<_> = self.engines.iter().map(|e| e.search(query)).collect();
        let results = join_all(futures).await;

        let mut seen: HashMap<String, (SearchResult, Vec<String>)> = HashMap::new();
        let mut engine_count = 0;

        for (i, result) in results.iter().enumerate() {
            match result {
                Ok(results) if !results.is_empty() => {
                    engine_count += 1;
                    let engine_name = self.engines[i].name();
                    tracing::info!("引擎" = engine_name, "数量" = %results.len(), "跨引擎搜索贡献数据");
                    for r in results {
                        let key = normalize_url(&r.url);
                        match seen.get_mut(&key) {
                            Some((_, engines)) => engines.push(engine_name.to_string()),
                            None => {
                                seen.insert(key, (r.clone(), vec![engine_name.to_string()]));
                            }
                        }
                    }
                }
                Ok(_) => tracing::warn!("引擎" = %self.engines[i].name(), "跨引擎搜索返回空"),
                Err(e) => {
                    tracing::warn!("引擎" = %self.engines[i].name(), "错误" = %e, "跨引擎搜索失败")
                }
            }
        }

        let mut all_results: Vec<SearchResult> = seen
            .into_iter()
            .map(|(_url, (mut result, engines))| {
                result.source = engines.join(",");
                result
            })
            .collect();
        all_results.sort_by(|a, b| {
            b.source
                .matches(',')
                .count()
                .cmp(&a.source.matches(',').count())
        });

        tracing::info!(
            "引擎数" = engine_count,
            "总结果" = all_results.len(),
            "跨引擎搜索完成"
        );
        if all_results.is_empty() {
            return Err(zhiyuan_core::Error::Search(
                "all engines returned no results".into(),
            ));
        }
        Ok(all_results)
    }
}

fn normalize_url(url: &str) -> String {
    let url = url.trim().trim_end_matches('/').trim_end_matches('#');
    if let Some(hash_pos) = url.find('#') {
        url[..hash_pos].trim_end_matches('/').to_lowercase()
    } else {
        url.to_lowercase()
    }
}

fn extract_keywords(query: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut prev_ascii = None;
    for c in query.chars() {
        let is_ascii = c.is_ascii_alphanumeric();
        if let Some(prev) = prev_ascii {
            if prev != is_ascii && !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
        }
        if !c.is_whitespace() {
            current.push(c);
        } else if !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        if c.is_ascii_alphanumeric() || c.is_alphabetic() {
            prev_ascii = Some(is_ascii);
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
        .into_iter()
        .filter(|w| w.chars().count() > 2)
        .collect()
}

fn filter_relevant(query: &str, results: Vec<SearchResult>) -> Vec<SearchResult> {
    let keywords: Vec<String> = extract_keywords(query)
        .into_iter()
        .map(|w| w.to_lowercase())
        .collect();
    if keywords.is_empty() {
        return results;
    }
    let fallback = results.clone();
    let filtered: Vec<_> = results
        .into_iter()
        .filter(|r| {
            let text = format!("{} {}", r.title, r.snippet).to_lowercase();
            keywords.iter().any(|k| text.contains(k))
        })
        .collect();
    if filtered.is_empty() {
        fallback
    } else {
        filtered
    }
}

fn dedup_results(results: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut seen = std::collections::HashSet::new();
    results
        .into_iter()
        .filter(|r| seen.insert(normalize_url(&r.url)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(title: &str) -> SearchResult {
        SearchResult {
            title: title.into(),
            url: "https://example.com".into(),
            snippet: "".into(),
            source: "searxng/test".into(),
            fetch_time: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_normalize_url_strips_trailing_slash() {
        assert_eq!(normalize_url("https://example.com/"), "https://example.com");
    }
    #[test]
    fn test_normalize_url_removes_fragment() {
        assert_eq!(
            normalize_url("https://example.com/page#section"),
            "https://example.com/page"
        );
    }
    #[test]
    fn test_normalize_url_lowercases() {
        assert_eq!(
            normalize_url("HTTPS://EXAMPLE.COM/Path"),
            "https://example.com/path"
        );
    }
    #[test]
    fn test_normalize_url_handles_mixed() {
        assert_eq!(normalize_url("  HTTP://A.COM/B/#ref "), "http://a.com/b");
    }

    #[test]
    fn test_dedup_removes_duplicates() {
        let mut r1 = make_result("1");
        r1.url = "https://a.com/page".into();
        let mut r2 = make_result("2");
        r2.url = "https://A.COM/page/".into();
        assert_eq!(dedup_results(vec![r1, r2]).len(), 1);
    }
    #[test]
    fn test_dedup_preserves_unique() {
        let mut r1 = make_result("1");
        r1.url = "https://a.com/one".into();
        let mut r2 = make_result("2");
        r2.url = "https://b.com/two".into();
        assert_eq!(dedup_results(vec![r1, r2]).len(), 2);
    }
    #[test]
    fn test_dedup_empty() {
        assert!(dedup_results(vec![]).is_empty());
    }

    #[test]
    fn test_extract_keywords_cjk_mixed() {
        let kws = extract_keywords("国产AUTOSAR工具链 厂商 产品 版本");
        assert!(kws.contains(&"AUTOSAR".to_string()), "应包含 AUTOSAR");
        assert!(kws.contains(&"工具链".to_string()), "应包含 工具链");
        assert!(!kws.contains(&"国产".to_string()), "国产 ≤2 字符应被过滤");
    }
    #[test]
    fn test_filter_keeps_relevant_generic() {
        let filtered = filter_relevant(
            "汽车嵌入式 AUTOSAR 软件平台 调研",
            vec![
                make_result("国产AUTOSAR工具链的技术亮点与实际应用 - 知乎"),
                make_result("AUTOSAR CP三大工具链全景对比 - 知乎"),
                make_result("AUTOSAR基础软件选型指南"),
                make_result("汽车电子AUTOSAR架构深度解析"),
            ],
        );
        assert_eq!(filtered.len(), 4, "含 AUTOSAR 的结果应全部保留");
    }
    #[test]
    fn test_filter_removes_irrelevant_generic() {
        let results = vec![
            make_result("AUTOSAR Classic and Adaptive | Vector"),
            make_result("国产电影大全 - 最新国产片推荐"),
            make_result("国产剧免费在线观看_国产剧排行榜"),
            make_result("The Ultimate Guide to AUTOSAR - Acsia"),
        ];
        let filtered = filter_relevant("汽车电子 AUTOSAR 供应商 对比", results);
        assert_eq!(filtered.len(), 2);
        for r in &filtered {
            assert!(
                r.title.contains("Vector") || r.title.contains("Acsia"),
                "只应保留 AUTOSAR 结果"
            );
        }
    }
    #[test]
    fn test_filter_fallback_on_empty() {
        let filtered = filter_relevant("ab", vec![make_result("anything"), make_result("else")]);
        assert_eq!(filtered.len(), 2);
    }
}
