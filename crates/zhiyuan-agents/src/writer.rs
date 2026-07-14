use chrono::Utc;
use uuid::Uuid;
use zhiyuan_core::{CitationGraph, Finding, LlmClient, QualityScore, ReportSection, ResearchReport, Result};

pub struct WriterAgent {
    llm: Box<dyn LlmClient>,
}

impl WriterAgent {
    pub fn new(llm: Box<dyn LlmClient>) -> Self {
        Self { llm }
    }

    pub async fn write_report(
        &self,
        research_question: &str,
        findings: &[Finding],
        citation_graph: &CitationGraph,
        quality_score: &QualityScore,
    ) -> Result<ResearchReport> {
        let findings_str: String = findings
            .iter()
            .map(|f| {
                let sources = f.sources.join(", ");
                format!("- {}\n  来源：{sources}", f.content)
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let system = "你是一个研究报告写作专家。你的任务是根据研究发现和引用信息，
生成结构清晰、内容深入、有引用标注的研究报告。报告应该使用 Markdown 格式。";

        let user = format!(
            "请根据以下研究发现，撰写一份结构化的研究报告。

研究问题：{research_question}

研究发现：
{findings_str}

质量评分：{}

请生成 Markdown 格式的报告，包含以下结构：
1. 摘要（概述主要发现）
2. 研究背景
3. 主要发现（分章节）
4. 结论与展望

每个章节请包含内联引用。",
            serde_json::to_string_pretty(quality_score).unwrap_or_default()
        );

        let response = self.llm.prompt(system, &user).await?;

        let sections = vec![ReportSection {
            heading: "研究结果".into(),
            content: response,
            citations: findings.iter().flat_map(|f| f.sources.clone()).collect(),
        }];

        Ok(ResearchReport {
            query_id: Uuid::new_v4(),
            title: format!("{research_question} - 研究报告"),
            sections,
            citation_graph: citation_graph.clone(),
            quality_score: quality_score.clone(),
            generated_at: Utc::now(),
        })
    }
}
