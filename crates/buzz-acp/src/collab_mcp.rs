//! Collaboration MCP sidecar. Spawned by `buzz-acp collab-mcp` with a host HMAC
//! key that is never given to `buzz-dev-mcp` or the agent shell.
//!
//! Tools are dispatched into the fusion-layer Python kernels after envelope
//! verification. Identity fields are not accepted from the model.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler, ServiceExt};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::collab_context::{
    verify_payload, CollabRole, RecipientBinding, SignedEnvelope, TurnClaim, AGENT_AUTHORITY_ENV,
    AGENT_PRINCIPAL_ENV, HELPER_ROOT_ENV, HMAC_KEY_ENV, TURN_PATH_ENV,
};

const HOST_RESERVED_FIELDS: &[&str] = &[
    "role",
    "agent_role",
    "principal",
    "principal_id",
    "agent_principal_id",
    "user_principal_id",
    "project_id",
    "channel_id",
    "session_id",
    "root_event_id",
    "event_id",
    "recipient",
    "recipient_principal_id",
    "authority",
    "runtime_verified",
];

#[derive(Clone)]
struct CollabMcp {
    tool_router: rmcp::handler::server::router::tool::ToolRouter<CollabMcp>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema, Default)]
struct EmptyArgs {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct JsonArgs {
    #[serde(flatten)]
    extra: HashMap<String, Value>,
}

fn hex_key() -> Result<Vec<u8>, ErrorData> {
    let hex = std::env::var(HMAC_KEY_ENV).map_err(|_| {
        ErrorData::internal_error("collaboration HMAC key is not configured", None)
    })?;
    hex::decode(hex.trim()).map_err(|_| {
        ErrorData::internal_error("collaboration HMAC key is not valid hex", None)
    })
}

fn load_verified_claim() -> Result<(Vec<u8>, SignedEnvelope, TurnClaim), ErrorData> {
    let key = hex_key()?;
    let path = std::env::var(TURN_PATH_ENV).map_err(|_| {
        ErrorData::internal_error("collaboration turn path is not configured", None)
    })?;
    let raw = std::fs::read(&path).map_err(|_| {
        ErrorData::internal_error("collaboration turn context is missing", None)
    })?;
    let envelope: SignedEnvelope = serde_json::from_slice(&raw).map_err(|_| {
        ErrorData::internal_error("collaboration turn context is invalid", None)
    })?;
    let payload = envelope
        .payload
        .as_object()
        .ok_or_else(|| ErrorData::internal_error("collaboration payload missing", None))?;
    let agent_principal = std::env::var(AGENT_PRINCIPAL_ENV).unwrap_or_default();
    let agent_authority = std::env::var(AGENT_AUTHORITY_ENV).unwrap_or_default();
    let claim = TurnClaim {
        project_id: payload_str(payload, "project_id")?,
        agent_principal_id: payload_str(payload, "agent_principal_id")?,
        agent_authority: payload_str(payload, "agent_authority")?,
        agent_role: CollabRole::parse(&payload_str(payload, "agent_role")?),
        channel_id: payload_str(payload, "channel_id")?,
        session_id: payload_str(payload, "session_id")?,
        root_event_id: payload_str(payload, "root_event_id")?,
        event_id: payload_str(payload, "event_id")?,
        user_principal_id: payload_str(payload, "user_principal_id")?,
        event_time: payload_str(payload, "event_time")?,
        statement: payload_str(payload, "statement")?,
        recipient: payload.get("recipient").and_then(|value| {
            let object = value.as_object()?;
            Some(RecipientBinding {
                principal_id: object.get("principal_id")?.as_str()?.to_string(),
                authority: object.get("authority")?.as_str()?.to_string(),
            })
        }),
    };
    if claim.agent_principal_id != agent_principal || claim.agent_authority != agent_authority {
        return Err(ErrorData::invalid_params(
            "宿主事件与当前调用边界不匹配：agent_principal_id",
            None,
        ));
    }
    verify_payload(&key, &envelope, &claim).map_err(|error| {
        ErrorData::invalid_params(error, None)
    })?;
    Ok((key, envelope, claim))
}

fn payload_str(
    payload: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<String, ErrorData> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| ErrorData::internal_error(format!("missing {field}"), None))
}

fn reject_reserved(arguments: &Value) -> Result<(), ErrorData> {
    let Some(object) = arguments.as_object() else {
        return Ok(());
    };
    for key in object.keys() {
        if HOST_RESERVED_FIELDS
            .iter()
            .any(|field| field.eq_ignore_ascii_case(key))
        {
            return Err(ErrorData::invalid_params(
                format!("工具参数包含宿主保留或未知字段：{key}"),
                None,
            ));
        }
    }
    Ok(())
}

fn envelope_allows(envelope: &SignedEnvelope, tool_name: &str) -> Result<(), ErrorData> {
    let tools = envelope.payload.get("tools").and_then(Value::as_array);
    let allowed = tools
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .any(|item| item == tool_name)
        })
        .unwrap_or(false);
    if allowed {
        Ok(())
    } else {
        Err(ErrorData::invalid_params(
            format!("当前宿主身份未获工具 capability：{tool_name}"),
            None,
        ))
    }
}

fn invoke_python(tool_name: &str, arguments: Value) -> Result<CallToolResult, ErrorData> {
    reject_reserved(&arguments)?;
    let (_key, envelope, _claim) = load_verified_claim()?;
    envelope_allows(&envelope, tool_name)?;
    let helper = PathBuf::from(std::env::var(HELPER_ROOT_ENV).map_err(|_| {
        ErrorData::internal_error("collaboration helper root is not configured", None)
    })?);
    let script = helper.join("scripts/collab_invoke.py");
    let mut child = Command::new("python3")
        .arg("-B")
        .arg(&script)
        .arg(tool_name)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("PYTHONPATH", helper.join("scripts"))
        .spawn()
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(arguments.to_string().as_bytes())
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    }
    let mut stdout = String::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_string(&mut stdout)
            .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    }
    let mut stderr = String::new();
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_string(&mut stderr);
    }
    let status = child
        .wait()
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    if !status.success() {
        return Err(ErrorData::internal_error(
            if stderr.trim().is_empty() {
                stdout
            } else {
                stderr
            },
            None,
        ));
    }
    Ok(CallToolResult::success(vec![Content::text(stdout)]))
}

#[tool_router]
impl CollabMcp {
    fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(name = "artifact.inspect", description = "Read-only artifact registry inspect")]
    async fn artifact_inspect(
        &self,
        Parameters(_args): Parameters<EmptyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        invoke_python("artifact.inspect", json!({}))
    }

    #[tool(
        name = "artifact.preview-register",
        description = "Preview artifact registration without writing"
    )]
    async fn artifact_preview(
        &self,
        Parameters(args): Parameters<JsonArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        invoke_python("artifact.preview-register", Value::Object(args.extra.into_iter().collect()))
    }

    #[tool(name = "artifact.register", description = "Main-only artifact registration")]
    async fn artifact_register(
        &self,
        Parameters(args): Parameters<JsonArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        invoke_python(
            "artifact.register",
            Value::Object(args.extra.into_iter().collect()),
        )
    }

    #[tool(name = "collaboration.inspect", description = "Read-only confirmation and handoff inspect")]
    async fn collaboration_inspect(
        &self,
        Parameters(_args): Parameters<EmptyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        invoke_python("collaboration.inspect", json!({}))
    }

    #[tool(
        name = "confirmation.confirm-current",
        description = "Main-only current-session confirmation"
    )]
    async fn confirm_current(
        &self,
        Parameters(args): Parameters<JsonArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        invoke_python(
            "confirmation.confirm-current",
            Value::Object(args.extra.into_iter().collect()),
        )
    }

    #[tool(name = "handoff.prepare", description = "Main-only handoff preparation")]
    async fn handoff_prepare(
        &self,
        Parameters(args): Parameters<JsonArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        invoke_python(
            "handoff.prepare",
            Value::Object(args.extra.into_iter().collect()),
        )
    }

    #[tool(name = "handoff.record-sent", description = "Main-only trusted send recording")]
    async fn handoff_sent(
        &self,
        Parameters(args): Parameters<JsonArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        invoke_python(
            "handoff.record-sent",
            Value::Object(args.extra.into_iter().collect()),
        )
    }

    #[tool(name = "handoff.verify-receipt", description = "Recipient-only read receipt")]
    async fn handoff_receipt(
        &self,
        Parameters(args): Parameters<JsonArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        invoke_python(
            "handoff.verify-receipt",
            Value::Object(args.extra.into_iter().collect()),
        )
    }

    #[tool(name = "requirements.context", description = "Requirements background read")]
    async fn requirements_context(
        &self,
        Parameters(_args): Parameters<EmptyArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        invoke_python("requirements.context", json!({}))
    }

    #[tool(name = "requirements.save-draft", description = "Requirements draft save")]
    async fn requirements_save(
        &self,
        Parameters(args): Parameters<JsonArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        invoke_python(
            "requirements.save-draft",
            Value::Object(args.extra.into_iter().collect()),
        )
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for CollabMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "buzz-collab-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
    }
}

pub async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .compact()
        .init();
    let service = CollabMcp::new().serve(rmcp::transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collab_context::{sign_payload, HMAC_KEY_ENV};
    use serde_json::json;

    #[test]
    fn rejects_identity_fields_in_tool_arguments() {
        for field in ["role", "project_id", "user_principal_id", "runtime_verified"] {
            let mut object = serde_json::Map::new();
            object.insert(field.to_string(), json!("forged"));
            let error = reject_reserved(&Value::Object(object)).unwrap_err();
            assert!(error.message.contains("宿主保留"));
        }
    }

    #[test]
    fn hmac_env_is_not_the_agent_private_key() {
        assert_eq!(HMAC_KEY_ENV, "BUZZ_COLLAB_HMAC_KEY");
        assert_ne!(HMAC_KEY_ENV, "BUZZ_PRIVATE_KEY");
    }

    #[test]
    fn envelope_tool_ceiling_is_enforced() {
        let envelope = sign_payload(
            b"unit-05-host-context-verification-key",
            &json!({"tools": ["artifact.inspect"]}),
        )
        .unwrap();
        assert!(envelope_allows(&envelope, "artifact.inspect").is_ok());
        assert!(envelope_allows(&envelope, "artifact.register").is_err());
    }
}
