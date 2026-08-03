use crate::{collaboration, managed_files, managed_git, managed_instructions, managed_jobs};
use buzz_runtime::{AssignmentSetStateRequest, AssignmentState, ClientError, RuntimeClient};
use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router, ErrorData, ServerHandler,
};
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ManagedAssignmentState {
    Reading,
    Working,
    Waiting,
    NeedsApproval,
    Blocked,
    Recovering,
    Completed,
    Failed,
    Cancelled,
}

impl From<ManagedAssignmentState> for AssignmentState {
    fn from(value: ManagedAssignmentState) -> Self {
        match value {
            ManagedAssignmentState::Reading => Self::Reading,
            ManagedAssignmentState::Working => Self::Working,
            ManagedAssignmentState::Waiting => Self::Waiting,
            ManagedAssignmentState::NeedsApproval => Self::NeedsApproval,
            ManagedAssignmentState::Blocked => Self::Blocked,
            ManagedAssignmentState::Recovering => Self::Recovering,
            ManagedAssignmentState::Completed => Self::Completed,
            ManagedAssignmentState::Failed => Self::Failed,
            ManagedAssignmentState::Cancelled => Self::Cancelled,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AssignmentSetStateParams {
    assignment_id: String,
    state: ManagedAssignmentState,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    blocker: Option<String>,
    #[serde(default)]
    approval_gate_id: Option<String>,
    #[serde(default)]
    delivery_evidence: Option<String>,
    #[serde(default)]
    reply_event_id: Option<String>,
}

#[derive(Clone)]
pub(crate) struct ManagedMcp {
    root: Arc<PathBuf>,
    runtime: RuntimeClient,
    collaboration: collaboration::CollaborationClient,
    tool_router: ToolRouter<ManagedMcp>,
}

#[tool_router]
impl ManagedMcp {
    pub(crate) fn new(root: PathBuf, runtime: RuntimeClient) -> Result<Self, ErrorData> {
        let receipt_path = std::env::var_os("BUZZ_RUNTIME_RECEIPT").ok_or_else(|| {
            ErrorData::invalid_request("managed runtime receipt is unavailable", None)
        })?;
        let receipt = buzz_runtime::read_runtime_receipt(PathBuf::from(receipt_path).as_path())
            .map_err(|_| {
                ErrorData::invalid_request("managed runtime receipt is unavailable", None)
            })?;
        let collaboration = collaboration::CollaborationClient::from_env(
            &receipt.key.pubkey,
            &receipt.key.relay_url,
        )?;
        Self::new_with_collaboration(root, runtime, collaboration)
    }

    fn new_with_collaboration(
        root: PathBuf,
        runtime: RuntimeClient,
        collaboration: collaboration::CollaborationClient,
    ) -> Result<Self, ErrorData> {
        Ok(Self {
            root: Arc::new(managed_files::canonical_root(root)?),
            runtime,
            collaboration,
            tool_router: Self::tool_router(),
        })
    }

    #[tool(
        name = "assignment_set_state",
        description = "Update the current durable assignment through the authenticated model capability. Waiting, blocked, approval, and completion states require their structured evidence; terminal assignments cannot reopen."
    )]
    async fn assignment_set_state(
        &self,
        Parameters(params): Parameters<AssignmentSetStateParams>,
    ) -> Result<String, ErrorData> {
        if params.assignment_id.trim().is_empty() {
            return Err(ErrorData::invalid_params(
                "assignment_id must be non-empty",
                Some(serde_json::json!({"code": "invalid_assignment_id"})),
            ));
        }
        let record = self
            .runtime
            .assignment_set_state(
                params.assignment_id,
                AssignmentSetStateRequest {
                    state: params.state.into(),
                    summary: params.summary,
                    reason: params.reason,
                    blocker: params.blocker,
                    approval_gate_id: params.approval_gate_id,
                    delivery_evidence: params.delivery_evidence,
                    reply_event_id: params.reply_event_id,
                },
            )
            .await
            .map_err(assignment_error)?;
        serde_json::to_string(&record)
            .map_err(|_| ErrorData::internal_error("cannot encode assignment response", None))
    }

    #[tool(
        name = "messages_send",
        description = "Send a signed channel-scoped message as the current managed identity. Membership, bounded mentions, and NIP-10 reply anchors are checked before publication."
    )]
    async fn messages_send(
        &self,
        Parameters(params): Parameters<collaboration::MessagesSendParams>,
    ) -> Result<String, ErrorData> {
        self.collaboration.messages_send(params).await
    }

    #[tool(
        name = "messages_get",
        description = "Read bounded signed messages from a channel where the current managed identity is a member. An optional since event ID acts as a scoped cursor."
    )]
    async fn messages_get(
        &self,
        Parameters(params): Parameters<collaboration::MessagesGetParams>,
    ) -> Result<String, ErrorData> {
        self.collaboration.messages_get(params).await
    }

    #[tool(
        name = "messages_thread",
        description = "Read a bounded signed NIP-10 thread rooted in a channel shared by the current managed identity."
    )]
    async fn messages_thread(
        &self,
        Parameters(params): Parameters<collaboration::MessagesThreadParams>,
    ) -> Result<String, ErrorData> {
        self.collaboration.messages_thread(params).await
    }

    #[tool(
        name = "messages_search",
        description = "Search bounded signed messages only across channels shared by the current managed identity, or within one explicitly shared channel."
    )]
    async fn messages_search(
        &self,
        Parameters(params): Parameters<collaboration::MessagesSearchParams>,
    ) -> Result<String, ErrorData> {
        self.collaboration.messages_search(params).await
    }

    #[tool(
        name = "agents_list",
        description = "List bounded managed-coworker summaries from a shared channel or all bounded shared channel scopes."
    )]
    async fn agents_list(
        &self,
        Parameters(params): Parameters<collaboration::AgentsListParams>,
    ) -> Result<String, ErrorData> {
        self.collaboration.agents_list(params).await
    }

    #[tool(
        name = "agents_status",
        description = "Read one managed coworker's current presence and NIP-38 work status after verifying a shared channel."
    )]
    async fn agents_status(
        &self,
        Parameters(params): Parameters<collaboration::AgentsStatusParams>,
    ) -> Result<String, ErrorData> {
        self.collaboration.agents_status(params).await
    }

    #[tool(
        name = "files_read",
        description = "Read a bounded UTF-8 text-file window inside the managed workspace. Paths are canonicalized and traversal or symlink escape is rejected. This tool never writes files."
    )]
    async fn files_read(
        &self,
        Parameters(params): Parameters<managed_files::FilesReadParams>,
    ) -> Result<String, ErrorData> {
        managed_files::files_read(self.root.as_path(), params)
    }

    #[tool(
        name = "files_list",
        description = "List a bounded number of immediate entries inside the managed workspace. Paths are canonicalized; links are identified but never followed outside the workspace."
    )]
    async fn files_list(
        &self,
        Parameters(params): Parameters<managed_files::FilesListParams>,
    ) -> Result<String, ErrorData> {
        managed_files::files_list(self.root.as_path(), params)
    }

    #[tool(
        name = "search_text",
        description = "Search bounded workspace files for literal text in process. The search never invokes a shell or executable and never follows symlinks."
    )]
    async fn search_text(
        &self,
        Parameters(params): Parameters<managed_files::SearchTextParams>,
    ) -> Result<String, ErrorData> {
        managed_files::search_text(self.root.as_path(), params)
    }

    #[tool(
        name = "git_status",
        description = "Inspect bounded Git working-tree status inside the canonical managed workspace. This fixed read-only operation accepts only an optional contained path scope and cannot run user-supplied Git options."
    )]
    async fn git_status(
        &self,
        Parameters(params): Parameters<managed_git::GitStatusParams>,
    ) -> Result<String, ErrorData> {
        managed_git::git_status(self.root.as_path(), params).await
    }

    #[tool(
        name = "git_diff",
        description = "Inspect a bounded unstaged Git diff inside the canonical managed workspace. External diff and text-conversion commands are disabled; no shell, arbitrary Git option, or mutation is available."
    )]
    async fn git_diff(
        &self,
        Parameters(params): Parameters<managed_git::GitDiffParams>,
    ) -> Result<String, ErrorData> {
        managed_git::git_diff(self.root.as_path(), params).await
    }

    #[tool(
        name = "jobs_start",
        description = "Request governed Legacy Harness work for this agent or another shared managed agent. Local/self targets use the authenticated runtime; a different target publishes a signed job request and never spawns or controls the remote runtime."
    )]
    async fn jobs_start(
        &self,
        Parameters(params): Parameters<managed_jobs::JobsStartParams>,
    ) -> Result<String, ErrorData> {
        managed_jobs::jobs_start(&self.runtime, &self.collaboration, params).await
    }

    #[tool(
        name = "jobs_status",
        description = "Read durable local status for a managed job UUID."
    )]
    async fn jobs_status(
        &self,
        Parameters(params): Parameters<managed_jobs::JobIdParams>,
    ) -> Result<String, ErrorData> {
        managed_jobs::jobs_status(&self.runtime, params).await
    }

    #[tool(
        name = "jobs_logs",
        description = "Read a bounded owner-local tail of a managed job's logs. Defaults to 100 lines and is capped at 1,000 lines by the runtime."
    )]
    async fn jobs_logs(
        &self,
        Parameters(params): Parameters<managed_jobs::JobsLogsParams>,
    ) -> Result<String, ErrorData> {
        managed_jobs::jobs_logs(&self.runtime, params).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for ManagedMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "buzz-managed-mcp",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(managed_instructions::MANAGED_INSTRUCTIONS)
    }
}

fn assignment_error(error: ClientError) -> ErrorData {
    match error {
        ClientError::Remote { code, message } => {
            ErrorData::invalid_params(message, Some(serde_json::json!({"code": code})))
        }
        ClientError::InvalidRequest(message) => ErrorData::invalid_params(
            message,
            Some(serde_json::json!({"code": "invalid_assignment_request"})),
        ),
        _ => ErrorData::internal_error(
            "managed assignment request failed",
            Some(serde_json::json!({"code": "assignment_runtime_unavailable"})),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_manifest_is_exact_allowlist() {
        let names = ManagedMcp::tool_router()
            .list_all()
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "agents_list",
                "agents_status",
                "assignment_set_state",
                "files_list",
                "files_read",
                "git_diff",
                "git_status",
                "jobs_logs",
                "jobs_start",
                "jobs_status",
                "messages_get",
                "messages_search",
                "messages_send",
                "messages_thread",
                "search_text",
            ]
        );
        for forbidden in [
            "shell",
            "str_replace",
            "todo",
            "_Stop",
            "_PostCompact",
            "shutdown",
            "lh",
            "files_write",
            "git_add",
            "git_apply",
            "git_commit",
        ] {
            assert!(!ManagedMcp::tool_router().map.contains_key(forbidden));
        }
    }

    #[tokio::test]
    async fn tools_list_returns_only_managed_allowlist() {
        use buzz_runtime::{
            ControlError, ControlHandlerFn, ControlServerConfig, RuntimeServer,
            CONTROL_PROTOCOL_VERSION,
        };
        use rmcp::ServiceExt as _;

        let generation = uuid::Uuid::from_u128(1);
        let config = ControlServerConfig::new("managed-test".into(), generation);
        let server = RuntimeServer::bind(config.clone()).await.unwrap();
        let address = server.local_addr().unwrap();
        let handler = Arc::new(ControlHandlerFn(|_, _| async {
            Err::<buzz_runtime::ControlPayload, _>(ControlError::new("unused", "unused"))
        }));
        let control_task = tokio::spawn(server.serve(handler));

        let receipt = tempfile::NamedTempFile::new().unwrap();
        let process_marker = buzz_runtime::current_process_start_marker().unwrap();
        let mut receipt_json = serde_json::json!({
            "schemaVersion": 2,
            "key": {
                "pubkey": "0".repeat(64),
                "relayUrl": "wss://relay.invalid"
            },
            "runtimeId": "managed-test",
            "pid": std::process::id(),
            "processStartMarker": process_marker,
            "generation": generation,
            "controlAddr": address,
            "controllerToken": config.controller_token,
            "modelToken": config.model_token,
            "startedAt": "2026-01-01T00:00:00Z",
            "protocolVersion": CONTROL_PROTOCOL_VERSION,
            "lockProtocolVersion": 1,
            "lockPathHash": "b".repeat(64),
            "ready": true
        });
        let model_token = receipt_json["modelToken"].clone();
        let mut forged_model_token = model_token.as_str().unwrap().to_owned();
        let replacement = if forged_model_token.starts_with('0') {
            "1"
        } else {
            "0"
        };
        forged_model_token.replace_range(0..1, replacement);
        receipt_json["modelToken"] = serde_json::json!(forged_model_token);
        std::fs::write(receipt.path(), serde_json::to_vec(&receipt_json).unwrap()).unwrap();
        assert!(
            RuntimeClient::from_receipt(receipt.path(), buzz_runtime::Capability::Model)
                .await
                .is_err()
        );
        receipt_json["modelToken"] = model_token;
        std::fs::write(receipt.path(), serde_json::to_vec(&receipt_json).unwrap()).unwrap();
        let runtime = RuntimeClient::from_receipt(receipt.path(), buzz_runtime::Capability::Model)
            .await
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let managed = ManagedMcp::new_with_collaboration(
            root.path().to_owned(),
            runtime,
            collaboration::CollaborationClient::unavailable_for_test(),
        )
        .unwrap();

        let (server_transport, client_transport) = tokio::io::duplex(16 * 1024);
        let mcp_task = tokio::spawn(async move {
            let service = managed.serve(server_transport).await.unwrap();
            service.waiting().await.unwrap();
        });
        let client = ().serve(client_transport).await.unwrap();
        let names = client
            .peer()
            .list_all_tools()
            .await
            .unwrap()
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "agents_list",
                "agents_status",
                "assignment_set_state",
                "files_list",
                "files_read",
                "git_diff",
                "git_status",
                "jobs_logs",
                "jobs_start",
                "jobs_status",
                "messages_get",
                "messages_search",
                "messages_send",
                "messages_thread",
                "search_text",
            ]
        );

        client.cancel().await.unwrap();
        mcp_task.abort();
        control_task.abort();
    }

    #[test]
    fn assignment_transition_error_preserves_typed_runtime_code() {
        let error = assignment_error(ClientError::Remote {
            code: "invalid_assignment_transition".into(),
            message: "terminal assignment cannot reopen".into(),
        });
        assert_eq!(
            error.data.as_ref().and_then(|data| data.get("code")),
            Some(&serde_json::json!("invalid_assignment_transition"))
        );
    }
}
