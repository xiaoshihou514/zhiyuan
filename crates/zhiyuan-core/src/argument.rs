use crate::{ArgumentEdge, ArgumentNode, ArgumentNodeType, ArgumentSkeleton, Finding, ResearchPlan};
use uuid::Uuid;

/// 论证骨架构建器
///
/// 将 ResearchPlan（core_thesis + reasoning_chain）与 Findings 融合为 ArgumentSkeleton。
/// 由 Orchestrator 每轮迭代调用一次，输出供 QualityEvaluator 和 WriterAgent 使用。
pub struct ArgumentBuilder;

impl ArgumentBuilder {
    /// 从研究计划和发现构建论证骨架
    ///
    /// 策略：
    /// 1. 从 plan.core_thesis 创建结论节点（layer=0）
    /// 2. 从 plan.reasoning_chain 创建前提节点链（layer 递增）
    /// 3. 将 findings 作为证据节点附加到对应前提/结论上
    /// 4. 若 plan 无核心论点，将所有 findings 平铺为证据节点（降级模式）
    pub fn build(plan: &ResearchPlan, findings: &[Finding]) -> ArgumentSkeleton {
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        match (&plan.core_thesis, &plan.reasoning_chain) {
            (Some(thesis), Some(chain)) if !chain.is_empty() => {
                // 1. 结论节点
                let conclusion_id = Uuid::new_v4();
                nodes.push(ArgumentNode {
                    id: conclusion_id,
                    claim: thesis.clone(),
                    node_type: ArgumentNodeType::Conclusion,
                    layer: 0,
                    sources: vec![],
                });

                // 2. 前提节点链（推理链中的每一步是一个前提）
                let mut prev_id = conclusion_id;
                for (i, step) in chain.iter().enumerate() {
                    let premise_id = Uuid::new_v4();
                    nodes.push(ArgumentNode {
                        id: premise_id,
                        claim: step.clone(),
                        node_type: ArgumentNodeType::Premise,
                        layer: i + 1,
                        sources: vec![],
                    });
                    edges.push((premise_id, prev_id, ArgumentEdge::Supports));
                    prev_id = premise_id;
                }

                // 3. 将 findings 作为证据挂到最近的 premise 上
                for finding in findings {
                    let evidence_id = Uuid::new_v4();
                    nodes.push(ArgumentNode {
                        id: evidence_id,
                        claim: finding.content.clone(),
                        node_type: ArgumentNodeType::Evidence,
                        layer: chain.len() + 1,
                        sources: finding.sources.clone(),
                    });
                    // 就近挂到第一个 premise
                    if let Some(first_premise) = nodes.iter().find(|n| n.node_type == ArgumentNodeType::Premise) {
                        edges.push((evidence_id, first_premise.id, ArgumentEdge::Supports));
                    }
                }
            }
            _ => {
                // 降级模式：无核心论点时，所有 finding 平铺为证据
                for finding in findings {
                    nodes.push(ArgumentNode {
                        id: Uuid::new_v4(),
                        claim: finding.content.clone(),
                        node_type: ArgumentNodeType::Evidence,
                        layer: 0,
                        sources: finding.sources.clone(),
                    });
                }
            }
        }

        ArgumentSkeleton {
            nodes,
            edges,
            chapter_mapping: vec![],
        }
    }

    /// 增量更新骨架：加入新的 findings，返回更新后的骨架
    ///
    /// 当前实现简单追加新证据节点。后续可优化为：
    /// - 检测新 finding 是否与已有论点冲突（触发 Undermines 边）
    /// - 根据 epistemic_status 调整边的类型
    pub fn update(
        mut skeleton: ArgumentSkeleton,
        _plan: &ResearchPlan,
        new_findings: &[Finding],
    ) -> ArgumentSkeleton {
        let premise_ids: Vec<Uuid> = skeleton
            .nodes
            .iter()
            .filter(|n| n.node_type == ArgumentNodeType::Premise)
            .map(|n| n.id)
            .collect();

        for finding in new_findings {
            let evidence_id = Uuid::new_v4();
            skeleton.nodes.push(ArgumentNode {
                id: evidence_id,
                claim: finding.content.clone(),
                node_type: ArgumentNodeType::Evidence,
                layer: 0,
                sources: finding.sources.clone(),
            });
            // 挂到第一个前提节点（若有）
            if let Some(&first) = premise_ids.first() {
                skeleton.edges.push((evidence_id, first, ArgumentEdge::Supports));
            }
        }

        skeleton
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ResearchPlan;
    use uuid::Uuid;

    #[test]
    fn test_build_with_thesis() {
        let plan = ResearchPlan {
            query_id: Uuid::new_v4(),
            sub_tasks: vec![],
            outline: None,
            core_thesis: Some("Rust 异步生态已经成熟".into()),
            reasoning_chain: Some(vec![
                "tokio 是主流运行时".into(),
                "async-std 已被边缘化".into(),
            ]),
        };

        let findings = vec![Finding {
            id: Uuid::new_v4(),
            content: "tokio 在 2025 年占 85% 市场份额".into(),
            sources: vec!["https://example.com".into()],
            sub_task_id: None,
            iteration: 1,
            epistemic_status: None,
        }];

        let skeleton = ArgumentBuilder::build(&plan, &findings);
        assert!(skeleton.nodes.len() >= 4, "应包含 1 结论 + 2 前提 + 1 证据");
        assert_eq!(
            skeleton.nodes.iter().filter(|n| n.node_type == ArgumentNodeType::Conclusion).count(),
            1
        );
    }

    #[test]
    fn test_build_fallback() {
        let plan = ResearchPlan {
            query_id: Uuid::new_v4(),
            sub_tasks: vec![],
            outline: None,
            core_thesis: None,
            reasoning_chain: None,
        };
        let findings = vec![Finding {
            id: Uuid::new_v4(),
            content: "测试发现".into(),
            sources: vec![],
            sub_task_id: None,
            iteration: 1,
            epistemic_status: None,
        }];
        let skeleton = ArgumentBuilder::build(&plan, &findings);
        assert_eq!(skeleton.nodes.len(), 1);
    }
}
