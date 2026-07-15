/// 从 LLM 响应中提取 JSON 字符串，自动剥离 markdown 代码块围栏。
pub fn extract_json(response: &str) -> &str {
    let trimmed = response.trim();
    if let Some(inner) = trimmed.strip_prefix("```json") {
        inner.strip_suffix("```").unwrap_or(inner)
    } else if let Some(inner) = trimmed.strip_prefix("```") {
        inner.strip_suffix("```").unwrap_or(inner)
    } else {
        trimmed
    }
    .trim()
}
