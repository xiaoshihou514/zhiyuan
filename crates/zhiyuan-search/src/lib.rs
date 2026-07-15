use async_trait::async_trait;
use chrono::Utc;
use scraper::{Html, Selector};
use serde::Deserialize;
use std::time::Duration;
use zhiyuan_core::{Result as CoreResult, SearchQuery, SearchResult};

#[async_trait]
pub trait SearchEngine: Send + Sync {
    async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>>;
    fn name(&self) -> &'static str;
}

pub struct BingEngine {
    api_key: String,
    endpoint: String,
    client: reqwest::Client,
}

impl BingEngine {
    pub fn new(api_key: String, endpoint: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            api_key,
            endpoint,
            client,
        }
    }
}

#[async_trait]
impl SearchEngine for BingEngine {
    fn name(&self) -> &'static str {
        "bing"
    }

    async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>> {
        let resp = self
            .client
            .get(&self.endpoint)
            .header("Ocp-Apim-Subscription-Key", &self.api_key)
            .query(&[("q", &query.query), ("count", &query.max_results.to_string())])
            .send()
            .await
            .map_err(|e| zhiyuan_core::Error::Search(format!("Bing request failed: {e}")))?;

        let body: BingResponse = resp
            .json()
            .await
            .map_err(|e| zhiyuan_core::Error::Search(format!("Bing parse failed: {e}")))?;

        Ok(body
            .web_pages
            .unwrap_or_default()
            .value
            .into_iter()
            .map(|v| SearchResult {
                title: v.name,
                url: v.url,
                snippet: v.snippet,
                source: "bing".into(),
                fetch_time: Utc::now(),
            })
            .collect())
    }
}

#[derive(Deserialize)]
struct BingResponse {
    #[serde(rename = "webPages")]
    web_pages: Option<WebPages>,
}

#[derive(Default, Deserialize)]
struct WebPages {
    value: Vec<BingResult>,
}

#[derive(Deserialize)]
struct BingResult {
    name: String,
    url: String,
    snippet: String,
}

pub struct GoogleEngine {
    api_key: String,
    cse_id: String,
    client: reqwest::Client,
}

impl GoogleEngine {
    pub fn new(api_key: String, cse_id: String) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");
        Self {
            api_key,
            cse_id,
            client,
        }
    }
}

#[async_trait]
impl SearchEngine for GoogleEngine {
    fn name(&self) -> &'static str {
        "google"
    }

    async fn search(&self, query: &SearchQuery) -> CoreResult<Vec<SearchResult>> {
        let resp = self
            .client
            .get("https://www.googleapis.com/customsearch/v1")
            .query(&[
                ("key", &self.api_key),
                ("cx", &self.cse_id),
                ("q", &query.query),
                ("num", &query.max_results.to_string()),
            ])
            .send()
            .await
            .map_err(|e| zhiyuan_core::Error::Search(format!("Google request failed: {e}")))?;

        let body: GoogleResponse = resp
            .json()
            .await
            .map_err(|e| zhiyuan_core::Error::Search(format!("Google parse failed: {e}")))?;

        Ok(body
            .items
            .unwrap_or_default()
            .into_iter()
            .map(|v| SearchResult {
                title: v.title,
                url: v.link,
                snippet: v.snippet,
                source: "google".into(),
                fetch_time: Utc::now(),
            })
            .collect())
    }
}

#[derive(Deserialize)]
struct GoogleResponse {
    items: Option<Vec<GoogleResult>>,
}

#[derive(Deserialize)]
struct GoogleResult {
    title: String,
    link: String,
    snippet: String,
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

        if !config.bing_api_key.is_empty() {
            engines.push(Box::new(BingEngine::new(
                config.bing_api_key.clone(),
                config.bing_endpoint.clone(),
            )));
        }
        if !config.google_api_key.is_empty() && !config.google_cse_id.is_empty() {
            engines.push(Box::new(GoogleEngine::new(
                config.google_api_key.clone(),
                config.google_cse_id.clone(),
            )));
        }
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

        let futures: Vec<_> = self.engines.iter().map(|e| e.search(query)).collect();
        let results = join_all(futures).await;

        let mut seen: std::collections::HashMap<String, SearchResult> = std::collections::HashMap::new();
        let mut engine_count = 0;

        for (i, result) in results.iter().enumerate() {
            match result {
                Ok(results) if !results.is_empty() => {
                    engine_count += 1;
                    tracing::info!(
                        engine = %self.engines[i].name(),
                        count = %results.len(),
                        "cross-search contributed"
                    );
                    for r in results {
                        let key = r.url.clone();
                        if !seen.contains_key(&key) {
                            seen.insert(key, r.clone());
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

        tracing::info!(
            engine_count,
            total_results = seen.len(),
            "cross-search completed"
        );

        if seen.is_empty() {
            return Err(zhiyuan_core::Error::Search("all engines returned no results".into()));
        }

        Ok(seen.into_values().collect())
    }
}
