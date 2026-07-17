use std::collections::HashSet;
use std::sync::{Arc, Mutex};
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
    progress: Option<Box<dyn ProgressReporter>>,
}

impl ResearchOrchestrator {
    pub async fn new(
        llm: Box<dyn LlmClient>,
        engine_pool: Arc<EnginePool>,
        config: ResearchSettings,
        memory_path: Option<String>,
        progress: Option<Box<dyn ProgressReporter>>,
        blocked_domains: Vec<String>,
    ) -> Self {
        let extractor = Arc::new(WebExtractor::new());
        let memory = memory_path.and_then(|p| MemoryManager::open(p).ok());

        Self {
            memory,
            planner: PlannerAgent::new(llm.clone_box()),
            searcher: SearcherAgent::new(llm.clone_box(), engine_pool),
            extractor_agent: ExtractorAgent::new(extractor, blocked_domains),
            synthesizer: SynthesizerAgent::new(llm.clone_box()),
            writer: WriterAgent::new(llm.clone_box()),
            verifier: VerifierAgent::new(llm.clone_box()),
            quality_evaluator: QualityEvaluator,
            config,
            llm,
            progress,
        }
    }

    fn report(&self, update: ProgressUpdate) {
        if let Some(ref p) = self.progress {
            p.report(update);
        }
    }

    pub async fn research(
        &self,
        query: ResearchQuery,
        pregenerated_plan: Option<ResearchPlan>,
    ) -> Result<ResearchReport> {
        tracing::info!("查询" = %query.query, "开始研究");

        let plan = match pregenerated_plan {
            Some(p) => p,
            None => match self.planner.create_plan(&query, &self.config).await {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("计划生成失败: {e}，降级为单任务计划");
                    ResearchPlan {
                        query_id: query.id,
                        sub_tasks: vec![SubTask {
                            id: Uuid::new_v4(),
                            description: query.full_query(),
                            status: TaskStatus::Pending,
                            dependencies: vec![],
                        }],
                        outline: None,
                    }
                }
            },
        };
        tracing::info!("任务" = %plan.sub_tasks.len(), "研究计划已生成");
        self.save_to_memory("plan", &serde_json::to_string(&plan).unwrap_or_default());
        self.report(ProgressUpdate::Started {
            max_iterations: self.config.max_iterations,
            total_tasks: plan.sub_tasks.len(),
        });

        let existing_findings = self
            .memory
            .as_ref()
            .and_then(|m| m.semantic.find_relevant_findings(&query.query).ok())
            .unwrap_or_default();

        if !existing_findings.is_empty() {
            tracing::info!("数量" = %existing_findings.len(), "在语义记忆中找到了相关发现");
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
        let seen_urls = Arc::new(Mutex::new(HashSet::new()));

        let mut empty_rounds = 0;
        let mut prev_quality = 0.0f64;
        let mut quality_stagnant_rounds = 0u32;

        for iteration in 1..=self.config.max_iterations {
            tracing::info!("轮次" = iteration, "发现数" = %state.findings.len(), "开始新一轮迭代");
            self.report(ProgressUpdate::Phase {
                name: "研究".into(),
                message: format!("第 {} 轮迭代", iteration),
            });

            let tasks = self.build_iteration_tasks(&plan, &state);
            if tasks.is_empty() {
                tracing::info!("没有待处理任务，结束迭代");
            }

            let new_findings = self
                .execute_tasks_concurrently(&tasks, iteration, &semaphore, &seen_urls, &state.findings)
                .await;

            // 合并相似发现
            let novel_count = self.merge_findings(&mut state.findings, &new_findings);

            if state.findings.is_empty() {
                tracing::warn!("迭代后无发现，停止");
                break;
            }

            if novel_count < 1 {
                empty_rounds += 1;
            } else {
                empty_rounds = 0;
            }
            if empty_rounds >= 2 {
                tracing::warn!("连续 2 轮无显著新发现，提前终止研究");
                break;
            }

            self.verify_findings(&mut state).await;

            let knowledge = KnowledgeBase {
                query_id: query.id,
                findings: state.findings.clone(),
            };
            let quality = self
                .quality_evaluator
                .evaluate(&knowledge, &query.full_query(), &plan, &state.citation_graph);
            tracing::info!(
                "总分" = %quality.overall,
                "覆盖" = %quality.coverage,
                "可靠" = %quality.reliability,
                "深度" = %quality.depth,
                "质量评分"
            );

            self.report(ProgressUpdate::Iteration {
                iteration,
                max_iterations: self.config.max_iterations,
                quality: Some(quality.clone()),
                findings_count: state.findings.len(),
                sources_count: state.citation_graph.sources.len(),
            });

            // 质量停滞检测
            if iteration > 1 {
                let delta = quality.overall - prev_quality;
                if delta < 0.01 {
                    quality_stagnant_rounds += 1;
                } else {
                    quality_stagnant_rounds = 0;
                }
            }
            prev_quality = quality.overall;
            if quality_stagnant_rounds >= 2 {
                tracing::warn!("连续 2 轮质量无提升，提前终止研究");
                break;
            }

            if quality.overall < self.config.quality_threshold {
                state.pending_directions = self
                    .synthesizer
                    .extract_directions(&query.query, &state.findings, Some(&plan.sub_tasks))
                    .await
                    .unwrap_or_default();
                if !state.pending_directions.is_empty() {
                    tracing::info!("新方向" = %state.pending_directions.len(), "发现新的研究方向");
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
                tracing::info!("质量阈值已达，停止迭代");
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
            &plan,
            &state.citation_graph,
        );
        tracing::info!(
            "总分" = %quality.overall,
            "覆盖" = %quality.coverage,
            "可靠" = %quality.reliability,
            "深度" = %quality.depth,
            "最终质量评分"
        );

        self.report(ProgressUpdate::Phase {
            name: "报告".into(),
            message: "正在生成研究报告".into(),
        });

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

        self.report(ProgressUpdate::Report(report.clone()));

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
        seen_urls: &Mutex<HashSet<String>>,
        existing_findings: &[Finding],
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
                    self.process_single_task(task_desc, iteration, seen_urls, existing_findings)
                })
                .collect();

            let results = join_all(futures).await;
            for result in results {
                match result {
                    Ok(findings) => all_findings.extend(findings),
                    Err(e) => tracing::warn!("任务失败: {e}"),
                }
            }
            drop(_permit);
        }

        all_findings
    }

    async fn process_single_task(
        &self,
        task_desc: &str,
        iteration: usize,
        seen_urls: &Mutex<HashSet<String>>,
        existing_findings: &[Finding],
    ) -> Result<Vec<Finding>> {
        let context = String::new();

        let queries = self
            .searcher
            .generate_queries(task_desc, &context)
            .await?;
        if queries.is_empty() {
            return Ok(vec![]);
        }

        let search_results = self
            .searcher
            .execute_search(&queries, 5, self.config.concurrency, self.config.cross_validate)
            .await?;

        let search_results = {
            let mut seen = seen_urls.lock().unwrap();
            search_results
                .into_iter()
                .filter(|r| seen.insert(r.url.clone()))
                .collect::<Vec<_>>()
        };
        if search_results.is_empty() {
            return Ok(vec![]);
        }

        let extracted = self.extractor_agent.extract_content(&search_results, task_desc).await?;
        let findings = self
            .synthesizer
            .synthesize(&extracted, Uuid::new_v4(), iteration, existing_findings)
            .await?;

        let findings = if self.config.cross_validate {
            self.cross_validate_findings(&findings, task_desc).await?
        } else {
            findings
        };

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

    async fn cross_validate_findings(
        &self,
        findings: &[Finding],
        task_description: &str,
    ) -> Result<Vec<Finding>> {
        if findings.is_empty() {
            return Ok(findings.to_vec());
        }

        let mut verified = Vec::new();
        for finding in findings {
            let system = "\
你是一个事实核查专家。判断以下研究发现是否准确可靠。

要求：
1. 检查是否存在事实错误或矛盾
2. 判断是否有多个独立来源支持
3. 如果发现包含推测或不可靠信息，应拒绝

仅回复 TRUE 或 FALSE，不要其他内容。";

            let user = format!(
                "研究任务：{task_description}\n\n\
                 发现：{}\n\n\
                 来源数量：{}\n\
                 来源：{}\n\n\
                 该发现是否可靠？TRUE 或 FALSE",
                finding.content,
                finding.sources.len(),
                finding.sources.join("\n"),
            );

            match self.llm.prompt(system, &user).await {
                Ok(response) => {
                    let trimmed = response.trim().to_uppercase();
                    if trimmed.starts_with("TRUE") {
                        verified.push(finding.clone());
                    } else {
                        tracing::warn!(
                            finding_id = %finding.id,
                            sources = %finding.sources.len(),
                            "交叉验证未通过"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!("交叉验证 LLM 调用失败: {e}，保留该发现");
                    verified.push(finding.clone());
                }
            }
        }

        let removed = findings.len() - verified.len();
        if removed > 0 {
            tracing::info!("交叉验证: {}/{} 个发现被过滤", removed, findings.len());
        }

        Ok(verified)
    }

    /// 字符 trigram Jaccard 相似度
    fn text_similarity(a: &str, b: &str) -> f64 {
        let trigrams_a: std::collections::HashSet<String> =
            a.chars().collect::<Vec<_>>().windows(3).map(|w| w.iter().collect()).collect();
        let trigrams_b: std::collections::HashSet<String> =
            b.chars().collect::<Vec<_>>().windows(3).map(|w| w.iter().collect()).collect();
        let intersection = trigrams_a.intersection(&trigrams_b).count();
        let union = trigrams_a.union(&trigrams_b).count();
        if union == 0 { 0.0 } else { intersection as f64 / union as f64 }
    }

    /// 合并相似发现，返回本轮新增的新颖发现数
    fn merge_findings(&self, existing: &mut Vec<Finding>, new_findings: &[Finding]) -> usize {
        let mut novel = 0usize;
        for nf in new_findings {
            let is_duplicate = existing
                .iter()
                .any(|ef| Self::text_similarity(&nf.content, &ef.content) > 0.7);
            if is_duplicate {
                tracing::debug!("合并重复发现: {}", nf.content.chars().take(80).collect::<String>());
            } else {
                novel += 1;
                existing.push(nf.clone());
            }
        }
        if novel < new_findings.len() {
            tracing::info!("{} 个新发现中 {} 个与已有发现重复", new_findings.len(), new_findings.len() - novel);
        }
        novel
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
                    tracing::warn!("渐进式报告更新失败: {e}");
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
            tracing::warn!("大纲无章节，降级为简单报告");
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
                tracing::warn!("长报告生成失败: {e}");
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
