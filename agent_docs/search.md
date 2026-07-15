# 搜索引擎抽象层设计

## SearchEngine trait

```rust
#[async_trait]
pub trait SearchEngine: Send + Sync {
    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError>;
    fn name(&self) -> &'static str;
}
```

## 支持引擎

| 引擎       | 方式                     | API Key | 实现              |
| ---------- | ------------------------ | ------- | ----------------- |
| Bing       | Azure Bing Search API v7 | 是      | reqwest + serde   |
| DuckDuckGo | HTML 爬取 lite 版        | 否      | reqwest + scraper |
| Google     | Custom Search JSON API   | 是      | reqwest + serde   |

## 故障切换策略

```rust
pub struct EnginePool {
    engines: Vec<Box<dyn SearchEngine>>,
    fallback_order: Vec<usize>,   // 尝试顺序
    failure_count: Vec<u32>,       // 连续失败计数
}

impl EnginePool {
    /// 按优先级依次尝试，失败自动切换到下一个
    pub async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, SearchError>;
}
```

## SearchQuery / SearchResult

```rust
pub struct SearchQuery {
    pub query: String,
    pub max_results: usize,
    pub region: Option<String>,   // 区域偏好
}

pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub source: String,           // 引擎名称
    pub fetch_time: DateTime<Utc>,
}
```
