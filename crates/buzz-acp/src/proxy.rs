//! Stdio JSON-RPC MCP tool-filter proxy.
//!
//! Spawned by the parent `buzz-acp` when a v2 `tool_filter` is attached to a
//! configured MCP server. The proxy stands between the agent and the real
//! MCP server:
//!
//! (Agent stdio on one side, the real MCP child on the other; the proxy in the middle.)
//!
//! It applies the same v2 policy at two distinct points in the wire:
//!
//! - `tools/list` *response*: strip tools whose bare name is not permitted by
//!   the policy before forwarding to the agent.
//! - `tools/call` *request*: parse `params.name`; if the name is not permitted
//!   (or `params.name` is missing/malformed) emit a JSON-RPC error response
//!   directly to the agent and never forward to the child.
//!
//! Every other frame — `initialize`, `notifications/*`, server-initiated
//! events, unknown methods — is forwarded byte-identical. The proxy is
//! transparent outside the two filtered methods so the OAuth dance and any
//! other protocol surface that mcp-remote or the upstream server relies on
//! is preserved.
//!
//! Defense in depth: the proxy re-reads the sidecar JSON at startup and
//! re-validates the filter against the latest file contents, so a tampered
//! or rotated file after parent load cannot silently widen the policy.
//!
//! Failure modes are fail-closed: missing sidecar, unknown server, missing
//! `params.name`, denied name, malformed `tools/list` response, or child
//! death all terminate the proxy with a non-zero exit.

use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tracing::{debug, info, warn};

use crate::config::{load_mcp_config, ConfiguredMcpServer, ToolFilter};

/// JSON-RPC 2.0 standard error code used for policy denials and malformed
/// `tools/call` requests.
const JSONRPC_INVALID_REQUEST: i64 = -32600;

/// Parsed sidecar argv for `--mcp-filter-proxy`.
#[derive(Debug)]
struct ProxyArgs {
    sidecar_path: PathBuf,
    server_name: String,
    child_command: String,
    child_args: Vec<String>,
}

/// Resolved filter state used at runtime to evaluate inbound/outbound frames.
#[derive(Debug)]
struct FilterState {
    server_name: String,
    allow: Option<HashSet<String>>,
    deny: Option<HashSet<String>>,
    strict_allow: bool,
}

impl FilterState {
    fn from_filter(server_name: &str, filter: &ToolFilter) -> Self {
        Self {
            server_name: server_name.to_string(),
            allow: filter
                .allow
                .as_ref()
                .map(|names| names.iter().cloned().collect::<HashSet<_>>()),
            deny: filter
                .deny
                .as_ref()
                .map(|names| names.iter().cloned().collect::<HashSet<_>>()),
            strict_allow: filter.strict_allow,
        }
    }

    fn permits(&self, tool_name: &str) -> bool {
        match (&self.allow, &self.deny) {
            (Some(allow), None) => allow.contains(tool_name),
            (None, Some(deny)) => !deny.contains(tool_name),
            // is_well_formed() guarantees this branch is unreachable at load.
            _ => false,
        }
    }
}

/// Outcome of inspecting a single inbound agent→child frame.
#[derive(Debug)]
enum AgentFrameAction {
    /// Forward the original line verbatim to the child, and remember this id
    /// as a pending `tools/list` request so the response can be filtered.
    ForwardToolsList { raw_line: String, id: Value },
    /// Forward the original line verbatim to the child.
    ForwardRaw(String),
    /// Drop the frame; emit a JSON-RPC error response to the agent with the
    /// given id. The child never sees this call.
    Deny {
        id: Value,
        code: i64,
        message: String,
    },
    /// Drop the frame silently (notification form or invalid JSON).
    Drop { reason: &'static str },
}

/// Spawned by `tokio_main()` when `argv[1] == "--mcp-filter-proxy"`. Resolves
/// the policy from disk, spawns the upstream server, and runs the frame loop
/// until either side closes stdio.
pub(crate) async fn run_mcp_filter_proxy() -> Result<()> {
    let args = parse_proxy_args().context("invalid --mcp-filter-proxy argv")?;
    let filter = load_filter_state(args.sidecar_path.as_path(), &args.server_name)
        .context("failed to load MCP filter policy")?;

    info!(
        server = %args.server_name,
        sidecar = %args.sidecar_path.display(),
        mode = if filter.allow.is_some() { "allow" } else { "deny" },
        allow_n = filter.allow.as_ref().map(|s| s.len()).unwrap_or(0),
        deny_n = filter.deny.as_ref().map(|s| s.len()).unwrap_or(0),
        strict = filter.strict_allow,
        "mcp-filter-proxy: spawning child"
    );

    let mut command = Command::new(&args.child_command);
    command
        .args(&args.child_args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);

    let mut child = command
        .spawn()
        .with_context(|| format!("failed to spawn MCP child {}", args.child_command))?;
    let child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("MCP child stdin unavailable"))?;
    let child_stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("MCP child stdout unavailable"))?;

    let proxy_result = run_proxy_loop(
        filter,
        tokio::io::stdin(),
        tokio::io::stdout(),
        child_stdout,
        child_stdin,
    )
    .await;

    if let Err(error) = child.start_kill() {
        debug!(?error, "child already exited");
    }
    let _ = child.wait().await;

    proxy_result
}

/// Core loop. Generic over I/O so tests can plug in `tokio::io::DuplexStream`
/// halves instead of spawning real processes.
async fn run_proxy_loop<AR, AW, CR, CW>(
    filter: FilterState,
    agent_reader: AR,
    mut agent_writer: AW,
    child_reader: CR,
    mut child_writer: CW,
) -> Result<()>
where
    AR: AsyncRead + Unpin,
    AW: AsyncWrite + Unpin,
    CR: AsyncRead + Unpin,
    CW: AsyncWrite + Unpin,
{
    let mut agent_in = BufReader::new(agent_reader).lines();
    let mut child_in = BufReader::new(child_reader).lines();
    // Pending `tools/list` request ids awaiting their filtered response. JSON-RPC
    // ids can be string, number, or null — we hash on the JSON representation,
    // which serde_json guarantees is stable for the same Value.
    let mut pending_list_ids: HashSet<String> = HashSet::new();
    let mut list_seen: bool = false;

    loop {
        tokio::select! {
            agent_line = agent_in.next_line() => {
                let line = match agent_line? {
                    Some(line) => line,
                    None => {
                        // Agent closed stdin. Half-close the child so it can
                        // exit cleanly; then exit ourselves with success.
                        let _ = shutdown_writer(&mut child_writer).await;
                        return Ok(());
                    }
                };
                let action = classify_agent_frame(&line, &filter);
                match action {
                    AgentFrameAction::ForwardToolsList { raw_line, id } => {
                        pending_list_ids.insert(canonical_id(&id));
                        write_frame(&mut child_writer, raw_line.as_bytes()).await?;
                    }
                    AgentFrameAction::ForwardRaw(raw) => {
                        write_frame(&mut child_writer, raw.as_bytes()).await?;
                    }
                    AgentFrameAction::Deny { id, code, message } => {
                        warn!(
                            server = %filter.server_name,
                            code,
                            message,
                            "mcp-filter-proxy: blocking tool call"
                        );
                        let response = jsonrpc_error_response(&id, code, &message);
                        write_frame(&mut agent_writer, response.as_bytes()).await?;
                    }
                    AgentFrameAction::Drop { reason } => {
                        debug!(reason, "mcp-filter-proxy: dropping agent frame");
                    }
                }
            }
            child_line = child_in.next_line() => {
                let line = match child_line? {
                    Some(line) => line,
                    None => {
                        // Child closed stdout. Exit non-zero so the agent
                        // surfaces a transport error instead of thinking the
                        // server is healthy.
                        let _ = shutdown_writer(&mut agent_writer).await;
                        bail!("MCP child closed stdout");
                    }
                };
                let forwarded = filter_child_frame(
                    &line,
                    &filter,
                    &mut pending_list_ids,
                    &mut list_seen,
                )?;
                if let Some(payload) = forwarded {
                    write_frame(&mut agent_writer, payload.as_bytes()).await?;
                }
            }
        }
    }
}

/// Decide what to do with a single agent→child JSON-RPC frame.
fn classify_agent_frame(line: &str, filter: &FilterState) -> AgentFrameAction {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return AgentFrameAction::Drop {
            reason: "empty line",
        };
    }
    let parsed: Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(error) => {
            debug!(
                ?error,
                "mcp-filter-proxy: invalid JSON from agent; dropping"
            );
            return AgentFrameAction::Drop {
                reason: "invalid json",
            };
        }
    };
    let object = match parsed.as_object() {
        Some(object) => object,
        None => return AgentFrameAction::ForwardRaw(line.to_string()),
    };
    let method = object.get("method").and_then(|m| m.as_str());
    let id = object.get("id").cloned();
    match method {
        Some("tools/call") => classify_tools_call(line, &parsed, id, filter),
        Some("tools/list") => match id {
            Some(id) => AgentFrameAction::ForwardToolsList {
                raw_line: line.to_string(),
                id,
            },
            None => AgentFrameAction::ForwardRaw(line.to_string()),
        },
        _ => AgentFrameAction::ForwardRaw(line.to_string()),
    }
}

/// Apply the policy to a `tools/call` frame.
fn classify_tools_call(
    raw_line: &str,
    parsed: &Value,
    id: Option<Value>,
    filter: &FilterState,
) -> AgentFrameAction {
    let Some(id) = id else {
        // Notification form (no id). Drop — the policy still applies and
        // there is no id to emit a response against.
        return AgentFrameAction::Drop {
            reason: "tools/call without id",
        };
    };
    let tool_name = parsed
        .get("params")
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str());
    match tool_name {
        None => AgentFrameAction::Deny {
            id,
            code: JSONRPC_INVALID_REQUEST,
            message: "tools/call missing params.name".into(),
        },
        Some(name) => {
            if !filter.permits(name) {
                AgentFrameAction::Deny {
                    id,
                    code: JSONRPC_INVALID_REQUEST,
                    message: format!("tool '{name}' is not permitted by tool_filter"),
                }
            } else {
                AgentFrameAction::ForwardRaw(raw_line.to_string())
            }
        }
    }
}

/// Filter a child→agent frame. Returns `Some(line)` to forward to the agent,
/// or `None` for empty/garbage lines. Bails (non-zero exit) on malformed
/// `tools/list` responses so the agent never sees partial catalog data.
fn filter_child_frame(
    line: &str,
    filter: &FilterState,
    pending_list_ids: &mut HashSet<String>,
    list_seen: &mut bool,
) -> Result<Option<String>> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    // Parse first; bail if the child emitted invalid JSON.
    let parsed: Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(error) => {
            bail!("MCP child emitted invalid JSON: {error}");
        }
    };
    let Some(object) = parsed.as_object() else {
        return Ok(Some(line.to_string()));
    };
    let Some(result) = object.get("result") else {
        return Ok(Some(line.to_string()));
    };
    let Some(id) = object.get("id").cloned() else {
        return Ok(Some(line.to_string()));
    };
    let id_key = canonical_id(&id);
    if !pending_list_ids.remove(&id_key) {
        return Ok(Some(line.to_string()));
    }
    let Some(tools_value) = result.get("tools") else {
        // tools/list response without `tools` key — forward as-is.
        return Ok(Some(line.to_string()));
    };
    let Some(tools_array) = tools_value.as_array() else {
        bail!("MCP child tools/list result.tools is not an array");
    };
    let before = tools_array.len();
    let kept_names: Vec<String> = tools_array
        .iter()
        .filter_map(|tool| {
            tool.get("name")
                .and_then(|name| name.as_str())
                .filter(|name| filter.permits(name))
                .map(str::to_string)
        })
        .collect();
    let mut modified = parsed.clone();
    let filtered_tools: Vec<Value> = tools_array
        .iter()
        .filter(|tool| {
            tool.get("name")
                .and_then(|name| name.as_str())
                .map(|name| filter.permits(name))
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    if filtered_tools.len() == before {
        // No filtering needed — forward original line byte-identical.
        if !*list_seen {
            check_strict_allow(filter, &kept_names)?;
            log_first_catalog(filter, &kept_names);
            *list_seen = true;
        }
        return Ok(Some(line.to_string()));
    }
    modified["result"]["tools"] = Value::Array(filtered_tools);
    if !*list_seen {
        check_strict_allow(filter, &kept_names)?;
        log_first_catalog(filter, &kept_names);
        *list_seen = true;
    }
    Ok(Some(serialize_frame(&modified)))
}

/// In strict mode, bail (non-zero exit) when any allowed name is missing
/// from the resolved catalog — surfacing Atlassian tool renames as a hard
/// failure instead of a silent deny-all.
fn check_strict_allow(filter: &FilterState, catalog_names: &[String]) -> Result<()> {
    if !filter.strict_allow {
        return Ok(());
    }
    let Some(allow) = &filter.allow else {
        return Ok(());
    };
    let mut missing: Vec<String> = Vec::new();
    for name in allow {
        if !catalog_names.iter().any(|n| n == name) {
            missing.push(name.clone());
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    missing.sort();
    bail!(
        "tool_filter.strict_allow: names not in catalog: {}",
        missing.join(", ")
    );
}

fn log_first_catalog(filter: &FilterState, catalog_names: &[String]) {
    let mut missing: Vec<String> = Vec::new();
    if let Some(allow) = &filter.allow {
        for name in allow {
            if !catalog_names.iter().any(|n| n == name) {
                missing.push(name.clone());
            }
        }
    }
    if !missing.is_empty() {
        missing.sort();
        warn!(
            server = %filter.server_name,
            missing = %missing.join(", "),
            "tool_filter names not present in upstream catalog"
        );
    }
    let mode = if filter.allow.is_some() {
        "allow"
    } else {
        "deny"
    };
    info!(
        server = %filter.server_name,
        mode,
        catalog_total = catalog_names.len(),
        "mcp-filter-proxy: catalog resolved"
    );
}

/// Parse the argv laid out by `build_mcp_servers` for the proxy sub-mode.
///
/// Layout: `current_exe --mcp-filter-proxy <sidecar> <server> -- <cmd> <args...>`
fn parse_proxy_args() -> Result<ProxyArgs> {
    let mut args = std::env::args().skip(1);
    let head = args
        .next()
        .ok_or_else(|| anyhow!("missing --mcp-filter-proxy token"))?;
    if head != "--mcp-filter-proxy" {
        bail!("first arg must be --mcp-filter-proxy, got '{head}'");
    }
    let sidecar_path = args.next().ok_or_else(|| anyhow!("missing sidecar path"))?;
    let server_name = args.next().ok_or_else(|| anyhow!("missing server name"))?;
    let separator = args
        .next()
        .ok_or_else(|| anyhow!("missing '--' separator"))?;
    if separator != "--" {
        bail!("expected '--' separator after server name, got '{separator}'");
    }
    let child_command = args
        .next()
        .ok_or_else(|| anyhow!("missing child command"))?;
    let child_args: Vec<String> = args.collect();
    Ok(ProxyArgs {
        sidecar_path: PathBuf::from(sidecar_path),
        server_name,
        child_command,
        child_args,
    })
}

/// Re-load the sidecar and resolve the filter for the named server. Defense
/// in depth — the parent already validated at startup, but the proxy
/// independently parses the file so a tampered or rotated file cannot widen
/// the policy after the fact.
fn load_filter_state(sidecar_path: &std::path::Path, server_name: &str) -> Result<FilterState> {
    let servers = load_mcp_config(sidecar_path, "")
        .with_context(|| format!("failed to load MCP config {}", sidecar_path.display()))?;
    let server = servers
        .into_iter()
        .find(|server| match server {
            ConfiguredMcpServer::Stdio { name, .. } => name == server_name,
        })
        .ok_or_else(|| anyhow!("server '{server_name}' not found in sidecar"))?;
    let ConfiguredMcpServer::Stdio {
        name, tool_filter, ..
    } = server;
    let filter = tool_filter
        .ok_or_else(|| anyhow!("server '{name}' has no tool_filter; proxy must not run"))?;
    Ok(FilterState::from_filter(&name, &filter))
}

fn jsonrpc_error_response(id: &Value, code: i64, message: &str) -> String {
    let response = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
        }
    });
    serialize_frame(&response)
}

fn serialize_frame(value: &Value) -> String {
    serde_json::to_string(value).expect("JSON-RPC frame serializes")
}

/// Canonical key for a JSON-RPC id used to track in-flight requests.
fn canonical_id(id: &Value) -> String {
    id.to_string()
}

async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, bytes: &[u8]) -> Result<()> {
    writer.write_all(bytes).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    Ok(())
}

async fn shutdown_writer<W: AsyncWrite + Unpin>(writer: &mut W) -> io::Result<()> {
    writer.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tokio::io::{duplex, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    /// Pair of independent duplex halves. `write` is the end the test writes
    /// to (data flows `test → proxy`); `read` is the end the test reads from
    /// (data flows `proxy → test`).
    struct DuplexPair {
        write: tokio::io::DuplexStream,
        read: tokio::io::DuplexStream,
    }

    fn spawn_proxy_pair(
        filter: FilterState,
    ) -> (
        tokio::task::JoinHandle<Result<()>>,
        DuplexPair, // agent
        DuplexPair, // child
    ) {
        // Agent: test → proxy ↔ proxy → test
        let (agent_write, proxy_read_agent) = duplex(64 * 1024);
        let (proxy_write_agent, agent_read) = duplex(64 * 1024);
        // Child: test → proxy ↔ proxy → test
        let (child_write, proxy_read_child) = duplex(64 * 1024);
        let (proxy_write_child, child_read) = duplex(64 * 1024);

        let handle = tokio::spawn(async move {
            run_proxy_loop(
                filter,
                proxy_read_agent,
                proxy_write_agent,
                proxy_read_child,
                proxy_write_child,
            )
            .await
        });

        (
            handle,
            DuplexPair {
                write: agent_write,
                read: agent_read,
            },
            DuplexPair {
                write: child_write,
                read: child_read,
            },
        )
    }

    fn allow_filter(allow: &[&str]) -> FilterState {
        let filter = ToolFilter {
            allow: Some(allow.iter().map(|s| (*s).to_string()).collect()),
            deny: None,
            strict_allow: false,
        };
        FilterState::from_filter("test", &filter)
    }

    fn deny_filter(deny: &[&str]) -> FilterState {
        let filter = ToolFilter {
            allow: None,
            deny: Some(deny.iter().map(|s| (*s).to_string()).collect()),
            strict_allow: false,
        };
        FilterState::from_filter("test", &filter)
    }

    /// Write a JSON-RPC frame followed by `\n` to the writer half.
    async fn write_json<W: AsyncWrite + Unpin>(writer: &mut W, payload: &str) {
        writer.write_all(payload.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();
    }

    #[tokio::test]
    async fn strips_tools_list_response_via_allow() {
        let filter = allow_filter(&["getJiraIssue", "addCommentToJiraIssue"]);
        let (handle, mut agent, mut child) = spawn_proxy_pair(filter);

        write_json(
            &mut agent.write,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .await;

        // Child receives the request verbatim.
        let mut child_reader = BufReader::new(&mut child.read);
        let mut received = String::new();
        child_reader.read_line(&mut received).await.unwrap();
        let received = received.trim_end_matches('\n').to_string();
        assert_eq!(
            received,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#
        );

        // Child responds with 3 tools, 2 allowed.
        write_json(
            &mut child.write,
            r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"getJiraIssue"},{"name":"addCommentToJiraIssue"},{"name":"deleteEverything"}]}}"#,
        )
        .await;

        let mut agent_reader = BufReader::new(&mut agent.read);
        let mut received = String::new();
        agent_reader.read_line(&mut received).await.unwrap();
        let parsed: Value = serde_json::from_str(received.trim_end_matches('\n')).unwrap();
        let tools = parsed["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["getJiraIssue", "addCommentToJiraIssue"]);

        drop(agent.write);
        drop(agent.read);
        drop(child.write);
        drop(child.read);
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn strips_tools_list_response_via_deny() {
        let filter = deny_filter(&["deleteEverything"]);
        let (handle, mut agent, mut child) = spawn_proxy_pair(filter);

        write_json(
            &mut agent.write,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .await;
        let mut child_reader = BufReader::new(&mut child.read);
        let mut discard = String::new();
        let _ = child_reader.read_line(&mut discard).await;

        write_json(
            &mut child.write,
            r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"getJiraIssue"},{"name":"deleteEverything"}]}}"#,
        )
        .await;

        let mut agent_reader = BufReader::new(&mut agent.read);
        let mut received = String::new();
        agent_reader.read_line(&mut received).await.unwrap();
        let parsed: Value = serde_json::from_str(received.trim_end_matches('\n')).unwrap();
        let tools = parsed["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["getJiraIssue"]);

        drop(agent.write);
        drop(agent.read);
        drop(child.write);
        drop(child.read);
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn tools_list_passthrough_when_all_allowed_is_byte_identical() {
        let filter = allow_filter(&["getJiraIssue", "addCommentToJiraIssue"]);
        let (handle, mut agent, mut child) = spawn_proxy_pair(filter);

        write_json(
            &mut agent.write,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .await;
        let mut child_reader = BufReader::new(&mut child.read);
        let mut discard = String::new();
        let _ = child_reader.read_line(&mut discard).await;

        let original = r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"getJiraIssue"},{"name":"addCommentToJiraIssue"}]}}"#;
        write_json(&mut child.write, original).await;

        let mut agent_reader = BufReader::new(&mut agent.read);
        let mut received = String::new();
        agent_reader.read_line(&mut received).await.unwrap();
        assert_eq!(received.trim_end_matches('\n'), original);

        drop(agent.write);
        drop(agent.read);
        drop(child.write);
        drop(child.read);
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn tools_call_allowed_is_byte_identical() {
        let filter = allow_filter(&["getJiraIssue"]);
        let (handle, mut agent, mut child) = spawn_proxy_pair(filter);

        let original = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"getJiraIssue","arguments":{"issueId":"ABC-1"}}}"#;
        write_json(&mut agent.write, original).await;

        let mut child_reader = BufReader::new(&mut child.read);
        let mut received = String::new();
        child_reader.read_line(&mut received).await.unwrap();
        assert_eq!(received.trim_end_matches('\n'), original);

        drop(agent.write);
        drop(agent.read);
        drop(child.write);
        drop(child.read);
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn tools_call_denied_name_returns_error_and_does_not_forward() {
        let filter = allow_filter(&["getJiraIssue"]);
        let (handle, mut agent, mut child) = spawn_proxy_pair(filter);

        write_json(
            &mut agent.write,
            r#"{"jsonrpc":"2.0","id":42,"method":"tools/call","params":{"name":"deleteEverything"}}"#,
        )
        .await;

        // Agent receives JSON-RPC error with the original id.
        let mut agent_reader = BufReader::new(&mut agent.read);
        let mut received = String::new();
        agent_reader.read_line(&mut received).await.unwrap();
        let parsed: Value = serde_json::from_str(received.trim_end_matches('\n')).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], json!(42));
        assert_eq!(parsed["error"]["code"], JSONRPC_INVALID_REQUEST);
        let msg = parsed["error"]["message"].as_str().unwrap();
        assert!(msg.contains("deleteEverything"));
        assert!(msg.contains("not permitted"));

        // Child never sees the frame: read with short timeout must time out.
        let mut probe = vec![0u8; 256];
        let read = tokio::time::timeout(
            std::time::Duration::from_millis(150),
            child.read.read(&mut probe),
        )
        .await;
        assert!(
            read.is_err(),
            "child must not receive denied frame; got {read:?}"
        );

        drop(agent.write);
        drop(agent.read);
        drop(child.write);
        drop(child.read);
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn tools_call_missing_params_name_is_fail_closed() {
        let filter = allow_filter(&["getJiraIssue"]);
        let (handle, mut agent, mut child) = spawn_proxy_pair(filter);

        write_json(
            &mut agent.write,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"arguments":{}}}"#,
        )
        .await;

        let mut agent_reader = BufReader::new(&mut agent.read);
        let mut received = String::new();
        agent_reader.read_line(&mut received).await.unwrap();
        let parsed: Value = serde_json::from_str(received.trim_end_matches('\n')).unwrap();
        assert_eq!(parsed["id"], json!(3));
        assert_eq!(parsed["error"]["code"], JSONRPC_INVALID_REQUEST);
        assert!(parsed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("missing params.name"));

        let mut probe = vec![0u8; 256];
        let read = tokio::time::timeout(
            std::time::Duration::from_millis(150),
            child.read.read(&mut probe),
        )
        .await;
        assert!(
            read.is_err(),
            "child must not receive fail-closed frame; got {read:?}"
        );

        drop(agent.write);
        drop(agent.read);
        drop(child.write);
        drop(child.read);
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn tools_call_notification_form_is_dropped() {
        let filter = allow_filter(&["getJiraIssue"]);
        let (handle, mut agent, mut child) = spawn_proxy_pair(filter);

        // Notification form (no `id`).
        write_json(
            &mut agent.write,
            r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"deleteEverything"}}"#,
        )
        .await;

        // Agent receives nothing: read times out.
        let mut probe = vec![0u8; 256];
        let read = tokio::time::timeout(
            std::time::Duration::from_millis(150),
            agent.read.read(&mut probe),
        )
        .await;
        assert!(
            read.is_err(),
            "agent must not receive a response to a notification; got {read:?}"
        );
        let read = tokio::time::timeout(
            std::time::Duration::from_millis(150),
            child.read.read(&mut probe),
        )
        .await;
        assert!(
            read.is_err(),
            "child must not receive the dropped notification; got {read:?}"
        );

        drop(agent.write);
        drop(agent.read);
        drop(child.write);
        drop(child.read);
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn initialize_is_byte_identical_passthrough() {
        let filter = allow_filter(&["getJiraIssue"]);
        let (handle, mut agent, mut child) = spawn_proxy_pair(filter);

        let init_req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{}}}"#;
        write_json(&mut agent.write, init_req).await;

        let mut child_reader = BufReader::new(&mut child.read);
        let mut received = String::new();
        child_reader.read_line(&mut received).await.unwrap();
        assert_eq!(received.trim_end_matches('\n'), init_req);

        let init_resp = r#"{"jsonrpc":"2.0","id":1,"result":{"serverInfo":{"name":"x"}}}"#;
        write_json(&mut child.write, init_resp).await;

        let mut agent_reader = BufReader::new(&mut agent.read);
        let mut received = String::new();
        agent_reader.read_line(&mut received).await.unwrap();
        assert_eq!(received.trim_end_matches('\n'), init_resp);

        drop(agent.write);
        drop(agent.read);
        drop(child.write);
        drop(child.read);
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn unknown_method_is_passthrough() {
        let filter = allow_filter(&["getJiraIssue"]);
        let (handle, mut agent, mut child) = spawn_proxy_pair(filter);

        let req = r#"{"jsonrpc":"2.0","id":9,"method":"resources/read","params":{"uri":"x"}}"#;
        write_json(&mut agent.write, req).await;
        let mut child_reader = BufReader::new(&mut child.read);
        let mut received = String::new();
        child_reader.read_line(&mut received).await.unwrap();
        assert_eq!(received.trim_end_matches('\n'), req);

        drop(agent.write);
        drop(agent.read);
        drop(child.write);
        drop(child.read);
        let _ = handle.await.unwrap();
    }

    #[tokio::test]
    async fn child_death_propagates_as_error() {
        let filter = allow_filter(&["getJiraIssue"]);
        let (handle, _agent, child) = spawn_proxy_pair(filter);

        // Simulate child death.
        drop(child.write);
        drop(child.read);

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle)
            .await
            .expect("proxy must exit within 2s of child death")
            .unwrap();
        assert!(result.is_err(), "proxy must surface child death as error");
    }

    #[tokio::test]
    async fn strict_allow_unknown_name_fails_after_catalog() {
        let filter = FilterState::from_filter(
            "test",
            &ToolFilter {
                allow: Some(vec!["doesNotExist".into(), "getJiraIssue".into()]),
                deny: None,
                strict_allow: true,
            },
        );
        let (handle, mut agent, mut child) = spawn_proxy_pair(filter);

        write_json(
            &mut agent.write,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .await;
        let mut child_reader = BufReader::new(&mut child.read);
        let mut discard = String::new();
        let _ = child_reader.read_line(&mut discard).await;

        write_json(
            &mut child.write,
            r#"{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"getJiraIssue"}]}}"#,
        )
        .await;

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle)
            .await
            .expect("proxy must exit on strict_allow violation")
            .unwrap();
        assert!(result.is_err());
        let err = format!("{:#}", result.unwrap_err());
        assert!(err.contains("strict_allow"));
        assert!(err.contains("doesNotExist"));
    }

    #[tokio::test]
    async fn malformed_child_frame_propagates_as_error() {
        let filter = allow_filter(&["getJiraIssue"]);
        let (handle, mut agent, mut child) = spawn_proxy_pair(filter);

        // First establish a tools/list request so the response is "in scope".
        write_json(
            &mut agent.write,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
        )
        .await;
        let mut child_reader = BufReader::new(&mut child.read);
        let mut discard = String::new();
        let _ = child_reader.read_line(&mut discard).await;

        // Now send invalid JSON on the response.
        write_json(&mut child.write, "this is not json").await;

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle)
            .await
            .expect("proxy must exit on malformed child frame")
            .unwrap();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn permits_allow_and_deny_semantics() {
        let allow = FilterState::from_filter(
            "x",
            &ToolFilter {
                allow: Some(vec!["a".into()]),
                deny: None,
                strict_allow: false,
            },
        );
        assert!(allow.permits("a"));
        assert!(!allow.permits("b"));

        let deny = FilterState::from_filter(
            "x",
            &ToolFilter {
                allow: None,
                deny: Some(vec!["b".into()]),
                strict_allow: false,
            },
        );
        assert!(deny.permits("a"));
        assert!(!deny.permits("b"));
    }
}
