use backoff::backoff::Backoff;
use backoff::ExponentialBackoff;
use std::future::Future;
use std::time::Duration;
use zhiyuan_core::{Error, Result};

pub async fn with_retry<F, Fut, T>(operation: F, max_retries: u32) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut backoff = ExponentialBackoff {
        max_elapsed_time: Some(Duration::from_secs(30)),
        max_interval: Duration::from_secs(5),
        ..ExponentialBackoff::default()
    };

    let mut retries = 0;

    loop {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                retries += 1;
                if retries >= max_retries {
                    return Err(e);
                }
                if let Some(delay) = backoff.next_backoff() {
                    tokio::time::sleep(delay).await;
                } else {
                    return Err(e);
                }
            }
        }
    }
}

pub async fn with_timeout<F, Fut, T>(duration: Duration, operation: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    match tokio::time::timeout(duration, operation()).await {
        Ok(result) => result,
        Err(_) => Err(Error::Robust("operation timed out".into())),
    }
}

pub fn validate_json(value: &serde_json::Value, schema: &serde_json::Value) -> Result<()> {
    if jsonschema::is_valid(schema, value) {
        Ok(())
    } else {
        Err(Error::Robust("JSON validation failed".into()))
    }
}

pub struct MessageNormalizer;

impl MessageNormalizer {
    pub fn normalize_agent_output(
        output: &str,
        expected_fields: &[&str],
    ) -> Result<serde_json::Value> {
        let parsed: serde_json::Value = serde_json::from_str(output)
            .map_err(|e| Error::Robust(format!("Failed to parse agent output as JSON: {e}")))?;

        let obj = parsed
            .as_object()
            .ok_or_else(|| Error::Robust("Agent output is not a JSON object".into()))?;

        for field in expected_fields {
            if !obj.contains_key(*field) {
                return Err(Error::Robust(format!("Missing required field: {field}")));
            }
        }

        Ok(parsed)
    }
}

pub fn default_schema_for(fields: &[&str]) -> serde_json::Value {
    let properties: serde_json::Map<String, serde_json::Value> = fields
        .iter()
        .map(|f| (f.to_string(), serde_json::json!({"type": "string"})))
        .collect();

    serde_json::json!({
        "type": "object",
        "required": fields,
        "properties": properties
    })
}
