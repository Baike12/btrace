use serde_json::Value;

/// 验证 ingestion 事件
///
/// 对应原 processEventBatch 中的 Zod validation
pub fn validate_event(event: &Value) -> Result<(), String> {
    let body = event.get("body").unwrap_or(event);

    // 必须字段检查
    if body.get("id").and_then(|i| i.as_str()).is_none() {
        return Err("Missing required field: body.id".to_string());
    }

    if body.get("type").and_then(|t| t.as_str()).is_none() && event.get("type").is_none() {
        return Err("Missing required field: type".to_string());
    }

    Ok(())
}

/// 读取事件类型
pub fn get_event_type(event: &Value) -> Option<String> {
    event
        .get("type")
        .or_else(|| event.get("body").and_then(|b| b.get("type")))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
}

/// 将事件归类到实体类型
/// 对应原 getClickhouseEntityType()
pub fn get_entity_type(event: &Value) -> Option<String> {
    Some(entity_type_for(&get_event_type(event)?).to_string())
}

/// Map an ingestion event type to the entity it targets.
///
/// Single source of truth for both the queue path and the synchronous HTTP
/// path; the two previously carried separate `match` blocks that disagreed on
/// which types counted as observations.
///
/// Unknown types fall through to `"observation"`, matching upstream Langfuse:
/// a newer SDK emitting an event type this build has not heard of should still
/// have its span stored rather than be rejected.
pub fn entity_type_for(event_type: &str) -> &'static str {
    match event_type {
        "trace-create" | "trace-update" => "trace",
        "score-create" | "score-update" => "score",
        "dataset-run-item-create" | "dataset-run-item-update" => "dataset_run_item",
        _ => "observation",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_every_trace_and_score_event_variant() {
        assert_eq!(entity_type_for("trace-create"), "trace");
        assert_eq!(entity_type_for("trace-update"), "trace");
        assert_eq!(entity_type_for("score-create"), "score");
        assert_eq!(entity_type_for("score-update"), "score");
        assert_eq!(entity_type_for("dataset-run-item-create"), "dataset_run_item");
    }

    #[test]
    fn maps_observation_flavoured_events_to_observation() {
        for event_type in [
            "observation-create",
            "observation-update",
            "span-create",
            "span-update",
            "generation-create",
            "generation-update",
            "event-create",
            "agent-create",
            "tool-create",
            "chain-create",
            "retriever-create",
            "evaluator-create",
            "embedding-create",
            "guardrail-create",
        ] {
            assert_eq!(
                entity_type_for(event_type),
                "observation",
                "{} should route to observations",
                event_type
            );
        }
    }

    #[test]
    fn unknown_event_types_default_to_observation() {
        assert_eq!(entity_type_for("something-new-create"), "observation");
    }

    #[test]
    fn reads_type_from_body_when_absent_at_top_level() {
        let event = json!({"body": {"type": "trace-create", "id": "t1"}});
        assert_eq!(get_entity_type(&event).as_deref(), Some("trace"));
    }
}
