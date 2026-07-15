use std::sync::Arc;
use tokio::sync::Semaphore;
use zhiyuan_core::{LlmClient, Result, SearchQuery, SearchResult};
use zhiyuan_search::EnginePool;

use crate::QueryPlannerAgent;

pub struct SearcherAgent {
    engine_pool: Arc<EnginePool>,
    query_planner: QueryPlannerAgent,
}

impl SearcherAgent {
    pub fn new(llm: Box<dyn LlmClient>, engine_pool: Arc<EnginePool>) -> Self {
        Self {
            engine_pool,
            query_planner: QueryPlannerAgent::new(llm),
        }
    }

    pub async fn generate_queries(
        &self,
        task_description: &str,
        context: &str,
    ) -> Result<Vec<String>> {
        self.query_planner.plan_queries(task_description, context).await
    }

    pub async fn execute_search(
        &self,
        queries: &[String],
        max_results: usize,
        concurrency: usize,
        cross_validate: bool,
    ) -> Result<Vec<SearchResult>> {
        let semaphore = Arc::new(Semaphore::new(concurrency));
        let mut handles = Vec::new();

        for query_str in queries {
            let permit = semaphore.clone().acquire_owned().await.unwrap();
            let engine = self.engine_pool.clone();
            let q = query_str.clone();

            handles.push(tokio::spawn(async move {
                let _permit = permit;
                let sq = SearchQuery {
                    query: q.clone(),
                    max_results,
                    region: None,
                    language: None,
                };

                let result = if cross_validate {
                    engine.search_all(&sq).await
                } else {
                    engine.search(&sq).await
                };
                (q, result)
            }));
        }

        let mut all_results = Vec::new();
        for handle in handles {
            match handle.await {
                Ok((_q, Ok(results))) => all_results.extend(results),
                Ok((_q, Err(e))) => tracing::warn!("Search failed for query '{_q}': {e}"),
                Err(e) => tracing::warn!("Search task panicked: {e}"),
            }
        }
        Ok(all_results)
    }
}
