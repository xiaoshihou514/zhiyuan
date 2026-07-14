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
        let response = self.build_report_content(research_question, findings, citation_graph, quality_score).await?;

        Ok(ResearchReport {
            query_id: Uuid::new_v4(),
            title: format!("{research_question} - 研究报告"),
            sections: vec![ReportSection {
                heading: "研究结果".into(),
                content: response,
                citations: findings.iter().flat_map(|f| f.sources.clone()).collect(),
            }],
            citation_graph: citation_graph.clone(),
            quality_score: quality_score.clone(),
            generated_at: Utc::now(),
        })
    }

    pub async fn update_report(
        &self,
        existing_report: &ResearchReport,
        new_findings: &[Finding],
        citation_graph: &CitationGraph,
        quality_score: &QualityScore,
    ) -> Result<ResearchReport> {
        let existing_content = existing_report
            .sections
            .iter()
            .map(|s| format!("# {}\n{}", s.heading, s.content))
            .collect::<Vec<_>>()
            .join("\n\n");

        let new_findings_str: String = new_findings
            .iter()
            .map(|f| {
                let sources = f.sources.join(", ");
                format!("- {}\n  来源：{sources}", f.content)
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let system = "你是一个研究报告写作专家。你的任务是基于已有的报告草稿和新的研究发现，
更新和优化研究报告。保持结构一致性，将新信息融合到适当章节中。
报告使用 Markdown 格式，包含内联引用。";

        let user = format!(
            "研究问题：{question}

已有报告草稿：
{existing_content}

新的研究发现：
{new_findings_str}

请将新发现整合到已有报告中，更新相关章节。
如果新发现引入了新主题，添加新的章节。
保持报告的整体连贯性和深度。",
            question = existing_report.title.replace(" - 研究报告", "")
        );

        let response = self.llm.prompt(system, &user).await?;

        let mut all_citations: Vec<String> = existing_report
            .sections
            .iter()
            .flat_map(|s| s.citations.clone())
            .chain(new_findings.iter().flat_map(|f| f.sources.clone()))
            .collect();
        all_citations.sort();
        all_citations.dedup();

        Ok(ResearchReport {
            query_id: existing_report.query_id,
            title: existing_report.title.clone(),
            sections: vec![ReportSection {
                heading: "研究结果".into(),
                content: response,
                citations: all_citations,
            }],
            citation_graph: citation_graph.clone(),
            quality_score: quality_score.clone(),
            generated_at: Utc::now(),
        })
    }

    async fn build_report_content(
        &self,
        research_question: &str,
        findings: &[Finding],
        _citation_graph: &CitationGraph,
        quality_score: &QualityScore,
    ) -> Result<String> {
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

        self.llm.prompt(system, &user).await
    }
}
