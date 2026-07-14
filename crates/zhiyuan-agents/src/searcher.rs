use std::sync::Arc;
use zhiyuan_core::{LlmClient, Result, SearchQuery, SearchResult};
use zhiyuan_search::EnginePool;

pub struct SearcherAgent {
    llm: Box<dyn LlmClient>,
    engine_pool: Arc<EnginePool>,
}

impl SearcherAgent {
    pub fn new(llm: Box<dyn LlmClient>, engine_pool: Arc<EnginePool>) -> Self {
        Self { llm, engine_pool }
    }

    pub async fn generate_queries(&self, task_description: &str, context: &str) -> Result<Vec<String>> {
        let system = "你是一个搜索专家。你的任务是根据研究子任务，生成最有效的搜索查询。
生成的查询应该精准、具体，能够直接获取到与研究问题相关的信息。
输出 JSON 格式的搜索查询数组。";

        let user = format!(
            "研究子任务：{task_description}
已有上下文：{context}
请生成 2-4 个具体的搜索查询语句，每个查询应该从不同角度覆盖该子任务。
输出 JSON 格式：{{\"queries\": [\"query1\", \"query2\"]}}"
        );

        let response = self.llm.prompt(system, &user).await?;
        let parsed: serde_json::Value = serde_json::from_str(&response)
            .map_err(|e| zhiyuan_core::Error::Agent(format!("Failed to parse searcher output: {e}")))?;

        let queries: Vec<String> = parsed["queries"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        Ok(queries)
    }

    pub async fn execute_search(&self, queries: &[String], max_results: usize) -> Result<Vec<SearchResult>> {
        let mut all_results = Vec::new();
        for query_str in queries {
            let sq = SearchQuery {
                query: query_str.clone(),
                max_results,
                region: None,
            };
            match self.engine_pool.search(&sq).await {
                Ok(results) => all_results.extend(results),
                Err(e) => tracing::warn!("Search failed for query '{}': {e}", query_str),
            }
        }
        Ok(all_results)
    }
}
