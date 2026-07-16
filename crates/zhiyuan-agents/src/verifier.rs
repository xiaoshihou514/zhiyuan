use uuid::Uuid;
use crate::util::extract_json;
use zhiyuan_core::{CitationEdge, CitationGraph, Claim, LlmClient, Result, SourceNode};

pub struct VerifierAgent {
    llm: Box<dyn LlmClient>,
}

impl VerifierAgent {
    pub fn new(llm: Box<dyn LlmClient>) -> Self {
        Self { llm }
    }

    pub async fn verify_claims(&self, claims: &[Claim], sources: &[SourceNode]) -> Result<CitationGraph> {
        let claims_str: String = claims
            .iter()
            .map(|c| format!("- 声明 ({})：{}", c.id, c.text))
            .collect::<Vec<_>>()
            .join("\n");

        let sources_str: String = sources
            .iter()
            .map(|s| format!("- 来源 ({})：[{}]({})", s.id, s.title, s.url))
            .collect::<Vec<_>>()
            .join("\n");

        let system = "你是一个事实核查专家。你的任务是交叉验证研究发现中的关键声明，\
检查是否有矛盾信息，并评估每个声明的可信度。\
只输出纯 JSON，不要 markdown 格式、不要代码块、不要其他文字。";

        let user = format!(
            "请验证以下声明与来源之间的支持或矛盾关系。

声明：
{claims_str}

来源：
{sources_str}

对于每个声明，判断它是否被各来源支持或矛盾。
输出 JSON 格式：
{{\"edges\": [{{\"claim_id\": \"...\", \"source_id\": \"...\", \"relation\": \"supports\"}}]}}"
        );

        let response = self.llm.prompt(system, &user).await?;
        let cleaned = extract_json(&response);
        let parsed: serde_json::Value = serde_json::from_str(cleaned)
            .unwrap_or_else(|e| {
                tracing::warn!("错误" = %e, "验证器 JSON 解析失败，使用空回退");
                serde_json::json!({"edges": []})
            });

        let edges: Vec<CitationEdge> = parsed["edges"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| {
                        let claim_id = v["claim_id"].as_str().and_then(|s| Uuid::parse_str(s).ok())?;
                        let source_id = v["source_id"].as_str().and_then(|s| Uuid::parse_str(s).ok())?;
                        let relation = v["relation"].as_str()?;
                        match relation {
                            "supports" => Some(CitationEdge::Supports { claim_id, source_id }),
                            "contradicts" => Some(CitationEdge::Contradicts { claim_id, source_id }),
                            _ => None,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        tracing::info!("声明" = %claims.len(), "来源" = %sources.len(), "边" = %edges.len(), "验证完成");

        Ok(CitationGraph {
            claims: claims.to_vec(),
            sources: sources.to_vec(),
            edges,
        })
    }
}
