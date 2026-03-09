use anyhow::{Context, Result};
use uuid::Uuid;

use crate::types::anthropic::{MessagesResponse, ResponseContentBlock, Usage};
use crate::types::responses::{OutputContent, OutputItem, ResponsesResponse};

/// 從 Codex 後端 SSE 串流文字中解析完整回應，轉換為 Anthropic 格式
/// Parse a complete response from Codex backend SSE stream text, convert to Anthropic format
pub fn convert_responses_to_anthropic(
    sse_text: &str,
    original_model: &str,
) -> Result<MessagesResponse> {
    let response = parse_sse_to_response(sse_text)?;
    convert_parsed_response(response, original_model)
}

/// 從 SSE 事件流中找到 response.completed 事件並解析完整回應
/// Find the response.completed event in the SSE stream and parse the complete response
fn parse_sse_to_response(sse_text: &str) -> Result<ResponsesResponse> {
    // SSE 格式：每個事件以 "event: <type>\ndata: <json>\n\n" 分隔
    // SSE format: each event separated by "event: <type>\ndata: <json>\n\n"
    let mut last_response: Option<ResponsesResponse> = None;

    for block in sse_text.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        let mut event_type = "";
        let mut data_line = "";

        for line in block.lines() {
            if let Some(et) = line.strip_prefix("event: ") {
                event_type = et.trim();
            } else if let Some(d) = line.strip_prefix("data: ") {
                data_line = d.trim();
            }
        }

        // 尋找 response.completed 或 response.done 事件
        // Look for response.completed or response.done events
        if (event_type == "response.completed" || event_type == "response.done")
            && !data_line.is_empty()
        {
            // data 可能包含 {"type": "response.completed", "response": {...}} 結構
            // data may contain {"type": "response.completed", "response": {...}} structure
            let json: serde_json::Value = serde_json::from_str(data_line)
                .with_context(|| {
                    format!("無法解析 SSE 事件資料 / Failed to parse SSE event data: {}", event_type)
                })?;

            let response_json = if json.get("response").is_some() {
                &json["response"]
            } else {
                &json
            };

            let response: ResponsesResponse = serde_json::from_value(response_json.clone())
                .with_context(|| {
                    let preview: String = response_json.to_string().chars().take(500).collect();
                    format!(
                        "無法解析 Responses 回應 / Failed to parse Responses response (preview: {})",
                        preview
                    )
                })?;

            last_response = Some(response);
        }
    }

    // 如果沒有找到 completed 事件，嘗試從最後一個 response.created 重建
    // If no completed event found, try to reconstruct from last events
    if last_response.is_none() {
        last_response = try_reconstruct_from_events(sse_text);
    }

    last_response.context(
        "SSE 串流中未找到完整回應 / No complete response found in SSE stream",
    )
}

/// 嘗試從各個 SSE 事件中重建回應（當缺少 completed 事件時的後備方案）
/// Try to reconstruct a response from individual SSE events (fallback when completed is missing)
fn try_reconstruct_from_events(sse_text: &str) -> Option<ResponsesResponse> {
    let mut response_id = String::new();
    let mut model = String::new();
    let mut output_items: Vec<OutputItem> = Vec::new();
    let mut usage = None;
    let mut found_created = false;

    for block in sse_text.split("\n\n") {
        let block = block.trim();
        if block.is_empty() {
            continue;
        }

        let mut event_type = "";
        let mut data_line = "";

        for line in block.lines() {
            if let Some(et) = line.strip_prefix("event: ") {
                event_type = et.trim();
            } else if let Some(d) = line.strip_prefix("data: ") {
                data_line = d.trim();
            }
        }

        if data_line.is_empty() {
            continue;
        }

        let Ok(json) = serde_json::from_str::<serde_json::Value>(data_line) else {
            continue;
        };

        match event_type {
            "response.created" => {
                found_created = true;
                if let Some(resp) = json.get("response") {
                    response_id = resp["id"].as_str().unwrap_or("").to_string();
                    model = resp["model"].as_str().unwrap_or("").to_string();
                }
            }
            "response.output_item.done" => {
                if let Ok(item) = serde_json::from_value::<OutputItem>(json["item"].clone()) {
                    output_items.push(item);
                }
            }
            "response.usage" | "response.completed" => {
                if let Some(u) = json.get("usage") {
                    usage = serde_json::from_value(u.clone()).ok();
                }
            }
            _ => {}
        }
    }

    if !found_created {
        return None;
    }

    Some(ResponsesResponse {
        id: if response_id.is_empty() {
            format!("resp_{}", Uuid::new_v4().simple())
        } else {
            response_id
        },
        status: "completed".to_string(),
        model,
        output: output_items,
        usage,
    })
}

/// 將已解析的 Responses API 回應轉換為 Anthropic Messages 回應
/// Convert a parsed Responses API response into an Anthropic Messages response
fn convert_parsed_response(
    resp: ResponsesResponse,
    original_model: &str,
) -> Result<MessagesResponse> {
    let mut content: Vec<ResponseContentBlock> = Vec::new();

    for item in &resp.output {
        match item {
            OutputItem::Message {
                content: parts,
                ..
            } => {
                for part in parts {
                    match part {
                        OutputContent::Text { text } => {
                            content.push(ResponseContentBlock::Text { text: text.clone() });
                        }
                        OutputContent::Unknown => {}
                    }
                }
            }
            OutputItem::FunctionCall {
                name,
                arguments,
                call_id,
            } => {
                let input: serde_json::Value =
                    serde_json::from_str(arguments).unwrap_or(serde_json::Value::Object(
                        serde_json::Map::new(),
                    ));
                content.push(ResponseContentBlock::ToolUse {
                    id: call_id.clone(),
                    name: name.clone(),
                    input,
                });
            }
            OutputItem::Unknown => {}
        }
    }

    // 若內容為空，插入空文字區塊
    // If content is empty, push an empty text block
    if content.is_empty() {
        content.push(ResponseContentBlock::Text {
            text: String::new(),
        });
    }

    let stop_reason = convert_status_to_stop_reason(&resp.status, &content);

    let usage = match &resp.usage {
        Some(u) => Usage {
            input_tokens: u.input_tokens,
            output_tokens: u.output_tokens,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        },
        None => Usage {
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        },
    };

    let id = format!("msg_{}", Uuid::new_v4().simple());

    Ok(MessagesResponse {
        id,
        response_type: "message".to_string(),
        role: "assistant".to_string(),
        model: original_model.to_string(),
        content,
        stop_reason: Some(stop_reason),
        stop_sequence: None,
        usage,
    })
}

/// 將 Responses API status 映射為 Anthropic stop_reason
/// Map Responses API status to Anthropic stop_reason
fn convert_status_to_stop_reason(status: &str, content: &[ResponseContentBlock]) -> String {
    let has_tool_use = content.iter().any(|b| matches!(b, ResponseContentBlock::ToolUse { .. }));

    if has_tool_use {
        return "tool_use".to_string();
    }

    match status {
        "completed" => "end_turn".to_string(),
        "incomplete" | "truncated" => "max_tokens".to_string(),
        "cancelled" => "end_turn".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_completed_event() {
        let sse = r#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_abc","status":"completed","model":"gpt-5-codex","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello!"}]}],"usage":{"input_tokens":10,"output_tokens":5,"total_tokens":15}}}

"#;

        let result = convert_responses_to_anthropic(sse, "claude-sonnet-4-6").unwrap();

        assert_eq!(result.response_type, "message");
        assert_eq!(result.role, "assistant");
        assert_eq!(result.model, "claude-sonnet-4-6");
        assert_eq!(result.stop_reason.as_deref(), Some("end_turn"));
        assert_eq!(result.content.len(), 1);
        assert_eq!(result.usage.input_tokens, 10);
        assert_eq!(result.usage.output_tokens, 5);
    }

    #[test]
    fn test_parse_function_call() {
        let sse = r#"event: response.completed
data: {"type":"response.completed","response":{"id":"resp_xyz","status":"completed","model":"gpt-5-codex","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Let me check."}]},{"type":"function_call","name":"get_weather","arguments":"{\"location\":\"SF\"}","call_id":"call_001"}],"usage":{"input_tokens":50,"output_tokens":20,"total_tokens":70}}}

"#;

        let result = convert_responses_to_anthropic(sse, "claude-sonnet-4-6").unwrap();

        assert_eq!(result.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(result.content.len(), 2);
    }
}
