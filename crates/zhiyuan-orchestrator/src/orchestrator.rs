use std::sync::Arc;
use futures::future::join_all;
use tokio::sync::Semaphore;
use zhiyuan_agents::*;
use zhiyuan_core::*;
use zhiyuan_extract::WebExtractor;
use zhiyuan_memory::MemoryManager;
use zhiyuan_search::EnginePool;

use crate::QualityEvaluator;

struct IterationState {
    findings: Vec<Finding>,
    citation_graph: CitationGraph,
    report: Option<ResearchReport>,
    pending_directions: Vec<ResearchDirection>,
}

pub struct ResearchOrchestrator {
    memory: Option<MemoryManager>,
    planner: PlannerAgent,
    searcher: SearcherAgent,
    extractor_agent: ExtractorAgent,
    synthesizer: SynthesizerAgent,
    writer: WriterAgent,
    verifier: VerifierAgent,
    quality_evaluator: QualityEvaluator,
    config: ResearchSettings,
    llm: Box<dyn LlmClient>,
}

impl ResearchOrchestrator {
    pub async fn new(
        llm: Box<dyn LlmClient>,
        engine_pool: Arc<EnginePool>,
        config: ResearchSettings,
        memory_path: Option<String>,
    ) -> Self {
        let extractor = Arc::new(WebExtractor::new());
        let memory = memory_path.and_then(|p| MemoryManager::open(p).ok());

        Self {
            memory,
            planner: PlannerAgent::new(llm.clone_box()),
            searcher: SearcherAgent::new(llm.clone_box(), engine_pool),
            extractor_agent: ExtractorAgent::new(llm.clone_box(), extractor),
            synthesizer: SynthesizerAgent::new(llm.clone_box()),
            writer: WriterAgent::new(llm.clone_box()),
            verifier: VerifierAgent::new(llm.clone_box()),
            quality_evaluator: QualityEvaluator,
            config,
            llm,
        }
    }

    pub async fn research(&self, query: ResearchQuery) -> Result<ResearchReport> {
        tracing::info!(query = %query.query, "starting research");

        let plan = self.planner.create_plan(&query, &self.config).await?;
        tracing::info!(tasks = %plan.sub_tasks.len(), "plan created");
        self.save_to_memory("plan", &serde_json::to_string(&plan).unwrap_or_default());

        let existing_findings = self
            .memory
            .as_ref()
            .and_then(|m| m.semantic.find_relevant_findings(&query.query).ok())
            .unwrap_or_default();

        if !existing_findings.is_empty() {
            tracing::info!(count = %existing_findings.len(), "found relevant findings in semantic memory");
        }

        let mut state = IterationState {
            findings: existing_findings.into_iter().map(|(f, _)| Finding {
                iteration: 0,
                ..f
            }).collect(),
            citation_graph: CitationGraph {
                claims: vec![],
                sources: vec![],
                edges: vec![],
            },
            report: None,
            pending_directions: vec![],
        };

        let semaphore = Arc::new(Semaphore::new(self.config.concurrency.max(1)));

        for iteration in 1..=self.config.max_iterations {
            tracing::info!(iteration, findings = %state.findings.len(), "starting iteration");

            let tasks = self.build_iteration_tasks(&plan, &state);
            if tasks.is_empty() {
                tracing::info!("no pending tasks, ending iteration");
            }

            let new_findings = self
                .execute_tasks_concurrently(&tasks, iteration, &semaphore)
                .await;
            state.findings.extend(new_findings);

            if state.findings.is_empty() {
                tracing::warn!("no findings after iteration, stopping");
                break;
            }

            self.verify_findings(&mut state).await;

            let knowledge = KnowledgeBase {
                query_id: query.id,
                findings: state.findings.clone(),
            };
            let quality = self.quality_evaluator.evaluate(&knowledge, &query.full_query());
            tracing::info!(
                overall = %quality.overall,
                coverage = %quality.coverage,
                reliability = %quality.reliability,
                depth = %quality.depth,
                "quality score"
            );

            if quality.overall < self.config.quality_threshold {
                state.pending_directions = self
                    .synthesizer
                    .extract_directions(&query.query, &state.findings)
                    .await
                    .unwrap_or_default();
                if !state.pending_directions.is_empty() {
                    tracing::info!(new_directions = %state.pending_directions.len(), "new research directions");
                }
            }

            let report = self
                .build_or_update_report(
                    &state,
                    &quality,
                    &query,
                )
                .await;
            state.report = Some(report);

            self.save_to_memory(
                &format!("iteration:{iteration}:quality"),
                &serde_json::to_string(&quality).unwrap_or_default(),
            );

            if quality.overall >= self.config.quality_threshold {
                tracing::info!("quality threshold reached, stopping iteration");
                break;
            }
        }

        self.verify_findings(&mut state).await;

        let quality = self.quality_evaluator.evaluate(
            &KnowledgeBase {
                query_id: query.id,
                findings: state.findings.clone(),
            },
            &query.full_query(),
        );

        let report = if self.config.long_report && plan.outline.is_some() {
            self.build_long_report(&query, &state, &quality, &plan).await?
        } else {
            match state.report {
                Some(existing) => self
                    .writer
                    .update_report(&existing, &state.findings, &state.citation_graph, &quality)
                    .await
                    .unwrap_or(existing),
                None => {
                    self.writer
                        .write_report(&query.query, &state.findings, &state.citation_graph, &quality)
                        .await?
                }
            }
        };

        self.save_to_memory("report", &serde_json::to_string(&report).unwrap_or_default());

        if let Some(ref memory) = self.memory {
            for finding in &state.findings {
                let topic = plan
                    .sub_tasks
                    .iter()
                    .find(|t| finding.sub_task_id.map(|id| t.id == id).unwrap_or(false))
                    .map(|t| t.description.as_str())
                    .unwrap_or(&query.query);
                let _ = memory.semantic.store_finding(topic, finding);
            }
        }

        Ok(report)
    }

    fn build_iteration_tasks(&self, plan: &ResearchPlan, state: &IterationState) -> Vec<String> {
        let mut tasks: Vec<String> = plan
            .sub_tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Pending)
            .map(|t| t.description.clone())
            .collect();

        for dir in &state.pending_directions {
            if dir.priority >= 0.5 {
                tasks.push(dir.description.clone());
            }
        }

        tasks
    }

    async fn execute_tasks_concurrently(
        &self,
        tasks: &[String],
        iteration: usize,
        semaphore: &Semaphore,
    ) -> Vec<Finding> {
        let mut all_findings = Vec::new();
        let mut chunks: Vec<Vec<&String>> = Vec::new();
        let mut current_chunk = Vec::new();

        for task in tasks {
            current_chunk.push(task);
            if current_chunk.len() >= self.config.concurrency.max(1) {
                chunks.push(std::mem::take(&mut current_chunk));
            }
        }
        if !current_chunk.is_empty() {
            chunks.push(current_chunk);
        }

        for chunk in chunks {
            let _permit = semaphore.acquire_many(chunk.len() as u32).await.unwrap();
            let futures: Vec<_> = chunk
                .iter()
                .map(|task_desc| {
                    self.process_single_task(task_desc, iteration)
                })
                .collect();

            let results = join_all(futures).await;
            for result in results {
                match result {
                    Ok(findings) => all_findings.extend(findings),
                    Err(e) => tracing::warn!("task failed: {e}"),
                }
            }
            drop(_permit);
        }

        all_findings
    }

    async fn process_single_task(&self, task_desc: &str, iteration: usize) -> Result<Vec<Finding>> {
        let context = String::new();

        let queries = self
            .searcher
            .generate_queries(task_desc, &context, &self.config)
            .await?;
        if queries.is_empty() {
            return Ok(vec![]);
        }

        let search_results = self
            .searcher
            .execute_search(&queries, 5, self.config.concurrency, self.config.cross_validate)
            .await?;

        let extracted = self.extractor_agent.extract_content(&search_results, task_desc).await?;
        let findings = self
            .synthesizer
            .synthesize(&extracted, Uuid::new_v4(), iteration)
            .await?;

        for f in &findings {
            if let Some(ref memory) = self.memory {
                let _ = memory.episodic.store_iteration("current", iteration, f);
            }
        }

        Ok(findings)
    }

    async fn verify_findings(&self, state: &mut IterationState) {
        if state.findings.is_empty() {
            return;
        }

        let claims: Vec<Claim> = state
            .findings
            .iter()
            .map(|f| Claim {
                id: f.id,
                text: f.content.clone(),
                confidence: 0.5,
            })
            .collect();

        let sources: Vec<SourceNode> = state
            .findings
            .iter()
            .flat_map(|f| {
                f.sources.iter().map(|url| SourceNode {
                    id: Uuid::new_v4(),
                    url: url.clone(),
                    title: url.clone(),
                    reliability: 0.5,
                })
            })
            .collect();

        if !claims.is_empty() && !sources.is_empty() {
            if let Ok(verified) = self.verifier.verify_claims(&claims, &sources).await {
                state.citation_graph = verified;
                self.save_to_memory(
                    "citation_graph",
                    &serde_json::to_string(&state.citation_graph).unwrap_or_default(),
                );
            }
        }
    }

    async fn build_or_update_report(
        &self,
        state: &IterationState,
        quality: &QualityScore,
        query: &ResearchQuery,
    ) -> ResearchReport {
        match &state.report {
            Some(existing) => self
                .writer
                .update_report(existing, &state.findings, &state.citation_graph, quality)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!("progressive report update failed: {e}");
                    ResearchReport {
                        query_id: query.id,
                        title: format!("{} - 研究报告", query.query),
                        sections: vec![],
                        citation_graph: state.citation_graph.clone(),
                        quality_score: quality.clone(),
                        generated_at: chrono::Utc::now(),
                    }
                }),
            None => ResearchReport {
                query_id: query.id,
                title: format!("{} - 研究报告", query.query),
                sections: vec![],
                citation_graph: state.citation_graph.clone(),
                quality_score: quality.clone(),
                generated_at: chrono::Utc::now(),
            },
        }
    }

    async fn build_long_report(
        &self,
        query: &ResearchQuery,
        state: &IterationState,
        quality: &QualityScore,
        plan: &ResearchPlan,
    ) -> Result<ResearchReport> {
        let outline = plan.outline.as_deref().unwrap_or("");
        let chapter_titles: Vec<String> = outline
            .lines()
            .filter(|l| l.starts_with("# "))
            .map(|l| l.trim_start_matches("# ").to_string())
            .collect();

        if chapter_titles.is_empty() {
            tracing::warn!("no chapters in outline, falling back to simple report");
            return self
                .writer
                .write_report(&query.query, &state.findings, &state.citation_graph, quality)
                .await;
        }

        let chapters = self
            .assign_findings_to_chapters(&chapter_titles, outline, &state.findings)
            .await;

        let cross_check = if chapters.len() > 1 {
            self.cross_check_chapters(&chapters).await
        } else {
            String::new()
        };

        let report = self
            .writer
            .write_long_report(&query.query, outline, &chapters, &cross_check, quality)
            .await
            .map_err(|e| {
                tracing::warn!("long report generation failed: {e}");
                e
            })
            .unwrap_or_else(|_| ResearchReport {
                query_id: query.id,
                title: format!("{} - 详细研究报告", query.query),
                sections: state
                    .findings
                    .iter()
                    .map(|f| zhiyuan_core::ReportSection {
                        heading: "研究发现".into(),
                        content: f.content.clone(),
                        citations: f.sources.clone(),
                    })
                    .collect(),
                citation_graph: state.citation_graph.clone(),
                quality_score: quality.clone(),
                generated_at: chrono::Utc::now(),
            });

        Ok(report)
    }

    async fn assign_findings_to_chapters(
        &self,
        chapter_titles: &[String],
        outline: &str,
        findings: &[Finding],
    ) -> Vec<ReportChapter> {
        if findings.is_empty() || chapter_titles.is_empty() {
            return vec![];
        }

        let chapters_str = chapter_titles
            .iter()
            .enumerate()
            .map(|(i, t)| format!("{i}. {t}"))
            .collect::<Vec<_>>()
            .join("\n");

        let findings_str: String = findings
            .iter()
            .enumerate()
            .map(|(i, f)| format!("[{i}] {}\n  来源：{}", f.content, f.sources.join(", ")))
            .collect::<Vec<_>>()
            .join("\n\n");

        let system = "你是一个研究分析专家。根据研究报告大纲章节和研究发现列表，将每个发现分配到最合适的章节。";

        let user = format!(
            "大纲章节：
{chapters_str}

研究发现（每条带编号）：
{findings_str}

请为每条发现分配一个章节编号。输出 JSON 格式：{{\"assignments\": [{{\"finding_index\": 0, \"chapter_index\": 0}}]}}
如果某条发现不适合任何章节，设置 chapter_index 为 -1。"
        );

        let response = self.llm.prompt(system, &user).await.ok();
        let assignments: Vec<(usize, usize)> = response
            .and_then(|r| serde_json::from_str::<serde_json::Value>(&r).ok())
            .and_then(|v| {
                v["assignments"].as_array().map(|arr| {
                    arr.iter()
                        .filter_map(|item| {
                            let fi = item["finding_index"].as_i64()? as usize;
                            let ci = item["chapter_index"].as_i64()?;
                            if ci >= 0 { Some((fi, ci as usize)) } else { None }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .unwrap_or_default();

        let mut chapters: Vec<ReportChapter> = chapter_titles
            .iter()
            .map(|title| {
                let desc = outline
                    .lines()
                    .skip_while(|l| !l.contains(title))
                    .skip(1)
                    .take_while(|l| !l.starts_with("# "))
                    .collect::<Vec<_>>()
                    .join("\n");
                ReportChapter {
                    title: title.clone(),
                    description: desc,
                    findings: vec![],
                }
            })
            .collect();

        for (fi, ci) in &assignments {
            if *ci < chapters.len() && *fi < findings.len() {
                chapters[*ci].findings.push(findings[*fi].clone());
            }
        }

        let assigned: std::collections::HashSet<usize> =
            assignments.iter().map(|(fi, _)| *fi).collect();
        for (i, f) in findings.iter().enumerate() {
            if !assigned.contains(&i) && !chapters.is_empty() {
                let best = i % chapters.len();
                chapters[best].findings.push(f.clone());
            }
        }

        chapters
    }

    async fn cross_check_chapters(&self, chapters: &[ReportChapter]) -> String {
        let chapters_str: String = chapters
            .iter()
            .map(|ch| {
                let findings_summary: String = ch
                    .findings
                    .iter()
                    .map(|f| format!("- {}", f.content))
                    .collect::<Vec<_>>()
                    .join("\n");
                format!("# {}\n{}\n\n{}", ch.title, ch.description, findings_summary)
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        let system = "你是一个研究报告校对专家。检查以下多章节研究报告各个章节之间是否存在重复、矛盾或遗漏，给出改进建议。";

        let user = format!(
            "请检查以下各章节内容，指出：
1. 重复内容（多个章节覆盖同一主题）
2. 矛盾之处（章节间的观点不一致）
3. 遗漏（应该覆盖但未涉及的角度）
4. 改进建议

各章节：
{chapters_str}"
        );

        self.llm
            .prompt(system, &user)
            .await
            .unwrap_or_default()
    }

    fn save_to_memory(&self, key: &str, value: &str) {
        if let Some(ref memory) = self.memory {
            let _ = memory.working.set(key, value);
        }
    }
}
