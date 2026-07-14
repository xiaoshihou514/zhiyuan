use zhiyuan_core::{sub_task_from_value, LlmClient, ResearchPlan, ResearchQuery, Result};

pub struct PlannerAgent {
    llm: Box<dyn LlmClient>,
}

impl PlannerAgent {
    pub fn new(llm: Box<dyn LlmClient>) -> Self {
        Self { llm }
    }

    pub async fn create_plan(&self, query: &ResearchQuery) -> Result<ResearchPlan> {
        let system = "你是一个研究规划专家。你的任务是根据用户的研究问题，生成结构化的研究计划。
你需要将复杂问题分解为具体的子任务，每个子任务应该是一个可以独立搜索和研究的方面。
输出必须是 JSON 格式，包含 sub_tasks 数组，每个子任务有 description 和 dependencies 字段。";

        let user = format!(
            "研究问题：{}
            研究范围：请将这个问题分解为 3-6 个具体的子任务，每个子任务应该聚焦于一个独立的方面。
            输出格式：{{\"sub_tasks\": [{{\"description\": \"...\", \"dependencies\": []}}]}}",
            query.full_query()
        );

        let response = self.llm.prompt(system, &user).await?;
        let parsed: serde_json::Value = serde_json::from_str(&response)
            .map_err(|e| zhiyuan_core::Error::Agent(format!("Failed to parse planner output: {e}")))?;

        let tasks = sub_task_from_value(&parsed);

        Ok(ResearchPlan {
            query_id: query.id,
            sub_tasks: tasks,
            outline: None,
        })
    }
}
