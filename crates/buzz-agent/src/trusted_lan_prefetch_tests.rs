use serde_json::json;

use super::{
    append_evidence_to_input, command_adviser_rag_endpoint, render_rag_response,
    retrieval_unavailable_instruction,
};
use crate::lmstudio::{LmStudioInput, LmStudioInputItem};

#[test]
fn only_command_advisers_can_use_the_exact_loopback_rag_endpoint() {
    assert!(command_adviser_rag_endpoint(
        Some("builtin:command-navigation"),
        Some("http://127.0.0.1:8005/mcp/")
    )
    .is_some());
    assert!(command_adviser_rag_endpoint(
        Some("builtin:command-navigation"),
        Some("http://192.168.1.11:8005/mcp/")
    )
    .is_none());
    assert!(command_adviser_rag_endpoint(
        Some("builtin:command-navigation"),
        Some("http://127.0.0.1:8005/mcp")
    )
    .is_none());
    assert!(command_adviser_rag_endpoint(
        Some("builtin:other"),
        Some("http://127.0.0.1:8005/mcp/")
    )
    .is_none());
}

#[test]
fn legacy_rag_response_is_bounded_and_keeps_citation_metadata() {
    let inner = json!({
        "query": "ANZAC Class Frigate FFH pivot point",
        "total": 8,
        "results": [{
            "point_id": "249238f3-a746-5ac3-87c1-b213f815e4e6",
            "doc_name": "ANZAC Class Ship Handling Guide 2014.pdf",
            "collection": "navy-publications",
            "page_no": 5,
            "section_path": ["2.1 Pivot Point"],
            "score": 0.6426,
            "text": "The pivot point in an FFH making headway is the Bridge pelorus."
        }],
        "diagnostics": {
            "snapshot_id": "f88174b38ae3bca3c0339d0d0bb9dafdec2fbb2507503c1b11e830c4895b735d",
            "retrieved_at": "2026-08-18T07:43:36.543283+00:00"
        }
    });
    let outer = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "content": [{"type": "text", "text": inner.to_string()}],
            "isError": false
        }
    });

    let rendered = render_rag_response(&serde_json::to_vec(&outer).expect("fixture JSON"))
        .expect("valid RAG response");

    assert!(rendered.contains("already searched local RAG"));
    assert!(rendered.contains("ANZAC Class Ship Handling Guide 2014.pdf"));
    assert!(rendered.contains("page 5"));
    assert!(rendered.contains("2.1 Pivot Point"));
    assert!(rendered.contains("249238f3-a746-5ac3-87c1-b213f815e4e6"));
    assert!(rendered.contains("Bridge pelorus"));
    assert!(rendered.contains("f88174b38ae3bca3c0339d0d0bb9dafdec2fbb2507503c1b11e830c4895b735d"));
    assert!(rendered.contains("Do not emit or describe a tool call"));
}

#[test]
fn evidence_is_appended_to_text_and_multimodal_native_inputs() {
    let instruction = "bounded local evidence".to_string();
    assert_eq!(
        append_evidence_to_input(LmStudioInput::Text("question".into()), &instruction),
        LmStudioInput::Text("question\n\nbounded local evidence".into())
    );

    let augmented = append_evidence_to_input(
        LmStudioInput::Items(vec![LmStudioInputItem::text("question")]),
        &instruction,
    );
    assert_eq!(
        augmented,
        LmStudioInput::Items(vec![
            LmStudioInputItem::text("question"),
            LmStudioInputItem::text("bounded local evidence")
        ])
    );
}

#[test]
fn retrieval_failure_is_short_visible_and_suppresses_pseudo_tool_calls() {
    let instruction = retrieval_unavailable_instruction();
    assert!(instruction.contains("Local RAG retrieval was unavailable"));
    assert!(instruction.contains("Do not emit tool-call syntax"));
    assert!(instruction.len() < 500);
}
