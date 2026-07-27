use super::cloud::parse_litellm_sse_body;
use super::lmstudio::AdviserExecutionErrorCode;

fn event(content: &str) -> String {
    format!(
        "data: {}\n\n",
        serde_json::json!({
            "choices": [{
                "delta": {
                    "content": content
                }
            }]
        })
    )
}

#[test]
fn reconstructs_strict_json_from_litellm_sse_content_deltas() {
    let body = format!(
        "{}{}data: [DONE]\n\n",
        event("{\"status\":"),
        event("\"ready\"}")
    );

    assert_eq!(
        parse_litellm_sse_body(body.as_bytes()).expect("valid SSE response"),
        "{\"status\":\"ready\"}"
    );
}

#[test]
fn accepts_metadata_events_without_content_but_requires_content_and_done() {
    let metadata = "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\n";
    let valid = format!("{metadata}{}data: [DONE]\n\n", event("{}"));
    assert_eq!(
        parse_litellm_sse_body(valid.as_bytes()).expect("metadata may precede content"),
        "{}"
    );

    for invalid in [
        "data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\ndata: [DONE]\n\n",
        "data: [DONE]\n\n",
        "",
    ] {
        let error =
            parse_litellm_sse_body(invalid.as_bytes()).expect_err("empty content must be rejected");
        assert_eq!(error.code(), AdviserExecutionErrorCode::InvalidOutput);
    }
}

#[test]
fn rejects_malformed_or_incomplete_litellm_sse() {
    let cases = [
        "event: message\ndata: {}\n\ndata: [DONE]\n\n",
        "data: not-json\n\ndata: [DONE]\n\n",
        "data: {\"choices\":[]}\n",
        "data: [DONE]\n\ndata: {\"choices\":[]}\n\n",
        "data:{\"choices\":[]}\n\ndata: [DONE]\n\n",
    ];

    for body in cases {
        let error =
            parse_litellm_sse_body(body.as_bytes()).expect_err("malformed SSE must be rejected");
        assert_eq!(error.code(), AdviserExecutionErrorCode::InvalidOutput);
    }
}

#[test]
fn rejects_oversized_litellm_sse() {
    let body = vec![b'x'; 4 * 1024 * 1024 + 1];
    let error = parse_litellm_sse_body(&body).expect_err("oversized SSE must be rejected");
    assert_eq!(error.code(), AdviserExecutionErrorCode::InvalidOutput);
}
