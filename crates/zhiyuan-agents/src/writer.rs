use chrono::Utc;
use uuid::Uuid;
use zhiyuan_core::{
    CitationGraph, Finding, LlmClient, QualityScore, ReportChapter, ReportSection, ResearchReport,
    Result, SourceNode,
};

fn bib_key(url: &str) -> String {
    let url = url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let domain = url.split('/').next().unwrap_or("unknown");
    let prefix = domain
        .trim_start_matches("www.")
        .split('.')
        .next()
        .unwrap_or("x");
    let path = url.trim_start_matches(domain).trim_matches('/');
    let slug: String = path
        .split('/')
        .last()
        .unwrap_or("")
        .trim_end_matches(".pdf")
        .trim_end_matches(".html")
        .trim_end_matches(".htm")
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let slug = slug.trim_matches('-').to_lowercase();
    let slug = if slug.len() > 12 {
        slug[..12].to_string()
    } else {
        slug
    };
    if slug.is_empty() || slug.len() < 3 {
        prefix.to_string()
    } else {
        format!("{}_{}", prefix, slug)
    }
}

fn extract_title(raw: &str) -> (String, String) {
    let raw = raw.trim();
    if let Some(rest) = raw.strip_prefix("= ") {
        let end = rest.find('\n').unwrap_or(rest.len());
        let title = rest[..end].trim().to_string();
        let body = rest[end..].trim().to_string();
        (title, body)
    } else {
        ("研究报告".into(), raw.to_string())
    }
}

fn key_map_table(entries: &[(String, String)]) -> String {
    let mut table = String::from("\n\n引用 key 对照表（使用 @key 格式标注引用，例如 @example_report）：\nkey                    标题\n────────────────────────────────────────────────────\n");
    let mut seen = std::collections::HashSet::new();
    for (url, title) in entries {
        let base = bib_key(url);
        let mut key = base.clone();
        let mut counter = 1;
        while !seen.insert(key.clone()) {
            counter += 1;
            key = format!("{}_{}", base, counter);
        }
        let label = if title.is_empty() { url } else { title };
        table.push_str(&format!("{:<22} {}\n", key, label));
    }
    table
}

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
        let (title, content) = self
            .build_report_content(research_question, findings, citation_graph, quality_score)
            .await?;

        Ok(ResearchReport {
            query_id: Uuid::new_v4(),
            title,
            sections: vec![ReportSection {
                heading: "研究结果".into(),
                content,
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
            .map(|s| format!("= {}\n{}", s.heading, s.content))
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

        let system = "根据已有报告草稿和新的研究发现，更新和优化研究报告。
保持结构一致性，将新信息融合到适当章节中。
报告使用 Typst 格式，引用必须使用 @key 格式（带 @ 前缀），例如 @kpmg_report23。
第一行必须是 = 开头的一级标题（报告标题）。
不要生成参考文献/参考资料章节，系统会自动添加。
只输出纯 Typst 正文，不要 ```typst 围栏。";

        let all_urls: Vec<String> = existing_report
            .sections
            .iter()
            .flat_map(|s| s.citations.clone())
            .chain(new_findings.iter().flat_map(|f| f.sources.clone()))
            .collect();

        let entries: Vec<(String, String)> = all_urls
            .iter()
            .map(|u| {
                let title = citation_graph
                    .sources
                    .iter()
                    .find(|s| s.url == *u)
                    .map(|s| s.title.clone())
                    .unwrap_or_default();
                (u.clone(), title)
            })
            .collect();

        let user = format!(
            "研究问题：{question}

已有报告草稿：
{existing_content}

新的研究发现：
{new_findings_str}

请将新发现整合到已有报告中，更新相关章节。
如果新发现引入了新主题，添加新的章节。
保持报告的整体连贯性和深度。
引用请使用 @key 格式。{key_table}",
            question = existing_report.title,
            key_table = key_map_table(&entries)
        );

        let raw = self.llm.prompt(system, &user).await?;
        let (title, content) = extract_title(&raw);

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
            title,
            sections: vec![ReportSection {
                heading: "研究结果".into(),
                content,
                citations: all_citations,
            }],
            citation_graph: citation_graph.clone(),
            quality_score: quality_score.clone(),
            generated_at: Utc::now(),
        })
    }

    pub async fn write_long_report(
        &self,
        research_question: &str,
        outline: &str,
        chapters: &[ReportChapter],
        cross_check_review: &str,
        quality_score: &QualityScore,
    ) -> Result<ResearchReport> {
        let system = "根据多章节大纲和各个章节的研究发现，生成完整的结构化长报告。
报告使用 Typst 格式，引用必须使用 @key 格式（带 @ 前缀），例如 @kpmg_report23。
第一行必须是 = 开头的一级标题（报告标题）。
不要生成参考文献/参考资料章节，系统会自动添加。
只输出纯 Typst 正文，不要 ```typst 围栏。";

        let chapters_str: String = chapters
            .iter()
            .map(|ch| {
                let findings_str: String = ch
                    .findings
                    .iter()
                    .map(|f| {
                        let sources = f.sources.join(", ");
                        format!("- {}\n  来源：{sources}", f.content)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                format!(
                    "= {}\n{}\n\n研究发现：\n{}",
                    ch.title, ch.description, findings_str
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let all_urls: Vec<String> = chapters
            .iter()
            .flat_map(|ch| ch.findings.iter().flat_map(|f| f.sources.clone()))
            .collect();

        let entries: Vec<(String, String)> = all_urls
            .iter()
            .map(|u| (u.clone(), String::new()))
            .collect();

        let user = format!(
            "研究问题：{research_question}

大纲：
{outline}

各章节研究发现：
{chapters_str}

交叉校对意见：
{cross_check_review}

质量评分：{quality}

请生成完整的 Typst 研究报告，包含：
1. 摘要
2. 研究背景
3. 各章节正文
4. 结论与展望

确保章节之间逻辑连贯，交叉校对意见已落实。
引用请使用 @key 格式。{key_table}",
            quality = serde_json::to_string_pretty(quality_score).unwrap_or_default(),
            key_table = key_map_table(&entries)
        );

        let raw = self.llm.prompt(system, &user).await?;
        let (title, content) = extract_title(&raw);

        let mut all_citations: Vec<String> = chapters
            .iter()
            .flat_map(|ch| ch.findings.iter().flat_map(|f| f.sources.clone()))
            .collect();
        all_citations.sort();
        all_citations.dedup();

        let sources: Vec<SourceNode> = all_citations
            .iter()
            .map(|url| SourceNode {
                id: Uuid::new_v4(),
                url: url.clone(),
                title: url.clone(),
                reliability: 0.5,
            })
            .collect();

        Ok(ResearchReport {
            query_id: Uuid::new_v4(),
            title,
            sections: vec![ReportSection {
                heading: "完整报告".into(),
                content,
                citations: all_citations,
            }],
            citation_graph: CitationGraph {
                claims: vec![],
                sources,
                edges: vec![],
            },
            quality_score: quality_score.clone(),
            generated_at: chrono::Utc::now(),
        })
    }

    async fn build_report_content(
        &self,
        research_question: &str,
        findings: &[Finding],
        citation_graph: &CitationGraph,
        quality_score: &QualityScore,
    ) -> Result<(String, String)> {
        let findings_str: String = findings
            .iter()
            .map(|f| {
                let sources = f.sources.join(", ");
                format!("- {}\n  来源：{sources}", f.content)
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let entries: Vec<(String, String)> = citation_graph
            .sources
            .iter()
            .map(|s| (s.url.clone(), s.title.clone()))
            .collect();

        let system = "根据研究发现和引用信息，生成结构清晰、内容深入、有引用标注的研究报告。报告使用 Typst 格式。
引用必须使用 @key 格式（带 @ 前缀），例如 @kpmg_report23，key 对照表见下方。
第一行必须是 = 开头的一级标题（报告标题）。
不要生成参考文献/参考资料章节，系统会自动添加。
只输出纯 Typst 正文，不要 ```typst 围栏。";

        let user = format!(
            "请根据以下研究发现，撰写一份结构化的研究报告。

研究问题：{research_question}

研究发现：
{findings_str}

质量评分：{}

请生成 Typst 格式的报告，包含以下结构：
1. 摘要（概述主要发现）
2. 研究背景
3. 主要发现（分章节）
4. 结论与展望

每个章节请使用 @key 格式包含内联引用。{key_table}",
            serde_json::to_string_pretty(quality_score).unwrap_or_default(),
            key_table = key_map_table(&entries)
        );

        let raw = self.llm.prompt(system, &user).await?;
        Ok(extract_title(&raw))
    }
}
