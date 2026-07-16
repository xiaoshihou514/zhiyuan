use crate::util::extract_json;
use zhiyuan_core::{LlmClient, Result};

pub struct QueryPlannerAgent {
    llm: Box<dyn LlmClient>,
}

impl QueryPlannerAgent {
    pub fn new(llm: Box<dyn LlmClient>) -> Self {
        Self { llm }
    }

    pub async fn plan_queries(
        &self,
        task_description: &str,
        context: &str,
    ) -> Result<Vec<String>> {
        let system = "\
你是一个搜索查询规划专家。你的任务是根据研究子任务，生成最有效的搜索查询。

要求：
- 所有查询必须使用英文
- 中文概念用对应英文表达（如 国产→Chinese domestic, 工具链→toolchain）
- 保留技术术语原文（AUTOSAR、Vector、EB 等专有名词不变）
- 每个查询从不同角度覆盖子任务
- 使用关键词组合而非完整句子

只输出纯 JSON，不要 markdown 格式、不要代码块、不要其他文字。";

        let user = format!(
            "研究子任务：{task_description}
已有上下文：{context}
请生成 2-4 个英文搜索查询，每个查询从不同角度覆盖该子任务。
输出 JSON 格式：{{\"queries\": [\"query1\", \"query2\"]}}"
        );

        let response = self.llm.prompt(system, &user).await?;
        tracing::debug!(response_len = %response.len(), "查询规划器响应");
        let cleaned = extract_json(&response);
        let parsed: serde_json::Value = serde_json::from_str(cleaned)
            .map_err(|e| zhiyuan_core::Error::Agent(
                format!("解析查询规划输出失败: {e}\n原始响应(前200字符): {}", response.chars().take(200).collect::<String>())
            ))?;

        let queries: Vec<String> = parsed["queries"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // 去重（保留首次出现顺序）
        let mut seen = std::collections::HashSet::new();
        let queries: Vec<String> = queries
            .into_iter()
            .filter(|q| seen.insert(q.to_lowercase()))
            .collect();

        tracing::info!("数量" = %queries.len(), ?queries, "已生成搜索查询");

        Ok(queries)
    }
}
