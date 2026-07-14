use uuid::Uuid;
use zhiyuan_core::{ExtractedContent, Finding, LlmClient, ResearchDirection, Result};

pub struct SynthesizerAgent {
    llm: Box<dyn LlmClient>,
}

impl SynthesizerAgent {
    pub fn new(llm: Box<dyn LlmClient>) -> Self {
        Self { llm }
    }

    pub async fn synthesize(
        &self,
        contents: &[ExtractedContent],
        sub_task_id: Uuid,
        iteration: usize,
    ) -> Result<Vec<Finding>> {
        let contents_str: String = contents
            .iter()
            .map(|c| format!("## [{}]({})\n{}", c.title, c.url, c.text.chars().take(2000).collect::<String>()))
            .collect::<Vec<_>>()
            .join("\n\n");

        let system = "你是一个信息综合专家。你的任务是将多个信息源的提取内容进行综合，
生成简洁而有深度的研究发现摘要。你需要识别关键信息、发现信息间的关联和矛盾。
输出 JSON 格式的发现。";

        let user = format!(
            "以下是多个信息源提取的内容，请综合这些信息，生成 2-4 个研究发现摘要。
每个发现应该包含核心观点和引用来源。
输出 JSON 格式：{{\"findings\": [{{\"content\": \"...\", \"sources\": [\"url1\"]}}]}}

内容：
{contents_str}"
        );

        let response = self.llm.prompt(system, &user).await?;
        let parsed: serde_json::Value = serde_json::from_str(&response)
            .unwrap_or(serde_json::json!({"findings": []}));

        let findings: Vec<Finding> = parsed["findings"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|v| Finding {
                        id: Uuid::new_v4(),
                        content: v["content"].as_str().unwrap_or("").to_string(),
                        sources: v["sources"]
                            .as_array()
                            .map(|s| s.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                            .unwrap_or_default(),
                        sub_task_id: Some(sub_task_id),
                        iteration,
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(findings)
    }

    pub async fn extract_directions(
        &self,
        research_question: &str,
        current_findings: &[Finding],
    ) -> Result<Vec<ResearchDirection>> {
        let findings_str: String = current_findings
            .iter()
            .map(|f| format!("- {} (来源: {})", f.content, f.sources.join(", ")))
            .collect::<Vec<_>>()
            .join("\n");

        let system = "你是一个研究方向识别专家。你的任务是分析当前已知信息，
找出知识盲区和值得进一步深入探索的方向。
输出 JSON 格式的方向列表。";

        let user = format!(
            "研究问题：{research_question}

当前已有发现：
{findings_str}

请分析以上发现，找出 1-3 个需要进一步探索的研究方向。
每个方向应包含：描述（direction）、理由（rationale）、优先级（priority, 0-1）。
输出 JSON 格式：
{{\"directions\": [{{\"description\": \"...\", \"rationale\": \"...\", \"priority\": 0.8}}]}}"
        );

        let response = self.llm.prompt(system, &user).await?;
        let parsed: serde_json::Value = serde_json::from_str(&response)
            .unwrap_or(serde_json::json!({"directions": []}));

        let directions: Vec<ResearchDirection> = parsed["directions"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|v| ResearchDirection {
                        description: v["description"].as_str().unwrap_or("").to_string(),
                        rationale: v["rationale"].as_str().unwrap_or("").to_string(),
                        priority: v["priority"].as_f64().unwrap_or(0.5),
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(directions)
    }
}
