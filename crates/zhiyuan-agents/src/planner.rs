use crate::util::extract_json;
use zhiyuan_core::{
    sub_task_from_value, LlmClient, ResearchPlan, ResearchQuery, ResearchSettings, Result,
};

pub struct PlannerAgent {
    llm: Box<dyn LlmClient>,
}

impl PlannerAgent {
    pub fn new(llm: Box<dyn LlmClient>) -> Self {
        Self { llm }
    }

    pub async fn generate_clarifying_questions(&self, query: &str) -> Result<Vec<String>> {
        let system = "你是一个研究助理。用户提供了一个研究问题，但它可能不够精确或完整。\
        请生成 2-4 个澄清性问题来帮助精炼研究方向。\
        每个问题应当简短且聚焦，帮助明确：时间范围、地域、具体领域、关注重点等维度。";

        let user = format!(
            "用户的研究问题：{query}\n\n\
            请生成 2-4 个澄清性问题，每个问题一行，不要编号。"
        );

        let response = self.llm.prompt(system, &user).await?;
        Ok(response
            .lines()
            .filter(|l| !l.is_empty() && l.len() > 5)
            .map(|l| {
                l.trim_start_matches(|c: char| {
                    c.is_ascii_digit() || c == '.' || c == ')' || c == ' ' || c == '\t'
                })
                .to_string()
            })
            .collect())
    }

    pub async fn create_plan(
        &self,
        query: &ResearchQuery,
        settings: &ResearchSettings,
    ) -> Result<ResearchPlan> {
        if settings.long_report {
            self.create_long_plan(query, settings).await
        } else {
            self.create_short_plan(query).await
        }
    }

    async fn create_short_plan(&self, query: &ResearchQuery) -> Result<ResearchPlan> {
        let system = "你是一个研究规划专家。你的任务是根据用户的研究问题，生成结构化的研究计划。\
你将复杂问题分解为具体的子任务，每个子任务应该是一个可以独立搜索和研究的方面。\
同时提炼本研究的核心论点（core_thesis）和预期推理链（reasoning_chain）。\
只输出纯 JSON，不要 markdown 格式、不要代码块、不要其他文字。";

        let user = format!(
            "研究问题：{}
研究范围：请将这个问题分解为 3-6 个具体的子任务，每个子任务应该聚焦于一个独立的方面。
同时给出：
- core_thesis：本研究最核心的论点（一句话）
- reasoning_chain：论证此论点的推理链（3-5 步，每步一句话）
输出 JSON 格式：
{{\"core_thesis\": \"...\", \"reasoning_chain\": [\"...\", \"...\"], \"sub_tasks\": [{{\"description\": \"...\", \"dependencies\": []}}]}}",
            query.full_query()
        );

        let response = self.llm.prompt(system, &user).await?;
        tracing::debug!(response_len = %response.len(), "规划器短报告响应");
        let cleaned = extract_json(&response);
        let parsed: serde_json::Value = serde_json::from_str(cleaned).map_err(|e| {
            zhiyuan_core::Error::Agent(format!(
                "解析规划输出失败: {e}\n原始响应(前200字符): {}",
                response.chars().take(200).collect::<String>()
            ))
        })?;

        let tasks = sub_task_from_value(&parsed);
        let core_thesis = parsed["core_thesis"].as_str().map(|s| s.to_string());
        let reasoning_chain = parsed["reasoning_chain"].as_array().map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });

        Ok(ResearchPlan {
            query_id: query.id,
            sub_tasks: tasks,
            outline: None,
            core_thesis,
            reasoning_chain,
        })
    }

    async fn create_long_plan(
        &self,
        query: &ResearchQuery,
        _settings: &ResearchSettings,
    ) -> Result<ResearchPlan> {
        let system = "你是一个研究规划和报告结构专家。你的任务是根据用户的研究问题，生成多章节的研究计划和大纲。\
每个章节应该覆盖一个独立的子主题，所有章节合起来形成完整的研究报告。\
同时提炼本研究的核心论点（core_thesis）和预期推理链（reasoning_chain）。\
只输出纯 JSON，不要 markdown 格式、不要代码块、不要其他文字。";

        let user = format!(
            "研究问题：{}
请生成一个研究大纲，包含适量的章节（通常 3-8 个），每个章节包含 title 和 description。
同时为每个章节生成 2-3 个具体的子任务（sub_tasks）。
同时给出：
- core_thesis：本研究最核心的论点（一句话）
- reasoning_chain：论证此论点的推理链（3-5 步，每步一句话）
输出 JSON 格式：
{{\
  \"core_thesis\": \"...\",\
  \"reasoning_chain\": [\"...\", \"...\"],\
  \"outline\": [\
    {{\"title\": \"章节标题\", \"description\": \"章节描述\"}}\
  ],\
  \"sub_tasks\": [\
    {{\"description\": \"子任务描述\", \"chapter_index\": 0, \"dependencies\": []}}\
  ]\
}}
其中 chapter_index 表示该子任务属于第几个章节（从 0 开始）。",
            query.full_query(),
        );

        let response = self.llm.prompt(system, &user).await?;
        tracing::debug!(response_len = %response.len(), "规划器长报告响应");
        let cleaned = extract_json(&response);
        let parsed: serde_json::Value = serde_json::from_str(cleaned).map_err(|e| {
            zhiyuan_core::Error::Agent(format!(
                "解析长报告规划输出失败: {e}\n原始响应(前200字符): {}",
                response.chars().take(200).collect::<String>()
            ))
        })?;

        let tasks = sub_task_from_value(&parsed);
        let outline = parsed["outline"].as_array().map(|arr| {
            arr.iter()
                .map(|v| {
                    let title = v["title"].as_str().unwrap_or("").to_string();
                    let desc = v["description"].as_str().unwrap_or("").to_string();
                    format!("# {title}\n{desc}")
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        });
        let core_thesis = parsed["core_thesis"].as_str().map(|s| s.to_string());
        let reasoning_chain = parsed["reasoning_chain"].as_array().map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });

        Ok(ResearchPlan {
            query_id: query.id,
            sub_tasks: tasks,
            outline,
            core_thesis,
            reasoning_chain,
        })
    }
}
