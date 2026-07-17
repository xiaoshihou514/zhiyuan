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
        match self.query_planner.plan_queries(task_description, context).await {
            Ok(q) => Ok(q),
            Err(e) => {
                tracing::warn!("搜索查询生成失败: {e}，降级使用任务描述作为搜索词");
                Ok(vec![task_description.to_string()])
            }
        }
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

        tracing::info!("查询数" = %queries.len(), "模式" = if cross_validate { "cross-validate" } else { "fallback" }, "正在执行搜索");

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
                Ok((_q, Ok(results))) => {
                    tracing::debug!(query = %_q, count = %results.len(), "搜索返回结果");
                    all_results.extend(results);
                }
                Ok((_q, Err(e))) => tracing::warn!("搜索查询 '{_q}' 失败: {e}"),
                Err(e) => tracing::warn!("搜索任务异常: {e}"),
            }
        }
        tracing::info!("总数" = %all_results.len(), "搜索完成");
        Ok(all_results)
    }
}
