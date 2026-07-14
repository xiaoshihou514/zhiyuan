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
        }
    }

    pub async fn research(&self, query: ResearchQuery) -> Result<ResearchReport> {
        tracing::info!(query = %query.query, "starting research");

        let plan = self.planner.create_plan(&query).await?;
        tracing::info!(tasks = %plan.sub_tasks.len(), "plan created");
        self.save_to_memory("plan", &serde_json::to_string(&plan).unwrap_or_default());

        let mut state = IterationState {
            findings: Vec::new(),
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

        let report = match state.report {
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
        };

        self.save_to_memory("report", &serde_json::to_string(&report).unwrap_or_default());

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

        let queries = self.searcher.generate_queries(task_desc, &context).await?;
        if queries.is_empty() {
            return Ok(vec![]);
        }

        let search_results = self
            .searcher
            .execute_search(&queries, 5, self.config.concurrency)
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

    fn save_to_memory(&self, key: &str, value: &str) {
        if let Some(ref memory) = self.memory {
            let _ = memory.working.set(key, value);
        }
    }
}
