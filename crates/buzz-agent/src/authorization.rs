use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::{oneshot, Mutex};

use crate::types::ToolCall;
use crate::wire::{self, WireSender};

const CONTRACT_VERSION: &str = "nxtlinq.authorization/v1";
const TRUSTED_DEV_MCP_SERVER: &str = "buzz-dev-mcp";
const SHELL_BASELINE_ENV: &str = "PATH";
const MAX_SHELL_ENVIRONMENT_NAMES: usize = 128;
const FORBIDDEN_SHELL_ENVIRONMENT_NAMES: &[&str] =
    &["BUZZ_PRIVATE_KEY", "NOSTR_PRIVATE_KEY", "BUZZ_AUTH_TAG"];
const CONTROL_PLANE_TOOLS: &[&str] = &[
    "buzz_message_send",
    "nxtlinq_setup",
    "todo",
    "_Stop",
    "_PostCompact",
];
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

#[derive(Debug)]
pub struct ToolAuthorization {
    pub actions: Vec<Value>,
    pub arguments: Value,
}

fn valid_environment_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b'A'..=b'Z' | b'a'..=b'z' | b'_'))
        && bytes.all(|byte| matches!(byte, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_'))
}

fn protected_shell_environment(arguments: &Value) -> Result<Vec<String>, String> {
    let requested: &[Value] = match arguments.get("environment") {
        Some(Value::Array(values)) => values,
        Some(_) => return Err("shell environment must be an array of variable names".into()),
        None => &[],
    };
    if requested.len() > MAX_SHELL_ENVIRONMENT_NAMES {
        return Err(format!(
            "shell environment contains too many names: {} > {MAX_SHELL_ENVIRONMENT_NAMES}",
            requested.len()
        ));
    }

    // PATH is the one modeled baseline: without the MCP-owned shim PATH the
    // exact approved command would resolve differently (or not at all).
    let mut names = vec![SHELL_BASELINE_ENV.to_string()];
    for value in requested {
        let name = value
            .as_str()
            .ok_or_else(|| "shell environment must contain variable names only".to_string())?;
        if !valid_environment_name(name) {
            return Err(format!("invalid shell environment variable name: {name}"));
        }
        if FORBIDDEN_SHELL_ENVIRONMENT_NAMES.contains(&name) {
            return Err(format!(
                "protected shell cannot receive host identity variable: {name}"
            ));
        }
        if name != SHELL_BASELINE_ENV && !names.iter().any(|existing| existing == name) {
            names.push(name.to_string());
        }
    }
    if names.len() > MAX_SHELL_ENVIRONMENT_NAMES {
        return Err(format!(
            "shell environment contains too many distinct names: {} > {MAX_SHELL_ENVIRONMENT_NAMES}",
            names.len()
        ));
    }
    Ok(names)
}

fn canonical_resource(
    call: &ToolCall,
    session_cwd: &Path,
    field: &str,
) -> Result<(PathBuf, Value), String> {
    let path = call
        .arguments
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{} requires a string {field}", call.name))?;
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
        .map_err(|error| format!("path not accessible: {} ({error})", candidate.display()))?;
    let mut arguments = call.arguments.clone();
    let object = arguments
        .as_object_mut()
        .ok_or_else(|| format!("{} arguments must be an object", call.name))?;
    object.insert(
        field.to_string(),
        Value::String(resource.display().to_string()),
    );
    Ok((resource, arguments))
}

pub fn action_for_tool(call: &ToolCall, session_cwd: &Path) -> Result<ToolAuthorization, String> {
    let (server_name, bare_name) = call
        .name
        // Server names cannot contain `__`, while hook names may begin with
        // `_` (producing three underscores at the boundary). Split from the
        // left so `_Stop` remains the bare tool name.
        .split_once("__")
        .ok_or_else(|| format!("unqualified MCP tool name: {}", call.name))?;
    if server_name.is_empty() || bare_name.is_empty() {
        return Err(format!("invalid qualified MCP tool name: {}", call.name));
    }

    let mcp_action = || {
        json!({
            "type": "mcp:invoke",
            "server": server_name,
            "tool": bare_name
        })
    };

    // These Buzz-owned tools either maintain in-process state or implement the
    // owner-review/reply control plane. They grant no project filesystem or
    // command authority and must remain usable to report a policy denial.
    if server_name == TRUSTED_DEV_MCP_SERVER && CONTROL_PLANE_TOOLS.contains(&bare_name) {
        return Ok(ToolAuthorization {
            actions: Vec::new(),
            arguments: call.arguments.clone(),
        });
    }

    // Only the bundled server is trusted to implement these semantic tools.
    // A same-named tool from another MCP server is authorized as mcp:invoke,
    // never mistaken for a filesystem or terminal operation.
    if server_name != TRUSTED_DEV_MCP_SERVER {
        return Ok(ToolAuthorization {
            actions: vec![mcp_action()],
            arguments: call.arguments.clone(),
        });
    }

    match bare_name {
        "read_file" => {
            let (resource, arguments) = canonical_resource(call, session_cwd, "path")?;
            Ok(ToolAuthorization {
                actions: vec![json!({ "type": "filesystem:read", "resource": resource })],
                arguments,
            })
        }
        // str_replace reads the existing file before it writes. Requiring both
        // capabilities prevents a write grant from becoming an implicit read.
        "str_replace" => {
            let (resource, arguments) = canonical_resource(call, session_cwd, "path")?;
            Ok(ToolAuthorization {
                actions: vec![
                    json!({ "type": "filesystem:read", "resource": resource }),
                    json!({ "type": "filesystem:write", "resource": resource }),
                ],
                arguments,
            })
        }
        "view_image" => {
            let source = call
                .arguments
                .get("source")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{} requires a string source", call.name))?;
            if source.starts_with("data:") {
                return Ok(ToolAuthorization {
                    actions: Vec::new(),
                    arguments: call.arguments.clone(),
                });
            }
            if source.starts_with("http://") || source.starts_with("https://") {
                return Ok(ToolAuthorization {
                    actions: vec![mcp_action()],
                    arguments: call.arguments.clone(),
                });
            }
            let (resource, arguments) = canonical_resource(call, session_cwd, "source")?;
            Ok(ToolAuthorization {
                actions: vec![json!({ "type": "filesystem:read", "resource": resource })],
                arguments,
            })
        }
        "shell" => {
            let input = call
                .arguments
                .as_object()
                .ok_or_else(|| format!("{} arguments must be an object", call.name))?;
            if let Some(field) = input.keys().find(|field| {
                !matches!(
                    field.as_str(),
                    "command" | "workdir" | "timeout_ms" | "environment"
                )
            }) {
                return Err(format!("{} has an unsupported field: {field}", call.name));
            }
            let command = input
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{} requires a string command", call.name))?;
            if command.is_empty() {
                return Err(format!("{} requires a non-empty command", call.name));
            }
            let requested_root = match input.get("workdir") {
                Some(Value::String(root)) => PathBuf::from(root),
                Some(_) => return Err(format!("{} requires a string workdir", call.name)),
                None => session_cwd.to_path_buf(),
            };
            let root = if requested_root.is_absolute() {
                requested_root
            } else {
                session_cwd.join(requested_root)
            };
            let cwd = std::fs::canonicalize(&root)
                .map_err(|e| format!("workdir not accessible: {} ({e})", root.display()))?;
            if !cwd.is_dir() {
                return Err(format!("workdir is not a directory: {}", cwd.display()));
            }
            let cwd_string = cwd
                .to_str()
                .ok_or_else(|| format!("workdir is not valid UTF-8: {}", cwd.display()))?
                .to_string();
            let environment_names = protected_shell_environment(&call.arguments)?;
            let mut arguments = call.arguments.clone();
            let object = arguments
                .as_object_mut()
                .ok_or_else(|| format!("{} arguments must be an object", call.name))?;
            object.insert("workdir".into(), Value::String(cwd_string.clone()));
            object.insert(
                "environment".into(),
                Value::Array(
                    environment_names
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
            Ok(ToolAuthorization {
                actions: vec![json!({
                    "type": "terminal:execute",
                    "command": command,
                    "args": [],
                    "cwd": cwd_string,
                    "environmentNames": environment_names
                })],
                arguments,
            })
        }
        _ => Ok(ToolAuthorization {
            actions: vec![mcp_action()],
            arguments: call.arguments.clone(),
        }),
    }
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

    fn only_action(call: &ToolCall, cwd: &Path) -> Value {
        let authorization = action_for_tool(call, cwd).expect("authorization");
        assert_eq!(authorization.actions.len(), 1);
        authorization.actions.into_iter().next().expect("action")
    }

    #[test]
    fn read_file_uses_canonical_absolute_resource() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("README.md"), "safe").expect("fixture");
        let call = call("read_file", json!({ "path": "README.md" }));
        let authorization = action_for_tool(&call, dir.path()).expect("authorization");
        let action = &authorization.actions[0];
        assert_eq!(action["type"], "filesystem:read");
        assert_eq!(
            action["resource"].as_str(),
            dir.path()
                .join("README.md")
                .canonicalize()
                .unwrap()
                .to_str()
        );
        assert_eq!(authorization.arguments["path"], action["resource"]);
    }

    #[test]
    fn env_read_is_not_exempted() {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join(".env"), "SECRET=fake").expect("fixture");
        let action = only_action(&call("read_file", json!({ "path": ".env" })), dir.path());
        assert_eq!(action["type"], "filesystem:read");
        assert!(action["resource"].as_str().unwrap().ends_with("/.env"));
    }

    #[test]
    fn shell_file_read_becomes_terminal_action() {
        let dir = tempdir().expect("tempdir");
        let call = call("shell", json!({ "command": "cat .env" }));
        let authorization = action_for_tool(&call, dir.path()).expect("authorization");
        let action = &authorization.actions[0];
        assert_eq!(action["type"], "terminal:execute");
        assert_eq!(action["command"], "cat .env");
        assert_eq!(action["environmentNames"], json!(["PATH"]));
        assert_eq!(authorization.arguments["environment"], json!(["PATH"]));
        assert_eq!(authorization.arguments["workdir"], action["cwd"]);
    }

    #[test]
    fn every_shell_command_requires_authorization() {
        let dir = tempdir().expect("tempdir");
        for command in [
            "buzz messages send --channel c --content safe",
            "printf 'safe' | buzz messages send --channel c --content -",
            "cat .env",
        ] {
            let action = only_action(&call("shell", json!({ "command": command })), dir.path());
            assert_eq!(action["type"], "terminal:execute");
        }
    }

    #[test]
    fn shell_environment_and_workdir_are_normalized_and_bound_to_execution() {
        let dir = tempdir().expect("tempdir");
        let subdir = dir.path().join("subdir");
        std::fs::create_dir(&subdir).expect("subdir");
        let authorization = action_for_tool(
            &call(
                "shell",
                json!({
                    "command": "npm test",
                    "workdir": "subdir",
                    "environment": ["CI", "PATH", "CI"]
                }),
            ),
            dir.path(),
        )
        .expect("authorization");
        let action = &authorization.actions[0];
        assert_eq!(action["environmentNames"], json!(["PATH", "CI"]));
        assert_eq!(
            action["cwd"].as_str(),
            subdir.canonicalize().expect("canonical subdir").to_str()
        );
        assert_eq!(
            authorization.arguments["environment"],
            json!(["PATH", "CI"])
        );
        assert_eq!(authorization.arguments["workdir"], action["cwd"]);
    }

    #[test]
    fn shell_environment_rejects_values_and_invalid_names() {
        let dir = tempdir().expect("tempdir");
        for environment in [
            json!("PATH"),
            json!(["TOKEN=secret"]),
            json!(["BUZZ_PRIVATE_KEY"]),
            json!([42]),
        ] {
            assert!(action_for_tool(
                &call(
                    "shell",
                    json!({ "command": "env", "environment": environment }),
                ),
                dir.path(),
            )
            .is_err());
        }
        assert!(action_for_tool(
            &call("shell", json!({ "command": "pwd", "workdir": 42 })),
            dir.path(),
        )
        .is_err());
        assert!(action_for_tool(
            &call("shell", json!({ "command": "pwd", "network": true })),
            dir.path(),
        )
        .is_err());
    }

    #[test]
    fn unknown_tools_require_explicit_mcp_invoke_authorization() {
        let dir = tempdir().expect("tempdir");
        let action = only_action(&call("future_tool", json!({})), dir.path());
        assert_eq!(action["type"], "mcp:invoke");
        assert_eq!(action["server"], "buzz-dev-mcp");
        assert_eq!(action["tool"], "future_tool");
    }

    #[test]
    fn untrusted_same_named_tools_do_not_gain_semantic_authority() {
        let dir = tempdir().expect("tempdir");
        let mut call = call("read_file", json!({ "path": ".env" }));
        call.name = "third-party__read_file".into();
        let action = only_action(&call, dir.path());
        assert_eq!(action["type"], "mcp:invoke");
        assert_eq!(action["server"], "third-party");
        assert_eq!(action["tool"], "read_file");
    }

    #[test]
    fn remote_image_fetch_requires_mcp_invoke_authorization() {
        let dir = tempdir().expect("tempdir");
        let action = only_action(
            &call(
                "view_image",
                json!({ "source": "https://example.test/image.png" }),
            ),
            dir.path(),
        );
        assert_eq!(action["type"], "mcp:invoke");
        assert_eq!(action["tool"], "view_image");
    }

    #[test]
    fn reply_and_review_control_plane_tools_remain_available() {
        let dir = tempdir().expect("tempdir");
        for name in CONTROL_PLANE_TOOLS {
            assert!(action_for_tool(&call(name, json!({})), dir.path())
                .expect("authorization")
                .actions
                .is_empty());
        }
    }

    #[test]
    fn str_replace_requires_read_and_write_for_the_same_resource() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("src.txt");
        std::fs::write(&target, "before").expect("fixture");
        let authorization = action_for_tool(
            &call(
                "str_replace",
                json!({ "path": "src.txt", "old_str": "before", "new_str": "after" }),
            ),
            dir.path(),
        )
        .expect("authorization");
        assert_eq!(authorization.actions.len(), 2);
        assert_eq!(authorization.actions[0]["type"], "filesystem:read");
        assert_eq!(authorization.actions[1]["type"], "filesystem:write");
        assert_eq!(
            authorization.actions[0]["resource"],
            authorization.actions[1]["resource"]
        );
        assert_eq!(
            authorization.arguments["path"],
            authorization.actions[0]["resource"]
        );
    }

    #[test]
    fn local_image_uses_source_and_binds_execution_to_canonical_path() {
        let dir = tempdir().expect("tempdir");
        let target = dir.path().join("image.png");
        std::fs::write(&target, "image").expect("fixture");
        let authorization = action_for_tool(
            &call("view_image", json!({ "source": "image.png" })),
            dir.path(),
        )
        .expect("authorization");
        assert_eq!(authorization.actions[0]["type"], "filesystem:read");
        assert_eq!(
            authorization.arguments["source"],
            authorization.actions[0]["resource"]
        );
    }

    #[test]
    fn unqualified_tools_fail_closed() {
        let dir = tempdir().expect("tempdir");
        let mut call = call("read_file", json!({ "path": "README.md" }));
        call.name = "read_file".into();
        assert!(action_for_tool(&call, dir.path())
            .expect_err("unqualified tool must fail")
            .contains("unqualified MCP tool"));
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
