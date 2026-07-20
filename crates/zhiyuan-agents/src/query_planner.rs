use crate::util::extract_json;
use zhiyuan_core::{LlmClient, Result};

use chrono;

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
你是一个搜索引擎优化专家。请根据研究子任务，生成3个不同策略的搜索查询，并标注每次搜索的 SearXNG 类别。

【策略1-精准匹配】：使用精确短语，排除噪音，必要时限定范围
【策略2-多维度拆解】：将子任务拆成2-3个递进角度，分别生成搜索词
【策略3-语言/术语增强】：根据子任务场景选择最合适的语言搜索——技术问题用英文、日本内容用日语、中文本土用中文等

要求：
- 每个查询只包含 2-3 个高热度词，不要完整句子，若要指定时间，请指定具体年份
- 每个搜索词不超过15个词
- 根据主题自然选择查询语言，可多语言覆盖不同地区来源（包括中英以外的语言，如法语、日语、德语等）
- 同时为本次搜索选择最合适的 SearXNG 搜索引擎类别，支持：science（学术论文/预印本）、general（通用网页）、news（新闻）、it（技术/Q&A/代码仓库），可组合使用逗号分隔，如 \"science,general\"

只输出纯 JSON，不要 markdown 格式、不要代码块、不要其他文字。";

        let now = chrono::Local::now();
        let date_str = now.format("%Y年%m月").to_string();
        let user = format!(
            "研究子任务：{task_description}
已有上下文：{context}
当前日期：{date_str}
请生成 3 个搜索查询，分别使用精准匹配、多维度拆解、语言增强三种策略覆盖该子任务。
输出 JSON 格式：{{\"categories\": \"science,general\", \"queries\": [\"query1\", \"query2\", \"query3\"]}}"
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
            .map(|q| q.split_whitespace().take(15).collect::<Vec<_>>().join(" "))
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
