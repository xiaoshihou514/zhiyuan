/// 从 LLM 响应中提取 JSON 字符串，自动剥离 markdown 代码块围栏。
pub fn extract_json(response: &str) -> &str {
    let trimmed = response.trim();
    if let Some(start) = trimmed.find("```") {
        let after = &trimmed[start + 3..];
        let after = after.strip_prefix("json").unwrap_or(after);
        let after = after.trim_start();
        if let Some(end) = after.rfind("```") {
            after[..end].trim()
        } else {
            after.trim()
        }
    } else {
        trimmed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_json() {
        assert_eq!(extract_json(r#"{"a": 1}"#), r#"{"a": 1}"#);
    }

    #[test]
    fn test_markdown_block() {
        let input = "```json\n{\"a\": 1}\n```";
        assert_eq!(extract_json(input), "{\"a\": 1}");
    }

    #[test]
    fn test_markdown_block_no_lang() {
        let input = "```\n{\"a\": 1}\n```";
        assert_eq!(extract_json(input), "{\"a\": 1}");
    }

    #[test]
    fn test_with_surrounding_text() {
        let input = "这是结果：\n```json\n{\"queries\": [\"a\", \"b\"]}\n```\n请查收。";
        assert_eq!(extract_json(input), "{\"queries\": [\"a\", \"b\"]}");
    }
}
