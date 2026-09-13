//! Shared `session/request_permission` answering helper for the synchronous
//! integration harnesses.
//!
//! Every model-issued tool call is now gated on a client authorization answer
//! (`buzz-agent/src/permission.rs`). A harness that reads agent output without
//! answering the ask will simply never see the tool run, so each suite whose
//! subject is *not* the authorization boundary auto-approves while it waits.
//!
//! The boundary itself is tested in `permission_boundary.rs`, which answers
//! deliberately rather than automatically.
//!
//! Selection is by option **`kind == "allow_once"`**, never by a hardcoded
//! `optionId` — the same rule buzz-acp's answering side follows, so an option-id
//! rename cannot silently turn every approval in the suite into a denial.

#![allow(dead_code)]

use serde_json::{json, Value};

/// The JSON-RPC method the agent uses to ask.
pub const REQUEST_PERMISSION: &str = "session/request_permission";

/// True when `msg` is an inbound `session/request_permission` request.
pub fn is_permission_request(msg: &Value) -> bool {
    msg.get("method").and_then(Value::as_str) == Some(REQUEST_PERMISSION)
}

/// Build the approving response for `request`, selecting the offered
/// `allow_once` option by kind.
pub fn approve(request: &Value) -> Value {
    let option_id = request["params"]["options"]
        .as_array()
        .and_then(|opts| opts.iter().find(|o| o["kind"] == "allow_once"))
        .and_then(|o| o["optionId"].as_str())
        .expect("request must offer an allow_once option");
    json!({
        "jsonrpc": "2.0",
        "id": request["id"],
        "result": { "outcome": { "outcome": "selected", "optionId": option_id } },
    })
}
