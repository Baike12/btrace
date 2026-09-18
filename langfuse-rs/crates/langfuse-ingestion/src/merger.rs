use std::collections::HashSet;

/// 不可变字段 — merge 时保护这些 key，不被后续事件覆盖
const IMMUTABLE_KEYS: &[&str] = &["id", "project_id", "timestamp", "created_at"];

/// camelCase → snake_case 键名标准化
///
/// SDK payloads are camelCase throughout; the typed records are snake_case, so
/// anything missing from this table is silently dropped on ingest. `userId` and
/// `sessionId` were the visible gaps — a trace arrived with no user or session
/// attribution.
///
/// `modelParameters` is deliberately absent: it is camelCase *in the database
/// too*, and a camel→camel entry would be a no-op. `ObservationRecord` carries a
/// serde alias for it instead.
const KEY_NORMALIZE: &[(&str, &str)] = &[
    ("traceId", "trace_id"),
    ("observationId", "observation_id"),
    ("parentObservationId", "parent_observation_id"),
    ("startTime", "start_time"),
    ("endTime", "end_time"),
    ("completionStartTime", "completion_start_time"),
    ("statusMessage", "status_message"),
    ("userId", "user_id"),
    ("sessionId", "session_id"),
    ("externalId", "external_id"),
    ("promptId", "prompt_id"),
    ("promptTokens", "prompt_tokens"),
    ("completionTokens", "completion_tokens"),
    ("totalTokens", "total_tokens"),
    ("inputCost", "input_cost"),
    ("outputCost", "output_cost"),
    ("totalCost", "total_cost"),
    ("usageDetails", "usage_details"),
    ("costDetails", "cost_details"),
];

/// 标准化键名：将 camelCase 键转为 snake_case，避免下游代码同时处理两种格式
fn normalize_keys(record: &mut serde_json::Map<String, serde_json::Value>) {
    for &(camel, snake) in KEY_NORMALIZE {
        if let Some(value) = record.remove(camel) {
            if !record.contains_key(snake) {
                record.insert(snake.to_string(), value);
            }
        }
    }
}

/// 合并 trace 事件列表为最终记录
///
/// 对应原 IngestionService 中的 mergeTraceRecords
/// - 事件按 timestamp 升序叠加
/// - 不可变字段 (id, project_id, timestamp, created_at, environment) 受保护
/// - input/output 由最后一个拥有该字段的事件决定
pub fn merge_trace_records(
    events: &[serde_json::Value],
    _existing: Option<&serde_json::Value>, // PG-only: always None
) -> serde_json::Value {
    let immutable: HashSet<&str> = IMMUTABLE_KEYS.iter().copied().collect();
    let mut record = serde_json::Map::new();

    // 按 timestamp 排序的事件列表
    let mut sorted: Vec<&serde_json::Value> = events.iter().collect();
    sorted.sort_by_key(|e| {
        e.get("timestamp")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default()
    });

    for event in &sorted {
        if let Some(body) = event.get("body").or(Some(event)) {
            if let Some(obj) = body.as_object() {
                for (key, value) in obj {
                    if key == "input" || key == "output" {
                        continue; // 特殊处理
                    }
                    if !immutable.contains(key.as_str()) || !record.contains_key(key) {
                        record.insert(key.clone(), value.clone());
                    }
                }
            }
        }
    }

    // 取最后一个非空的 input/output
    for event in sorted.iter().rev() {
        let body = event.get("body").unwrap_or(event);
        if let Some(input) = body.get("input") {
            if !input.is_null() && !record.contains_key("input") {
                record.insert("input".to_string(), input.clone());
            }
        }
        if let Some(output) = body.get("output") {
            if !output.is_null() && !record.contains_key("output") {
                record.insert("output".to_string(), output.clone());
            }
        }
    }

    normalize_keys(&mut record);
    serde_json::Value::Object(record)
}

/// 合并 observation 事件列表为最终记录
pub fn merge_observation_records(
    events: &[serde_json::Value],
    _existing: Option<&serde_json::Value>,
) -> serde_json::Value {
    let immutable: HashSet<&str> = IMMUTABLE_KEYS.iter().copied()
        .chain(["trace_id", "start_time"].iter().copied())
        .collect();

    let mut record = serde_json::Map::new();

    // 按 timestamp 排序
    let mut sorted: Vec<&serde_json::Value> = events.iter().collect();
    sorted.sort_by_key(|e| {
        e.get("timestamp")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default()
    });

    for event in &sorted {
        if let Some(body) = event.get("body").or(Some(event)) {
            if let Some(obj) = body.as_object() {
                for (key, value) in obj {
                    if !immutable.contains(key.as_str()) || !record.contains_key(key) {
                        record.insert(key.clone(), value.clone());
                    }
                }
            }
        }
    }

    // input/output: 最后非空
    for event in sorted.iter().rev() {
        let body = event.get("body").unwrap_or(event);
        for &field in &["input", "output", "metadata"] {
            if let Some(v) = body.get(field) {
                if !v.is_null() && !record.contains_key(field) {
                    record.insert(field.to_string(), v.clone());
                }
            }
        }
    }

    normalize_keys(&mut record);
    serde_json::Value::Object(record)
}

/// 合并 score 事件列表
pub fn merge_score_records(
    events: &[serde_json::Value],
    _existing: Option<&serde_json::Value>,
) -> serde_json::Value {
    let immutable: HashSet<&str> = ["id", "project_id", "timestamp", "created_at"]
        .iter().copied().collect();

    let mut record = serde_json::Map::new();

    let mut sorted: Vec<&serde_json::Value> = events.iter().collect();
    sorted.sort_by_key(|e| {
        e.get("timestamp")
            .and_then(|t| t.as_str())
            .map(|s| s.to_string())
            .unwrap_or_default()
    });

    for event in &sorted {
        if let Some(body) = event.get("body").or(Some(event)) {
            if let Some(obj) = body.as_object() {
                for (key, value) in obj {
                    if !immutable.contains(key.as_str()) || !record.contains_key(key) {
                        record.insert(key.clone(), value.clone());
                    }
                }
            }
        }
    }

    normalize_keys(&mut record);
    serde_json::Value::Object(record)
}
