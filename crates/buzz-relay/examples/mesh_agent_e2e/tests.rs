use super::*;
use serde_json::{json, Value};

fn request() -> Value {
    json!({"jsonrpc":"2.0", "id":3, "method":"session/request_permission",
        "params":{"sessionId":"s", "toolCall":{"title":"dev__shell",
            "rawInput":{"command":marker_command("marker.txt")}},
            "options":[{"kind":"allow_always","optionId":"persistent"},
                {"kind":"allow_once","optionId":"one-shot"}]}})
}

#[test]
fn exact_marker_grants_offered_once_option_and_preserves_id() {
    let mut approved = false;
    let response = permission_response(&request(), Some("s"), Some("marker.txt"), &mut approved);
    assert!(approved);
    assert_eq!(response["id"], 3);
    assert_eq!(
        response["result"]["outcome"],
        json!({"outcome":"selected","optionId":"one-shot"})
    );
    assert_eq!(
        permission_response(&request(), Some("s"), Some("marker.txt"), &mut approved)["result"]
            ["outcome"]["outcome"],
        "cancelled"
    );
}

#[test]
fn hardware_payload_allows_null_workdir_but_rejects_non_null_values() {
    let mut r = request();
    r["params"]["toolCall"]["rawInput"] = json!({
        "command": "printf %s BUZZ_OK > marker.txt",
        "timeout_ms": 120000,
        "workdir": null
    });
    let mut approved = false;
    let response = permission_response(&r, Some("s"), Some("marker.txt"), &mut approved);
    assert!(approved);
    assert_eq!(response["id"], 3);
    assert_eq!(
        response["result"]["outcome"],
        json!({"outcome":"selected","optionId":"one-shot"})
    );
    assert_eq!(
        permission_response(&r, Some("s"), Some("marker.txt"), &mut approved)["result"]["outcome"]
            ["outcome"],
        "cancelled"
    );
    for workdir in [
        json!(""),
        json!("/tmp"),
        json!("."),
        json!(false),
        json!(0),
        json!([]),
        json!({}),
    ] {
        r["params"]["toolCall"]["rawInput"]["workdir"] = workdir;
        let mut approved = false;
        let response = permission_response(&r, Some("s"), Some("marker.txt"), &mut approved);
        assert_eq!(response["result"]["outcome"]["outcome"], "cancelled", "{r}");
        assert!(!approved);
    }
}

#[test]
fn permission_fails_closed_on_scope_command_options_and_protocol() {
    let mut mutations = Vec::new();
    for command in [
        "printf %s BUZZ_OK > other.txt",
        "printf %s BUZZ_OK > marker.txt; pwd",
        "printf %s BUZZ_OK > ../marker.txt",
        "echo BUZZ_OK > marker.txt",
    ] {
        let mut r = request();
        r["params"]["toolCall"]["rawInput"]["command"] = json!(command);
        mutations.push(r);
    }
    for (pointer, value) in [
        ("/params/sessionId", json!("other")),
        ("/params/toolCall/title", json!("other__shell")),
        (
            "/params/toolCall/rawInput",
            json!({"command":marker_command("marker.txt"),"workdir":"/tmp"}),
        ),
        (
            "/params/toolCall/rawInput",
            json!({"command":marker_command("marker.txt"),"timeout_ms":999999}),
        ),
        (
            "/params/options",
            json!([{"kind":"allow_always","optionId":"always"}]),
        ),
        ("/params/toolCall", Value::Null),
    ] {
        let mut r = request();
        *r.pointer_mut(pointer).unwrap() = value;
        mutations.push(r);
    }
    for r in mutations {
        let mut approved = false;
        let response = permission_response(&r, Some("s"), Some("marker.txt"), &mut approved);
        assert_eq!(response["result"]["outcome"]["outcome"], "cancelled", "{r}");
        assert!(!approved);
    }
    for (session, marker) in [
        (None, Some("marker.txt")),
        (Some("s"), None),
        (Some("s"), Some("../marker.txt")),
    ] {
        assert_eq!(
            permission_response(&request(), session, marker, &mut false)["result"]["outcome"]
                ["outcome"],
            "cancelled"
        );
    }
}

#[test]
fn collector_excludes_thoughts_tool_results_and_nontext_content() {
    let mut text = String::new();
    for update in [
        "agent_thought_chunk",
        "tool_call",
        "tool_call_update",
        "user_message_chunk",
    ] {
        collect_text(
            &json!({"sessionUpdate":update,"content":{"type":"text","text":"PONG"}}),
            &mut text,
        );
    }
    collect_text(
        &json!({"sessionUpdate":"agent_message_chunk","content":{"type":"image","text":"PONG"}}),
        &mut text,
    );
    assert!(text.is_empty());
    for chunk in ["PO", "NG"] {
        collect_text(
            &json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":chunk}}),
            &mut text,
        );
    }
    assert_eq!(text, "PONG");
}

/// Real stdio framing through drive_acp, no model, MCP or native runtime.
/// The permission request intentionally collides with our prompt response id.
#[tokio::test]
async fn acp_permission_request_is_not_mistaken_for_prompt_response() {
    let script = r#"
import json, sys
def read(): return json.loads(sys.stdin.readline())
def send(v): print(json.dumps(v), flush=True)
assert read()['method'] == 'initialize'
send({'id':1,'result':{'protocolVersion':1}})
assert read()['method'] == 'session/new'
send({'id':2,'result':{'sessionId':'s'}})
assert read()['method'] == 'session/prompt'
send({'id':3,'method':'session/request_permission','params':{'sessionId':'s','toolCall':{'title':'dev__shell','rawInput':{'command':'printf %s BUZZ_OK > marker.txt','timeout_ms':120000,'workdir':None}},'options':[{'kind':'allow_once','optionId':'yes'}]}})
r = read()
assert r['id'] == 3 and r['result']['outcome'] == {'outcome':'selected','optionId':'yes'}
for kind,text in [('agent_thought_chunk','NOT VISIBLE'),('agent_message_chunk','PONG')]:
    send({'method':'session/update','params':{'sessionId':'s','update':{'sessionUpdate':kind,'content':{'type':'text','text':text}}}})
send({'id':3,'result':{'stopReason':'end_turn'}})
"#;
    let mut child = Command::new("python3")
        .args(["-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let home = tempfile::tempdir().unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        drive_acp(&mut child, "test", &[], home.path(), Some("marker.txt")),
    )
    .await
    .unwrap();
    assert_eq!(result.unwrap(), "PONG");
    assert!(child.wait().await.unwrap().success());
}
