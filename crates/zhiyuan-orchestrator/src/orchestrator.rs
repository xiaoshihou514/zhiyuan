use std::sync::Arc;
use zhiyuan_agents::*;
use zhiyuan_core::*;
use zhiyuan_extract::WebExtractor;
use zhiyuan_memory::MemoryManager;
use zhiyuan_search::EnginePool;

use crate::QualityEvaluator;

pub struct ResearchOrchestrator {
    #[allow(dead_code)]
    engine_pool: Arc<EnginePool>,
    #[allow(dead_code)]
    extractor: Arc<WebExtractor>,
    memory: Option<MemoryManager>,
    planner: PlannerAgent,
    searcher: SearcherAgent,
    extractor_agent: ExtractorAgent,
    synthesizer: SynthesizerAgent,
    writer: WriterAgent,
    verifier: Option<VerifierAgent>,
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

        let planner = PlannerAgent::new(llm.clone_box());
        let searcher = SearcherAgent::new(llm.clone_box(), engine_pool.clone());
        let extractor_agent = ExtractorAgent::new(llm.clone_box(), extractor.clone());
        let synthesizer = SynthesizerAgent::new(llm.clone_box());
        let writer = WriterAgent::new(llm.clone_box());
        let verifier = Some(VerifierAgent::new(llm.clone_box()));

        Self {
            engine_pool,
            extractor,
            memory,
            planner,
            searcher,
            extractor_agent,
            synthesizer,
            writer,
            verifier,
            quality_evaluator: QualityEvaluator,
            config,
        }
    }

    pub async fn research(&self, query: ResearchQuery) -> Result<ResearchReport> {
        tracing::info!(query = %query.query, "starting research");

        let plan = self.planner.create_plan(&query).await?;
        tracing::info!(tasks = %plan.sub_tasks.len(), "plan created");

        self.save_to_memory("plan", &serde_json::to_string(&plan).unwrap_or_default());

        let mut all_findings: Vec<Finding> = Vec::new();
        let mut iteration = 0;

        loop {
            iteration += 1;
            if iteration > self.config.max_iterations {
                tracing::warn!("max iterations reached");
                break;
            }

            tracing::info!(iteration, "starting iteration");

            for task in &plan.sub_tasks {
                if task.status != TaskStatus::Pending {
                    continue;
                }

                let context = all_findings
                    .iter()
                    .map(|f| f.content.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");

                let queries = match self.searcher.generate_queries(&task.description, &context).await {
                    Ok(q) => q,
                    Err(e) => {
                        tracing::warn!("query generation failed: {e}");
                        continue;
                    }
                };

                let search_results = match self.searcher.execute_search(&queries, 5).await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("search execution failed: {e}");
                        continue;
                    }
                };

                let extracted = match self.extractor_agent.extract_content(&search_results, &task.description).await {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!("extraction failed: {e}");
                        continue;
                    }
                };

                let findings = match self.synthesizer.synthesize(&extracted, task.id, iteration).await {
                    Ok(f) => f,
                    Err(e) => {
                        tracing::warn!("synthesis failed: {e}");
                        continue;
                    }
                };

                for f in &findings {
                    if let Some(ref memory) = self.memory {
                        let _ = memory.episodic.store_iteration(&query.id.to_string(), iteration, f);
                    }
                }

                all_findings.extend(findings);
            }

            let knowledge = KnowledgeBase {
                query_id: query.id,
                findings: all_findings.clone(),
            };

            let quality = self.quality_evaluator.evaluate(&knowledge, &query.full_query());
            tracing::info!(quality = %quality.overall, "iteration quality score");

            if quality.overall >= self.config.quality_threshold {
                tracing::info!("quality threshold reached, stopping iteration");
                break;
            }

            if all_findings.is_empty() {
                tracing::warn!("no findings after iteration, stopping");
                break;
            }

            self.save_to_memory(
                &format!("iteration:{}:quality", iteration),
                &serde_json::to_string(&quality).unwrap_or_default(),
            );
        }

        let citation_graph = CitationGraph {
            claims: vec![],
            sources: vec![],
            edges: vec![],
        };

        if let Some(ref verifier) = self.verifier {
            let claims: Vec<Claim> = all_findings
                .iter()
                .map(|f| Claim {
                    id: f.id,
                    text: f.content.clone(),
                    confidence: 0.5,
                })
                .collect();

            let sources: Vec<SourceNode> = all_findings
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

            if let Ok(verified) = verifier.verify_claims(&claims, &sources).await {
                self.save_to_memory("citation_graph", &serde_json::to_string(&verified).unwrap_or_default());
            }
        }

        let quality = self.quality_evaluator.evaluate(
            &KnowledgeBase {
                query_id: query.id,
                findings: all_findings.clone(),
            },
            &query.full_query(),
        );

        let report = self
            .writer
            .write_report(&query.query, &all_findings, &citation_graph, &quality)
            .await?;

        self.save_to_memory("report", &serde_json::to_string(&report).unwrap_or_default());

        Ok(report)
    }

    fn save_to_memory(&self, key: &str, value: &str) {
        if let Some(ref memory) = self.memory {
            let _ = memory.working.set(key, value);
        }
    }
}
