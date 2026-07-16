use async_trait::async_trait;
use base64::Engine;
use chrono::Utc;
use scraper::{Html, Selector};
use std::time::Duration;
use zhiyuan_core::{Result as CoreResult, SearchQuery, SearchResult};

fn decode_bing_url(href: &str) -> String {
    if !href.contains("bing.com/ck/") {
        return href.to_string();
    }
    for prefix in ["&u=a1", "&u=ao"] {
        if let Some(u_start) = href.find(prefix) {
            let encoded = &href[u_start + prefix.len()..];
            let end = encoded.find('&').unwrap_or(encoded.len());
            let b64 = &encoded[..end];
            let engine = base64::engine::general_purpose::STANDARD;
            if let Ok(decoded) = engine.decode(b64) {
                if let Ok(url) = String::from_utf8(decoded) {
                    return url;
                }
            }
        }
    }
    href.to_string()
}

fn decode_ddg_url(href: &str) -> String {
    if !href.contains("duckduckgo.com/l/") {
        return href.to_string();
    }
    if let Some(u_start) = href.find("uddg=") {
        let encoded = &href[u_start + 5..];
        let end = encoded.find('&').unwrap_or(encoded.len());
        if let Ok(decoded) = urlencoding::decode(&encoded[..end]) {
            return decoded.to_string();
        }
    }
    href.to_string()
}

fn normalize_query(query: &str) -> String {
    let mut out = String::with_capacity(query.len() + 8);
    let mut prev_cjk = false;
    for c in query.chars() {
        let is_cjk = ('\u{4e00}'..='\u{9fff}').contains(&c)
            || ('\u{3400}'..='\u{4dbf}').contains(&c)
            || ('\u{f900}'..='\u{faff}').contains(&c);
        if !out.is_empty() && prev_cjk != is_cjk {
            out.push(' ');
        }
        out.push(c);
        prev_cjk = is_cjk;
    }
    out
}

#[async_trait]
pub trait SearchEngine: Send + Sync {
    async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>>;
    fn name(&self) -> &'static str;
}

pub struct BingEngine {
    client: reqwest::Client,
    max_results: usize,
    searches_dir: Option<std::path::PathBuf>,
}

impl BingEngine {
    pub fn new(max_results: usize) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            client,
            max_results,
            searches_dir: None,
        }
    }

    pub fn with_searches_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.searches_dir = Some(dir);
        self
    }
}

#[async_trait]
impl SearchEngine for BingEngine {
    fn name(&self) -> &'static str {
        "bing"
    }

    async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>> {
        let q = normalize_query(&query.query);
        let html = self
            .client
            .get("https://cn.bing.com/search")
            .query(&[
                ("q", q.as_str()),
                ("setlang", "zh-Hans"),
                ("ensearch", "1"),
                ("FORM", "BESBTB"),
            ])
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .send()
            .await
            .map_err(|e| zhiyuan_core::Error::Search(format!("Bing request failed: {e}")))?
            .text()
            .await
            .map_err(|e| zhiyuan_core::Error::Search(format!("Bing read failed: {e}")))?;

        if let Some(dir) = &self.searches_dir {
            std::fs::create_dir_all(dir).ok();
            let safe_name = normalize_query(&query.query).chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(100).collect::<String>();
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_micros()).unwrap_or(0);
            let filepath = dir.join(format!("{}_{}.html", ts, safe_name));
            if let Err(e) = std::fs::write(&filepath, &html) {
                tracing::warn!("保存 Bing HTML 失败 {}: {e}", filepath.display());
            }
        }

        let doc = Html::parse_document(&html);
        let algo_sel = Selector::parse("li.b_algo")
            .map_err(|_| zhiyuan_core::Error::Search("Bing selector parse error".into()))?;
        let h2_sel = Selector::parse("h2")
            .map_err(|_| zhiyuan_core::Error::Search("Bing h2 selector parse error".into()))?;
        let caption_sel = Selector::parse(".b_caption p")
            .map_err(|_| zhiyuan_core::Error::Search("Bing caption selector parse error".into()))?;

        let results: Vec<SearchResult> = doc
            .select(&algo_sel)
            .take(self.max_results)
            .map(|algo| {
                let title = algo
                    .select(&h2_sel)
                    .next()
                    .map(|h2| h2.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();

                let url = decode_bing_url(
                    &algo
                        .select(&h2_sel)
                        .next()
                        .and_then(|h2| {
                            h2.select(&Selector::parse("a").unwrap())
                                .next()
                                .and_then(|a| a.value().attr("href"))
                        })
                        .unwrap_or("")
                );

                let snippet = algo
                    .select(&caption_sel)
                    .next()
                    .map(|p| p.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();

                SearchResult {
                    title,
                    url,
                    snippet,
                    source: "bing".into(),
                    fetch_time: Utc::now(),
                }
            })
            .collect();

        Ok(results)
    }
}

pub struct StartpageEngine {
    max_results: usize,
}

impl StartpageEngine {
    pub fn new(max_results: usize) -> Self {
        Self { max_results }
    }
}

#[async_trait]
impl SearchEngine for StartpageEngine {
    fn name(&self) -> &'static str {
        "startpage"
    }

    async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>> {
        let ua_list = [
            "Mozilla/5.0 (compatible; Googlebot/2.1; +http://www.google.com/bot.html)",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            "Mozilla/5.0 (compatible; Bingbot/2.0; +http://www.bing.com/bingbot.htm)",
        ];

        for ua in &ua_list {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("Failed to create HTTP client");

            let resp = match client
                .get("https://www.startpage.com/sp/search")
                .query(&[("query", &query.query)])
                .header("User-Agent", *ua)
                .send()
                .await
            {
                Ok(r) => r,
                Err(_) => continue,
            };

            let html = match resp.text().await {
                Ok(h) => h,
                Err(_) => continue,
            };

            let link_sel = Selector::parse("a.result-title.result-link");
            let desc_sel = Selector::parse(".description");
            let (Ok(link_sel), Ok(desc_sel)) = (link_sel, desc_sel) else { continue };

            let doc = Html::parse_document(&html);
            let results: Vec<SearchResult> = doc
                .select(&link_sel)
                .zip(doc.select(&desc_sel))
                .take(self.max_results)
                .map(|(a, d)| {
                    let raw_title: String = a.text()
                        .filter(|s| !s.trim().is_empty() && !s.contains('{') && !s.starts_with(".css-"))
                        .collect();
                    let title = raw_title.split_whitespace().collect::<Vec<_>>().join(" ");
                    SearchResult {
                        title,
                        url: a.value().attr("href").unwrap_or("").to_string(),
                        snippet: d.text().collect::<String>().trim().to_string(),
                        source: "startpage".into(),
                        fetch_time: Utc::now(),
                    }
                })
                .collect();

            if !results.is_empty() {
                return Ok(results);
            }
        }

        Ok(vec![])
    }
}

pub struct DuckDuckGoEngine {
    max_results: usize,
}

impl DuckDuckGoEngine {
    pub fn new(max_results: usize) -> Self {
        Self { max_results }
    }
}

#[async_trait]
impl SearchEngine for DuckDuckGoEngine {
    fn name(&self) -> &'static str {
        "duckduckgo"
    }

    async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>> {
        let ua_list = [
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:120.0) Gecko/20100101 Firefox/120.0",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        ];

        for ua in &ua_list {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("Failed to create HTTP client");

            let resp = match client
                .get("https://html.duckduckgo.com/html/")
                .query(&[("q", &query.query)])
                .header("User-Agent", *ua)
                .send()
                .await
            {
                Ok(r) => r,
                Err(_) => continue,
            };

            let html = match resp.text().await {
                Ok(h) => h,
                Err(_) => continue,
            };

            let doc = Html::parse_document(&html);
            let link_sel = match Selector::parse("a.result__a") {
                Ok(s) => s,
                Err(_) => continue,
            };
            let snippet_sel = match Selector::parse("a.result__snippet") {
                Ok(s) => s,
                Err(_) => continue,
            };

            let results: Vec<SearchResult> = doc
                .select(&link_sel)
                .zip(doc.select(&snippet_sel))
                .take(self.max_results)
                .map(|(a, s)| {
                    let title = a.text().collect::<String>().trim().to_string();
                    let url = decode_ddg_url(
                        a.value()
                            .attr("href")
                            .unwrap_or("")
                    );
                    let snippet = s.text().collect::<String>().trim().to_string();
                    SearchResult {
                        title,
                        url,
                        snippet,
                        source: "duckduckgo".into(),
                        fetch_time: Utc::now(),
                    }
                })
                .collect();

            if !results.is_empty() {
                return Ok(results);
            }
        }

        Ok(vec![])
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
        match prev_ascii {
            Some(prev) if prev != is_ascii && !current.is_empty() => {
                words.push(std::mem::take(&mut current));
            }
            _ => {}
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
    let need = if keywords.len() >= 3 { 2 } else { 1 };
    let fallback = results.clone();
    let filtered: Vec<_> = results
        .into_iter()
        .filter(|r| {
            let text = format!("{} {}", r.title, r.snippet).to_lowercase();
            keywords.iter().filter(|k| text.contains(*k)).count() >= need
        })
        .collect();
    if filtered.is_empty() { fallback } else { filtered }
}

fn dedup_results(results: Vec<SearchResult>) -> Vec<SearchResult> {
    let mut seen = std::collections::HashSet::new();
    results
        .into_iter()
        .filter(|r| seen.insert(normalize_url(&r.url)))
        .collect()
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

    pub fn from_config(config: &zhiyuan_core::SearchConfig, searches_dir: Option<std::path::PathBuf>) -> Self {
        let bing = if let Some(ref dir) = searches_dir {
            BingEngine::new(config.max_results).with_searches_dir(dir.join("bing"))
        } else {
            BingEngine::new(config.max_results)
        };
        let engines: Vec<Box<dyn SearchEngine>> = vec![
            Box::new(bing),
            Box::new(StartpageEngine::new(config.max_results)),
            Box::new(DuckDuckGoEngine::new(config.max_results)),
        ];

        Self::new(engines)
    }

    pub async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>> {
        let mut last_err = None;
        for &idx in &self.fallback_order {
            match self.engines[idx].search(query).await {
                Ok(results) if !results.is_empty() => {
                    let deduped = dedup_results(results);
                    tracing::info!("引擎" = %self.engines[idx].name(), "数量" = %deduped.len(), "结果" = ?deduped.iter().map(|r| format!("{} ({})", r.title, r.url)).collect::<Vec<_>>(), "搜索返回");
                    let results = filter_relevant(&query.query, deduped);
                    return Ok(results);
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
        Err(last_err.unwrap_or_else(|| {
            zhiyuan_core::Error::Search("all search engines failed".into())
        }))
    }

    pub fn engine_count(&self) -> usize {
        self.engines.len()
    }

    pub async fn search_all(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>> {
        use futures::future::join_all;
        use std::collections::HashMap;

        let futures: Vec<_> = self.engines.iter().map(|e| e.search(query)).collect();
        let results = join_all(futures).await;

        // url -> (SearchResult, engine_names)
        let mut seen: HashMap<String, (SearchResult, Vec<String>)> = HashMap::new();
        let mut engine_count = 0;

        for (i, result) in results.iter().enumerate() {
            match result {
                Ok(results) if !results.is_empty() => {
                    engine_count += 1;
                    let engine_name = self.engines[i].name();
                    tracing::info!(
                        "引擎" = engine_name,
                        "数量" = %results.len(),
                        "跨引擎搜索贡献数据"
                    );
                    for r in results {
                        let key = normalize_url(&r.url);
                        match seen.get_mut(&key) {
                            Some((_, engines)) => {
                                engines.push(engine_name.to_string());
                            }
                            None => {
                                seen.insert(key, (r.clone(), vec![engine_name.to_string()]));
                            }
                        }
                    }
                }
                Ok(_) => {
                    tracing::warn!("引擎" = %self.engines[i].name(), "跨引擎搜索返回空");
                }
                Err(e) => {
                    tracing::warn!("引擎" = %self.engines[i].name(), "错误" = %e, "跨引擎搜索失败");
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

        // Sort by cross-engine count (most verified first)
        all_results.sort_by(|a, b| {
            let a_count = a.source.matches(',').count();
            let b_count = b.source.matches(',').count();
            b_count.cmp(&a_count)
        });

        tracing::info!(
            "引擎数" = engine_count,
            "总结果" = all_results.len(),
            "跨引擎搜索完成"
        );

        if all_results.is_empty() {
            return Err(zhiyuan_core::Error::Search("all engines returned no results".into()));
        }

        Ok(all_results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_url_strips_trailing_slash() {
        assert_eq!(
            normalize_url("https://example.com/"),
            "https://example.com"
        );
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
        assert_eq!(
            normalize_url("  HTTP://A.COM/B/#ref "),
            "http://a.com/b"
        );
    }

    #[test]
    fn test_dedup_removes_duplicates() {
        let results = vec![
            SearchResult {
                url: "https://a.com/page".into(),
                title: "1".into(),
                snippet: "".into(),
                source: "bing".into(),
                fetch_time: chrono::Utc::now(),
            },
            SearchResult {
                url: "https://A.COM/page/".into(),
                title: "2".into(),
                snippet: "".into(),
                source: "bing".into(),
                fetch_time: chrono::Utc::now(),
            },
        ];
        let deduped = dedup_results(results);
        assert_eq!(deduped.len(), 1);
    }

    #[test]
    fn test_dedup_preserves_unique() {
        let results = vec![
            SearchResult {
                url: "https://a.com/one".into(),
                title: "".into(),
                snippet: "".into(),
                source: "bing".into(),
                fetch_time: chrono::Utc::now(),
            },
            SearchResult {
                url: "https://b.com/two".into(),
                title: "".into(),
                snippet: "".into(),
                source: "bing".into(),
                fetch_time: chrono::Utc::now(),
            },
        ];
        let deduped = dedup_results(results);
        assert_eq!(deduped.len(), 2);
    }

    #[test]
    fn test_dedup_empty() {
        assert!(dedup_results(vec![]).is_empty());
    }

    #[test]
    fn test_decode_bing_url_passthrough() {
        let url = "https://zhuanlan.zhihu.com/p/686299087";
        assert_eq!(decode_bing_url(url), url);
    }

    #[test]
    fn test_decode_bing_url_a1() {
        let url = "https://www.bing.com/ck/a?!&&p=xxx&u=a1aHR0cHM6Ly96aHVhbmxhbi56aGlodS5jb20vcC82ODYyOTkwODc=&ntb=1";
        let pos = url.find("&u=a1");
        println!("pos: {:?}", pos);
        assert!(pos.is_some(), "should find &u=a1 in URL");
        let decoded = decode_bing_url(url);
        println!("decoded: {}", decoded);
        assert_eq!(decoded, "https://zhuanlan.zhihu.com/p/686299087");
    }

    #[test]
    fn test_decode_bing_url_no_u_param() {
        let url = "https://www.bing.com/ck/a?!&&p=xxx&ntb=1";
        assert_eq!(decode_bing_url(url), url);
    }

    fn make_result(title: &str) -> SearchResult {
        SearchResult {
            title: title.into(),
            url: "https://example.com".into(),
            snippet: "".into(),
            source: "bing".into(),
            fetch_time: chrono::Utc::now(),
        }
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
        let results = vec![
            make_result("国产AUTOSAR工具链的技术亮点与实际应用 - 知乎"),
            make_result("AUTOSAR CP三大工具链全景对比 - 知乎"),
            make_result("AUTOSAR基础软件选型指南"),
            make_result("汽车电子AUTOSAR架构深度解析"),
        ];
        // 搜索词跟结果标题完全不同，但关键词 AUTOSAR 应该命中
        let filtered = filter_relevant("汽车嵌入式 AUTOSAR 软件平台 调研", results);
        assert_eq!(filtered.len(), 4, "含 AUTOSAR 关键词的结果应全部保留");
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
        assert_eq!(filtered.len(), 2, "只应保留 2 条 AUTOSAR 结果");
        let titles: Vec<&str> = filtered.iter().map(|r| r.title.as_str()).collect();
        assert!(titles.iter().any(|t| t.contains("Vector")));
        assert!(titles.iter().any(|t| t.contains("Acsia")));
        assert!(!titles.iter().any(|t| t.contains("电影")));
        assert!(!titles.iter().any(|t| t.contains("免费在线观看")));
    }

    #[test]
    fn test_filter_fallback_on_empty() {
        let results = vec![
            make_result("国产AUTOSAR三巨头"),
            make_result("something completely unrelated"),
        ];
        // 搜索词不含任何 >2 字符的关键词时不过滤
        let filtered = filter_relevant("ab", results);
        assert_eq!(filtered.len(), 2, "无有效关键词时原样返回");
    }
}
