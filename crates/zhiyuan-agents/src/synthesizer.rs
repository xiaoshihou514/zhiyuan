use crate::util::extract_json;
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
        existing_findings: &[Finding],
    ) -> Result<Vec<Finding>> {
        if !contents.is_empty() {
            tracing::info!("开始综合发现（{} 个来源）", contents.len());
        }
        let content_items: String = contents
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let text: String = c.text.chars().take(1500).collect();
                format!("[{i}] {} ({})\n{}", c.title, c.url, text)
            })
            .collect::<Vec<_>>()
            .join("\n\n---\n\n");

        let summaries: Vec<String> = if !contents.is_empty() && content_items.len() > 5000 {
            self.summarize(&content_items).await
        } else {
            Vec::new()
        };

        let contents_str = if summaries.is_empty() {
            contents
                .iter()
                .map(|c| {
                    let text: String = c.text.chars().take(500).collect();
                    format!("## [{}]({})\n{}", c.title, c.url, text)
                })
                .collect::<Vec<_>>()
                .join("\n\n")
        } else {
            contents
                .iter()
                .zip(summaries.iter())
                .map(|(c, s)| format!("## [{}]({})\n{}", c.title, c.url, s))
                .collect::<Vec<_>>()
                .join("\n\n")
        };

        let existing_str = if existing_findings.is_empty() {
            String::new()
        } else {
            let summaries: Vec<String> = existing_findings
                .iter()
                .map(|f| format!("- {}（来源：{}）", f.content, f.sources.join(", ")))
                .collect();
            format!("已有研究发现：\n{}\n\n", summaries.join("\n"))
        };

        let system = "你是一个信息综合专家。你的任务是将本轮新提取的内容与已有研究发现进行比对，\
只关注新信息、矛盾点和补充细节，避免重复已有内容。\
只输出纯 JSON，不要 markdown 格式、不要代码块、不要其他文字。";

        let user = format!(
            "{existing_str}\
本轮新提取的内容：
{contents_str}

请对照已有研究发现，只输出：
1. 新信息（已有发现未覆盖的）
2. 矛盾之处（需要修正已有发现的）
3. 补充细节（可以丰富已有发现的）

避免重复已有内容。每个发现应包含核心观点和引用来源。
输出 JSON 格式：{{\"findings\": [{{\"content\": \"...\", \"sources\": [\"url1\"]}}]}}"
        );

        let response = self.llm.prompt(system, &user).await?;
        let cleaned = extract_json(&response);
        let parsed: serde_json::Value = serde_json::from_str(cleaned).unwrap_or_else(|e| {
            tracing::warn!("错误" = %e, "综合器 JSON 解析失败，使用空回退");
            serde_json::json!({"findings": []})
        });

        let mut findings: Vec<Finding> = parsed["findings"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|v| Finding {
                        id: Uuid::new_v4(),
                        content: v["content"].as_str().unwrap_or("").to_string(),
                        sources: v["sources"]
                            .as_array()
                            .map(|s| {
                                s.iter()
                                    .filter_map(|x| x.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_default(),
                        sub_task_id: Some(sub_task_id),
                        iteration,
                        epistemic_status: None,
                    })
                    .collect()
            })
            .unwrap_or_default();

        if findings.is_empty() && !contents.is_empty() {
            for c in contents.iter().take(3) {
                let snippet: String = c.text.chars().take(500).collect();
                if !snippet.trim().is_empty() {
                    findings.push(Finding {
                        id: Uuid::new_v4(),
                        content: snippet,
                        sources: vec![c.url.clone()],
                        sub_task_id: Some(sub_task_id),
                        iteration,
                        epistemic_status: None,
                    });
                }
            }
        }

        tracing::info!("发现" = %findings.len(), "综合发现完成");

        Ok(findings)
    }

    async fn summarize(&self, content: &str) -> Vec<String> {
        let system = "你是一个研究摘要专家。请为以下每条信息提炼核心观点，\
确保每条摘要包含关键数据、结论和技术要点。按原文返回对应数量。
输出 JSON 格式：{\"summaries\": [\"摘要1\", \"摘要2\", ...]}";

        let user = format!(
            "请为以下每条信息生成简明摘要（每条 100-200 字），保留核心数据和结论：

{content}"
        );

        match self.llm.prompt(system, &user).await {
            Ok(response) => {
                let cleaned = extract_json(&response);
                serde_json::from_str::<serde_json::Value>(&cleaned)
                    .ok()
                    .and_then(|v| {
                        v["summaries"].as_array().map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                    })
                    .unwrap_or_default()
            }
            Err(e) => {
                tracing::warn!("摘要生成失败: {e}，回退到截取模式");
                Vec::new()
            }
        }
    }

    pub async fn extract_directions(
        &self,
        research_question: &str,
        current_findings: &[Finding],
        sub_tasks: Option<&[zhiyuan_core::SubTask]>,
    ) -> Result<Vec<ResearchDirection>> {
        let findings_str: String = current_findings
            .iter()
            .map(|f| format!("- {} (来源: {})", f.content, f.sources.join(", ")))
            .collect::<Vec<_>>()
            .join("\n");

        let coverage_str = if let Some(tasks) = sub_tasks {
            let mut lines = Vec::new();
            for st in tasks {
                let has_finding = current_findings.iter().any(|f| {
                    let desc_lower = st.description.to_lowercase();
                    let words: Vec<&str> = desc_lower
                        .split_whitespace()
                        .filter(|w| w.len() > 2)
                        .collect();
                    if words.is_empty() {
                        return true;
                    }
                    words.iter().any(|w| f.content.to_lowercase().contains(w))
                });
                lines.push(format!(
                    "  {}: {}",
                    st.description,
                    if has_finding {
                        "✅ 已有发现"
                    } else {
                        "❌ 未覆盖"
                    }
                ));
            }
            format!("\n子任务覆盖情况：\n{}\n", lines.join("\n"))
        } else {
            String::new()
        };

        let system = "你是一个研究方向识别专家。你的任务是分析当前已知信息和子任务覆盖情况，\
找出知识盲区和值得进一步深入探索的方向。优先关注尚未覆盖的子任务。\
只输出纯 JSON，不要 markdown 格式、不要代码块、不要其他文字。";

        let user = format!(
            "研究问题：{research_question}
{coverage_str}
当前已有发现：
{findings_str}

请分析以上信息，找出 1-3 个需要进一步探索的研究方向。\
优先关注尚未覆盖的子任务。每个方向应包含：描述（direction）、理由（rationale）、优先级（priority, 0-1）。
输出 JSON 格式：
{{\"directions\": [{{\"description\": \"...\", \"rationale\": \"...\", \"priority\": 0.8}}]}}"
        );

        let response = self.llm.prompt(system, &user).await?;
        let cleaned = extract_json(&response);
        let parsed: serde_json::Value = serde_json::from_str(cleaned).unwrap_or_else(|e| {
            tracing::warn!("错误" = %e, "方向解析 JSON 失败，使用空回退");
            serde_json::json!({"directions": []})
        });

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
