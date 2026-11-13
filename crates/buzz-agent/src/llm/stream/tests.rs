use super::*;

#[test]
fn sse_framing_handles_split_unicode_crlf_comments_and_multiline_data() {
    let bytes = "\u{feff}: keepalive\r\ndata: {\r\ndata: \"text\":\"🐝\"}\r\n\r\n".as_bytes();
    for split in 0..=bytes.len() {
        let mut decoder = Decoder::default();
        let mut events = decoder.push(&bytes[..split]).unwrap();
        events.extend(decoder.push(&bytes[split..]).unwrap());
        assert_eq!(events, vec!["{\n\"text\":\"🐝\"}"]);
    }
}

#[test]
fn terminal_events_are_required_before_tools_can_be_returned() {
    let mut chat = chat::Chat::default();
    chat.event(r#"{"choices":[{"index":0,"delta":{"content":"partial"},"finish_reason":null}]}"#)
        .unwrap();
    assert!(chat.finish().is_err());
    assert!(responses::Responses::default().finish().is_err());
    assert!(anthropic::Anthropic::default().finish().is_err());
}

#[test]
fn chat_reassembles_interleaved_calls_opaque_metadata_and_usage() {
    let mut response = chat::Chat::default();
    for value in [
        serde_json::json!({"choices":[{"index":0,"delta":{"content":"hello ","reasoning_content":"checking ","tool_calls":[
            {"index":1,"id":"b","type":"function","thoughtSignature":"opaque","function":{"name":"second","arguments":"{\"n\":"}},
            {"index":0,"id":"a","type":"function","function":{"name":"first","arguments":"{"}}
        ]},"finish_reason":null}]}),
        serde_json::json!({"choices":[{"index":0,"delta":{"content":"world","reasoning_content":"now","tool_calls":[
            {"index":0,"function":{"arguments":"}"}},
            {"index":1,"function":{"arguments":"2}"}}
        ]},"finish_reason":"tool_calls"}]}),
        serde_json::json!({"choices":[],"usage":{"prompt_tokens":7,"completion_tokens":3,"total_tokens":10,"prompt_tokens_details":{"cached_tokens":5}}}),
    ] {
        response.event(&value.to_string()).unwrap();
    }
    response.event("[DONE]").unwrap();
    let parsed = super::super::parse_openai(response.finish().unwrap()).unwrap();
    assert_eq!(parsed.text, "hello world");
    assert_eq!(parsed.reasoning, "checking now");
    assert_eq!(parsed.tool_calls.len(), 2);
    assert_eq!(parsed.tool_calls[0].provider_id, "a");
    assert_eq!(parsed.tool_calls[1].arguments, serde_json::json!({"n":2}));
    assert_eq!(
        parsed.tool_calls[1].provider_extra["thoughtSignature"],
        "opaque"
    );
    assert!(!parsed.tool_calls[1].provider_extra.contains_key("index"));
    assert_eq!(parsed.cached_input_tokens, Some(5));
    assert_eq!(parsed.total_tokens, Some(10));
}

#[test]
fn anthropic_preserves_signed_interleaving_and_drops_truncated_arguments() {
    for truncated in [false, true] {
        let mut response = anthropic::Anthropic::default();
        for value in [
            serde_json::json!({"type":"message_start","message":{"content":[],"usage":{"input_tokens":7,"cache_read_input_tokens":5}}}),
            serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}),
            serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"check"}}),
            serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signed"}}),
            serde_json::json!({"type":"content_block_start","index":1,"content_block":{"type":"redacted_thinking","data":"opaque"}}),
            serde_json::json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"a","name":"run","input":{}}}),
            serde_json::json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":if truncated { "{broken" } else { "{\"n\":1}" }}}),
            serde_json::json!({"type":"message_delta","delta":{"stop_reason":if truncated {"max_tokens"} else {"tool_use"}},"usage":{"output_tokens":3}}),
            serde_json::json!({"type":"message_stop"}),
        ] {
            response.event(&value.to_string()).unwrap();
        }
        let parsed = super::super::parse_anthropic(response.finish().unwrap()).unwrap();
        assert_eq!(parsed.reasoning, "check");
        assert_eq!(parsed.input_tokens, Some(12));
        assert_eq!(parsed.output_tokens, Some(3));
        if truncated {
            assert!(parsed.tool_calls.is_empty());
            assert!(parsed.reasoning_details.is_none());
        } else {
            assert_eq!(parsed.tool_calls[0].arguments, serde_json::json!({"n":1}));
            let blocks = &parsed.reasoning_details.unwrap()["anthropic_content"];
            assert_eq!(blocks[0]["signature"], "signed");
            assert_eq!(blocks[1]["data"], "opaque");
            assert_eq!(blocks[2]["type"], "tool_use");
        }
    }
}

#[test]
fn responses_require_terminal_status_and_retain_completed_items() {
    for truncated in [false, true] {
        let mut response = responses::Responses::default();
        response.event(&serde_json::json!({"type":"response.output_item.done", "output_index":0,
            "item":{"type":"function_call","call_id":"a","name":"run","arguments":if truncated {"{broken"} else {"{}"}}
        }).to_string()).unwrap();
        response.event(&serde_json::json!({"type":if truncated {"response.incomplete"} else {"response.completed"},
            "response":{"status":if truncated {"incomplete"} else {"completed"},"incomplete_details":{"reason":"max_output_tokens"},
            "usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}
        }).to_string()).unwrap();
        let parsed = super::super::parse_responses(response.finish().unwrap()).unwrap();
        assert_eq!(parsed.tool_calls.len(), usize::from(!truncated));
        assert_eq!(parsed.total_tokens, Some(10));
    }
    let mut response = responses::Responses::default();
    assert!(response
        .event(r#"{"type":"response.completed","response":{"status":"failed"}}"#)
        .is_err());
    assert!(response
        .event(r#"{"type":"response.failed","response":{"error":{"message":"failed"}}}"#)
        .is_err());
}

#[test]
fn framing_rejects_oversized_unterminated_events_and_invalid_utf8() {
    assert!(Decoder::default().push(&[b'x'; 1]).unwrap().is_empty());
    assert!(Decoder::default().push(b"data: \xff\n").is_err());
    assert!(Decoder::default()
        .push(&vec![b'x'; MAX_LLM_RESPONSE_BYTES + 1])
        .is_err());
}

#[test]
fn malformed_native_events_fail_without_panicking_or_committing_tools() {
    for value in [
        serde_json::json!({"type":"message_start","message":false}),
        serde_json::json!({"type":"message_start","message":{"usage":true}}),
        serde_json::json!({"type":"content_block_start","index":0,"content_block":false}),
        serde_json::json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}),
    ] {
        assert!(anthropic::Anthropic::default()
            .event(&value.to_string())
            .is_err());
    }
    let mut response = anthropic::Anthropic::default();
    response
        .event(r#"{"type":"message_start","message":{"usage":{}}}"#)
        .unwrap();
    assert!(response
        .event(r#"{"type":"content_block_start","index":0,"content_block":false}"#)
        .is_err());
    response.event(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"a","name":"run","input":{}}}"#).unwrap();
    assert!(response.event(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":4}}"#).is_err());
    assert!(response
        .event(r#"{"type":"message_delta","delta":{"usage":false},"usage":{"output_tokens":1}}"#)
        .is_err());
}

#[test]
fn refusals_and_unsuccessful_terminal_statuses_never_commit_tool_calls() {
    let mut chat = chat::Chat::default();
    chat.event(r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"a","type":"function","function":{"name":"run","arguments":"{}"}}]},"finish_reason":"content_filter"}]}"#).unwrap();
    chat.event("[DONE]").unwrap();
    assert!(chat.finish().is_err());
    for response in [
        serde_json::json!({"status":"incomplete","incomplete_details":{"reason":"content_filter"},"output":[{"type":"function_call","call_id":"a","name":"run","arguments":"{}"}]}),
        serde_json::json!({"status":"completed","output":[{"type":"function_call","status":"incomplete","call_id":"a","name":"run","arguments":"{}"}]}),
    ] {
        let kind = if response["status"] == "completed" {
            "response.completed"
        } else {
            "response.incomplete"
        };
        assert!(responses::Responses::default()
            .event(&serde_json::json!({"type":kind,"response":response}).to_string())
            .is_err());
    }
}
