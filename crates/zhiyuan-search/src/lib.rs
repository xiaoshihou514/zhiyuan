use async_trait::async_trait;
use chrono::Utc;
use scraper::{Html, Selector};
use std::time::Duration;
use zhiyuan_core::{Result as CoreResult, SearchQuery, SearchResult};

#[async_trait]
pub trait SearchEngine: Send + Sync {
    async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>>;
    fn name(&self) -> &'static str;
}

pub struct BingEngine {
    client: reqwest::Client,
    max_results: usize,
}

impl BingEngine {
    pub fn new(max_results: usize) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
        Self { client, max_results }
    }
}

#[async_trait]
impl SearchEngine for BingEngine {
    fn name(&self) -> &'static str {
        "bing"
    }

    async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>> {
        let html = self
            .client
            .get("https://www.bing.com/search")
            .query(&[("q", &query.query)])
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
            .send()
            .await
            .map_err(|e| zhiyuan_core::Error::Search(format!("Bing request failed: {e}")))?
            .text()
            .await
            .map_err(|e| zhiyuan_core::Error::Search(format!("Bing read failed: {e}")))?;

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

                let url = algo
                    .select(&h2_sel)
                    .next()
                    .and_then(|h2| {
                        h2.select(&Selector::parse("a").unwrap())
                            .next()
                            .and_then(|a| a.value().attr("href"))
                    })
                    .unwrap_or("")
                    .to_string();

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

pub struct GoogleEngine {
    client: reqwest::Client,
    max_results: usize,
}

impl GoogleEngine {
    pub fn new(max_results: usize) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
        Self { client, max_results }
    }
}

#[async_trait]
impl SearchEngine for GoogleEngine {
    fn name(&self) -> &'static str {
        "google"
    }

    async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>> {
        let html = self
            .client
            .get("https://www.google.com/search")
            .query(&[("q", &query.query)])
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
            .send()
            .await
            .map_err(|e| zhiyuan_core::Error::Search(format!("Google request failed: {e}")))?
            .text()
            .await
            .map_err(|e| zhiyuan_core::Error::Search(format!("Google read failed: {e}")))?;

        let doc = Html::parse_document(&html);
        let result_sel = Selector::parse("div.g")
            .map_err(|_| zhiyuan_core::Error::Search("Google selector parse error".into()))?;
        let h3_sel = Selector::parse("h3")
            .map_err(|_| zhiyuan_core::Error::Search("Google h3 selector parse error".into()))?;
        let link_sel = Selector::parse("a")
            .map_err(|_| zhiyuan_core::Error::Search("Google link selector parse error".into()))?;
        let snippet_sel = Selector::parse(".VwiC3b")
            .map_err(|_| zhiyuan_core::Error::Search("Google snippet selector parse error".into()))?;

        let results: Vec<SearchResult> = doc
            .select(&result_sel)
            .take(self.max_results)
            .filter_map(|g| {
                let h3 = g.select(&h3_sel).next()?;
                let title = h3.text().collect::<String>().trim().to_string();
                if title.is_empty() {
                    return None;
                }

                let url = g
                    .select(&link_sel)
                    .next()
                    .and_then(|a| {
                        let href = a.value().attr("href")?;
                        // Google 搜索结果链接格式：/url?q=REAL_URL&...
                        let decoded = urlencoding::decode(href).ok()?;
                        let decoded = decoded.trim();
                        if let Some(q_pos) = decoded.find("?q=") {
                            let start = q_pos + 3;
                            let end = decoded[start..].find('&').map(|i| start + i).unwrap_or(decoded.len());
                            Some(decoded[start..end].to_string())
                        } else if decoded.starts_with("http://") || decoded.starts_with("https://") {
                            Some(decoded.to_string())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_default();

                if url.is_empty() {
                    return None;
                }

                let snippet = g
                    .select(&snippet_sel)
                    .next()
                    .map(|s| s.text().collect::<String>().trim().to_string())
                    .unwrap_or_default();

                Some(SearchResult {
                    title,
                    url,
                    snippet,
                    source: "google".into(),
                    fetch_time: Utc::now(),
                })
            })
            .collect();

        Ok(results)
    }
}

pub struct DuckDuckGoEngine {
    client: reqwest::Client,
    max_results: usize,
}

impl DuckDuckGoEngine {
    pub fn new(max_results: usize) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            client,
            max_results,
        }
    }
}

#[async_trait]
impl SearchEngine for DuckDuckGoEngine {
    fn name(&self) -> &'static str {
        "duckduckgo"
    }

    async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>> {
        let html = self
            .client
            .get("https://html.duckduckgo.com/html/")
            .query(&[("q", &query.query)])
            .send()
            .await
            .map_err(|e| zhiyuan_core::Error::Search(format!("DDG request failed: {e}")))?
            .text()
            .await
            .map_err(|e| zhiyuan_core::Error::Search(format!("DDG read failed: {e}")))?;

        let doc = Html::parse_document(&html);
        let link_sel =
            Selector::parse("a.result__a").map_err(|_| zhiyuan_core::Error::Search("Selector parse error".into()))?;
        let snippet_sel =
            Selector::parse("a.result__snippet").map_err(|_| zhiyuan_core::Error::Search("Selector parse error".into()))?;

        let results: Vec<SearchResult> = doc
            .select(&link_sel)
            .zip(doc.select(&snippet_sel))
            .take(self.max_results)
            .map(|(a, s)| {
                let title = a.text().collect::<String>().trim().to_string();
                let url = a
                    .value()
                    .attr("href")
                    .unwrap_or("")
                    .to_string();
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
        let mut engines: Vec<Box<dyn SearchEngine>> = Vec::new();

        engines.push(Box::new(BingEngine::new(config.ddg_max_results)));
        engines.push(Box::new(GoogleEngine::new(config.ddg_max_results)));
        engines.push(Box::new(DuckDuckGoEngine::new(config.ddg_max_results)));

        Self::new(engines)
    }

    pub async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>> {
        let mut last_err = None;
        for &idx in &self.fallback_order {
            match self.engines[idx].search(query).await {
                Ok(results) if !results.is_empty() => {
                    tracing::info!(engine = %self.engines[idx].name(), count = %results.len(), "search succeeded");
                    return Ok(results);
                }
                Ok(_) => {
                    tracing::warn!(engine = %self.engines[idx].name(), "search returned empty results");
                }
                Err(e) => {
                    tracing::warn!(engine = %self.engines[idx].name(), error = %e, "search failed");
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
                        engine = engine_name,
                        count = %results.len(),
                        "cross-search contributed"
                    );
                    for r in results {
                        let key = r.url.clone();
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
                    tracing::warn!(engine = %self.engines[i].name(), "cross-search returned empty");
                }
                Err(e) => {
                    tracing::warn!(engine = %self.engines[i].name(), error = %e, "cross-search failed");
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
            engine_count,
            total_results = all_results.len(),
            "cross-search completed"
        );

        if all_results.is_empty() {
            return Err(zhiyuan_core::Error::Search("all engines returned no results".into()));
        }

        Ok(all_results)
    }
}
