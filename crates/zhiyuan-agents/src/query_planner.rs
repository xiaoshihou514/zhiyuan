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
    ) -> Result<(String, Vec<String>)> {
        let system = "\
你是一个搜索查询规划专家。你的任务是根据研究子任务，生成最有效的搜索查询。

要求：
- 每个查询只包含 2-3 个高热度词，不要完整句子，若要指定时间，请指定具体年份
- 每个查询从不同角度覆盖子任务
- 根据主题自然选择查询语言，可多语言覆盖不同地区来源（包括中英以外的语言，如法语、日语、德语等）
- 同时为本次搜索选择最合适的 SearXNG 搜索引擎类别，支持：science（学术论文/预印本）、general（通用网页）、news（新闻）、it（技术/Q&A/代码仓库），可组合使用逗号分隔，如 \"science,general\"

只输出纯 JSON，不要 markdown 格式、不要代码块、不要其他文字。";

        let user = format!(
            "研究子任务：{task_description}
已有上下文：{context}
请生成 2-4 个搜索查询，每个查询从不同角度覆盖该子任务。
输出 JSON 格式：{{\"categories\": \"science,general\", \"queries\": [\"query1\", \"query2\"]}}"
        );

        let response = self.llm.prompt(system, &user).await?;
        tracing::debug!(response_len = %response.len(), "查询规划器响应");
        let cleaned = extract_json(&response);
        let parsed: serde_json::Value = serde_json::from_str(cleaned).map_err(|e| {
            zhiyuan_core::Error::Agent(format!(
                "解析查询规划输出失败: {e}\n原始响应(前200字符): {}",
                response.chars().take(200).collect::<String>()
            ))
        })?;

        let queries: Vec<String> = parsed["queries"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // 去重 + 截断到 5 个词
        let mut seen = std::collections::HashSet::new();
        let queries: Vec<String> = queries
            .into_iter()
            .filter(|q| seen.insert(q.to_lowercase()))
            .map(|q| q.split_whitespace().take(5).collect::<Vec<_>>().join(" "))
            .collect();

        // 提取类别
        let categories = parsed["categories"]
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "general".to_string());

        tracing::info!("类别" = %categories, "数量" = %queries.len(), ?queries, "已生成搜索查询");

        Ok((categories, queries))
    }
}
