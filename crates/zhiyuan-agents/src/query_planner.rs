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
你是一个搜索查询规划专家。你的任务是根据研究子任务，分析其内容特征并生成最有效的搜索查询。

分析要点：
1. 识别子任务中的技术术语、英文专有名词、框架/库/工具名称
2. 判断是否需要混合使用中英文查询以覆盖不同来源的信息
3. 如果子任务含有大量英文技术词汇，应生成部分英文查询以获取更准确的结果
4. 如果子任务完全是中文领域内容，则使用中文查询

只输出纯 JSON，不要 markdown 格式、不要代码块、不要其他文字。";

        let user = format!(
            "研究子任务：{task_description}
已有上下文：{context}
请生成 2-4 个具体的搜索查询语句，每个查询应该从不同角度覆盖该子任务。
输出 JSON 格式：{{\"queries\": [\"query1\", \"query2\"]}}"
        );

        let response = self.llm.prompt(system, &user).await?;
        let cleaned = extract_json(&response);
        let parsed: serde_json::Value = serde_json::from_str(cleaned)
            .map_err(|e| zhiyuan_core::Error::Agent(
                format!("解析查询规划输出失败: {e}\n原始响应(前200字符): {}", response.chars().take(200).collect::<String>())
            ))?;

        let queries: Vec<String> = parsed["queries"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default();

        Ok(queries)
    }
}
