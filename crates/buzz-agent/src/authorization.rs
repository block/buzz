use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};

use crate::types::ToolCall;
use crate::wire::{self, WireSender};

const CONTRACT_VERSION: &str = "nxtlinq.authorization/v1";
pub const POLICY_DENIAL_PREFIX: &str = "Nxtlinq authorization denied this tool call";

fn action_subject(action: &Value) -> String {
    let capability = action
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    for field in ["resource", "command", "server", "tool"] {
        if let Some(value) = action.get(field).and_then(Value::as_str) {
            return format!("capability={capability} {field}={value}");
        }
    }
    format!("capability={capability}")
}

#[derive(Default)]
pub struct PermissionBroker {
    next_id: AtomicU64,
    pending: Mutex<HashMap<String, oneshot::Sender<Value>>>,
}

impl PermissionBroker {
    pub async fn resolve(&self, id: &Value, response: Value) -> bool {
        let Some(id) = id.as_str() else {
            return false;
        };
        let Some(tx) = self.pending.lock().await.remove(id) else {
            return false;
        };
        let _ = tx.send(response);
        true
    }

    pub async fn authorize(
        self: &Arc<Self>,
        wire: &WireSender,
        session_id: &str,
        call: &ToolCall,
        action: Value,
    ) -> Result<(), String> {
        let serial = self.next_id.fetch_add(1, Ordering::Relaxed);
        let id = format!("nxtlinq-permission-{serial}");
        let allow_id = format!("{id}-allow");
        let reject_id = format!("{id}-reject");
        let denied_subject = action_subject(&action);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);

        wire::send(
            wire,
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "session/request_permission",
                "params": {
                    "sessionId": session_id,
                    "toolCall": {
                        "toolCallId": call.provider_id,
                        "title": format!("Authorize {}", call.name),
                        "kind": "other",
                        "status": "pending",
                        "rawInput": call.arguments,
                        "_meta": {
                            "nxtlinq": {
                                "contractVersion": CONTRACT_VERSION,
                                "action": action
                            }
                        }
                    },
                    "options": [
                        { "optionId": allow_id, "name": "Allow once", "kind": "allow_once" },
                        { "optionId": reject_id, "name": "Reject once", "kind": "reject_once" }
                    ]
                }
            }),
        )
        .await;

        let response = match tokio::time::timeout(std::time::Duration::from_secs(30), rx).await {
            Ok(Ok(value)) => value,
            Ok(Err(_)) => return Err("authorization channel closed".into()),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                return Err("authorization timed out".into());
            }
        };

        let selected = response
            .pointer("/result/outcome")
            .and_then(Value::as_object)
            .is_some_and(|outcome| {
                outcome.get("outcome").and_then(Value::as_str) == Some("selected")
                    && outcome.get("optionId").and_then(Value::as_str) == Some(allow_id.as_str())
            });
        let gateway_allowed = response
            .pointer("/result/_meta/nxtlinq/decision")
            .and_then(Value::as_str)
            == Some("allow");
        if selected && gateway_allowed {
            tracing::info!(
                target: "nxtlinq::authorization",
                tool = %call.name,
                receipt_id = response
                    .pointer("/result/_meta/nxtlinq/receiptId")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown"),
                "authorization allowed; executing MCP handler"
            );
            Ok(())
        } else {
            let reason = response
                .pointer("/result/_meta/nxtlinq/reason")
                .and_then(Value::as_str)
                .unwrap_or("permission rejected");
            tracing::warn!(
                target: "nxtlinq::authorization",
                tool = %call.name,
                receipt_id = response
                    .pointer("/result/_meta/nxtlinq/receiptId")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown"),
                reason,
                "authorization denied; MCP handler skipped"
            );
            Err(format!(
                "{POLICY_DENIAL_PREFIX}: {denied_subject} reason={reason}. Only this exact call was denied; its handler did not run. The tool, workspace, and session remain available. Do not repeat the identical call during this user turn. On every later user message, invoke the requested tool normally—even the same tool with a different resource—because Nxtlinq authorizes each call independently. Reply now with a short plain-language explanation using the structured `buzz_message_send` tool, never shell; do not quote this raw result"
            ))
        }
    }
}

pub fn action_for_tool(call: &ToolCall, session_cwd: &Path) -> Result<Option<Value>, String> {
    let bare_name = call
        .name
        .rsplit_once("__")
        .map_or(call.name.as_str(), |(_, bare)| bare);
    let action_type = match bare_name {
        "read_file" => "filesystem:read",
        "str_replace" => "filesystem:write",
        "view_image" => {
            let path = call
                .arguments
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if path.starts_with("http://")
                || path.starts_with("https://")
                || path.starts_with("data:")
            {
                return Ok(None);
            }
            "filesystem:read"
        }
        "shell" => {
            let command = call
                .arguments
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{} requires a string command", call.name))?;
            let root = call
                .arguments
                .get("workdir")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_else(|| session_cwd.to_path_buf());
            let cwd = std::fs::canonicalize(&root)
                .map_err(|e| format!("workdir not accessible: {} ({e})", root.display()))?;
            return Ok(Some(json!({
                "type": "terminal:execute",
                "command": command,
                "args": [],
                "cwd": cwd,
                "environmentNames": []
            })));
        }
        _ => return Ok(None),
    };
    let path = call
        .arguments
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{} requires a string path", call.name))?;
    let root = call
        .arguments
        .get("workdir")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| session_cwd.to_path_buf());
    let candidate = if Path::new(path).is_absolute() {
        PathBuf::from(path)
    } else {
        root.join(path)
    };
    let resource = std::fs::canonicalize(&candidate)
        .map_err(|e| format!("path not accessible: {} ({e})", candidate.display()))?;
    Ok(Some(json!({ "type": action_type, "resource": resource })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::WireMsg;
    use serde_json::{json, Map};
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    fn call(name: &str, arguments: Value) -> ToolCall {
        ToolCall {
            provider_id: "call-1".into(),
            name: format!("buzz-dev-mcp__{name}"),
            arguments,
            provider_extra: Map::new(),
        }
    }

    #[test]
    fn read_file_uses_canonical_absolute_resource() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("README.md"), "safe").expect("fixture");
        let action = action_for_tool(
            &call("read_file", json!({ "path": "README.md" })),
            dir.path(),
        )
        .expect("action")
        .expect("protected tool");
        assert_eq!(action["type"], "filesystem:read");
        assert_eq!(
            action["resource"].as_str(),
            dir.path()
                .join("README.md")
                .canonicalize()
                .unwrap()
                .to_str()
        );
    }

    #[test]
    fn env_read_is_not_exempted() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join(".env"), "SECRET=fake").expect("fixture");
        let action = action_for_tool(&call("read_file", json!({ "path": ".env" })), dir.path())
            .expect("action")
            .expect("protected tool");
        assert_eq!(action["type"], "filesystem:read");
        assert!(action["resource"].as_str().unwrap().ends_with("/.env"));
    }

    #[test]
    fn shell_file_read_becomes_terminal_action() {
        let dir = tempdir().expect("tempdir");
        let action = action_for_tool(&call("shell", json!({ "command": "cat .env" })), dir.path())
            .expect("action")
            .expect("protected shell");
        assert_eq!(action["type"], "terminal:execute");
        assert_eq!(action["command"], "cat .env");
    }

    #[test]
    fn every_shell_command_requires_authorization() {
        let dir = tempdir().expect("tempdir");
        for command in [
            "buzz messages send --channel c --content safe",
            "printf 'safe' | buzz messages send --channel c --content -",
            "cat .env",
        ] {
            let action = action_for_tool(&call("shell", json!({ "command": command })), dir.path())
                .expect("action")
                .expect("protected shell");
            assert_eq!(action["type"], "terminal:execute");
        }
    }

    #[tokio::test]
    async fn permission_requires_gateway_allow_evidence() {
        let broker = Arc::new(PermissionBroker::default());
        let (wire_tx, mut wire_rx) = mpsc::channel(1);
        let task = {
            let broker = Arc::clone(&broker);
            tokio::spawn(async move {
                broker
                    .authorize(
                        &wire_tx,
                        "session-1",
                        &call("read_file", json!({ "path": "README.md" })),
                        json!({ "type": "filesystem:read", "resource": "/tmp/README.md" }),
                    )
                    .await
            })
        };
        let WireMsg::Notify(request) = wire_rx.recv().await.expect("request");
        let id = request["id"].clone();
        let allow_id = request["params"]["options"][0]["optionId"].clone();
        assert!(
            broker
                .resolve(
                    &id,
                    json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": {
                            "outcome": { "outcome": "selected", "optionId": allow_id },
                            "_meta": { "nxtlinq": { "decision": "allow", "reason": "in_scope" } }
                        }
                    }),
                )
                .await
        );
        assert!(task.await.expect("join").is_ok());
    }

    #[tokio::test]
    async fn plain_host_allow_without_gateway_evidence_fails_closed() {
        let broker = Arc::new(PermissionBroker::default());
        let (wire_tx, mut wire_rx) = mpsc::channel(1);
        let task = {
            let broker = Arc::clone(&broker);
            tokio::spawn(async move {
                broker
                    .authorize(
                        &wire_tx,
                        "session-1",
                        &call("read_file", json!({ "path": ".env" })),
                        json!({ "type": "filesystem:read", "resource": "/tmp/.env" }),
                    )
                    .await
            })
        };
        let WireMsg::Notify(request) = wire_rx.recv().await.expect("request");
        let id = request["id"].clone();
        let allow_id = request["params"]["options"][0]["optionId"].clone();
        broker
            .resolve(
                &id,
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "outcome": { "outcome": "selected", "optionId": allow_id } }
                }),
            )
            .await;
        let error = task.await.expect("join").unwrap_err();
        assert!(error.starts_with(POLICY_DENIAL_PREFIX));
        assert!(error.contains("capability=filesystem:read resource=/tmp/.env"));
        assert!(error.contains("Only this exact call was denied"));
        assert!(error.contains("tool, workspace, and session remain available"));
        assert!(error.contains("every later user message"));
        assert!(error.contains("different resource"));
    }
}
