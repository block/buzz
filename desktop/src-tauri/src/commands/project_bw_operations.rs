//! Thin Tauri I/O adapter for the shared BW issue business operations.

use super::project_bw::load as load_bw_input;
use super::project_bw_git::{
    genesis_clone_url, resolve_stream_head_blocking, validate_stream_for_git,
};
use super::project_git_exec::{build_git_auth_config, validate_workspace_clone_url};
use super::project_git_workflow::project_owner_identity;
use crate::app_state::AppState;
use crate::bw_projection::{self, Input};
use crate::relay::{query_relay, submit_signed_event_with_keys};
use buzz_sdk_pkg::bw::operations::{
    self, AcceptToBacklogRequest, AssignWriterRequest, IssueRequest, MarkImplementedRequest,
    MoveToReadyRequest, OperationOutcome, OperationPort, PortFuture, RawHistory, SigningIdentity,
};
use nostr::{Event, EventBuilder, Kind, Keys, Tag, Timestamp};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tauri::{AppHandle, State};

#[derive(Clone)]
struct StoredIdentity {
    keys: Keys,
    auth_tag: Option<String>,
}

struct DesktopOperationPort<'a> {
    app: &'a AppHandle,
    state: &'a AppState,
    identities: BTreeMap<String, StoredIdentity>,
    latest_history: Option<Input>,
}

impl<'a> DesktopOperationPort<'a> {
    fn new(app: &'a AppHandle, state: &'a AppState) -> Self {
        Self {
            app,
            state,
            identities: BTreeMap::new(),
            latest_history: None,
        }
    }

    fn resolve_identity(&mut self, required: Option<&str>) -> Result<StoredIdentity, String> {
        if let Some(required) = required {
            let required = required.to_ascii_lowercase();
            if let Some(identity) = self.identities.get(&required) {
                return Ok(identity.clone());
            }
            let identity = project_owner_identity(self.app, self.state, &required)?;
            let stored = StoredIdentity {
                keys: identity.keys,
                auth_tag: identity.auth_tag,
            };
            self.identities.insert(required, stored.clone());
            return Ok(stored);
        }

        let keys = self.state.signing_keys()?;
        let pubkey = keys.public_key().to_hex();
        let stored = StoredIdentity {
            keys,
            auth_tag: None,
        };
        self.identities.insert(pubkey, stored.clone());
        Ok(stored)
    }
}

impl OperationPort for DesktopOperationPort<'_> {
    type Error = String;

    fn now(&mut self) -> Result<u64, Self::Error> {
        Ok(Timestamp::now().as_secs())
    }

    fn load_history<'a>(
        &'a mut self,
        repo: &'a str,
    ) -> PortFuture<'a, RawHistory, Self::Error> {
        Box::pin(async move {
            let input = load_bw_input(self.state, repo).await?;
            let history = RawHistory {
                trust: input.trust.clone(),
                evidence: input.external.clone(),
                now: input.now,
                events: input
                    .events
                    .iter()
                    .map(|event| event.as_bytes().to_vec())
                    .collect(),
            };
            self.latest_history = Some(input);
            Ok(history)
        })
    }

    fn signing_identity<'a>(
        &'a mut self,
        required_pubkey: Option<&'a str>,
    ) -> PortFuture<'a, SigningIdentity, Self::Error> {
        Box::pin(async move {
            let identity = self.resolve_identity(required_pubkey)?;
            Ok(SigningIdentity {
                pubkey: identity.keys.public_key().to_hex(),
                event_tags: Vec::new(),
            })
        })
    }

    fn sign<'a>(
        &'a mut self,
        candidate: &'a Value,
        required_pubkey: Option<&'a str>,
    ) -> PortFuture<'a, Event, Self::Error> {
        Box::pin(async move {
            let identity = self.resolve_identity(required_pubkey)?;
            if candidate["pubkey"].as_str() != Some(identity.keys.public_key().to_hex().as_str()) {
                return Err("BW candidate signer does not match the selected identity".to_string());
            }
            let tags = candidate["tags"]
                .as_array()
                .ok_or_else(|| "bw:shape:tags".to_string())?
                .iter()
                .map(|tag| {
                    let fields = tag
                        .as_array()
                        .ok_or_else(|| "bw:shape:tags".to_string())?
                        .iter()
                        .map(|field| {
                            field
                                .as_str()
                                .ok_or_else(|| "bw:shape:tags".to_string())
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Tag::parse(fields).map_err(|error| error.to_string())
                })
                .collect::<Result<Vec<_>, _>>()?;
            let kind = candidate["kind"]
                .as_u64()
                .and_then(|kind| u16::try_from(kind).ok())
                .ok_or_else(|| "bw:shape:kind".to_string())?;
            let content = candidate["content"]
                .as_str()
                .ok_or_else(|| "bw:shape:content".to_string())?;
            let created_at = candidate["created_at"]
                .as_u64()
                .ok_or_else(|| "bw:shape:created-at".to_string())?;
            EventBuilder::new(Kind::Custom(kind), content)
                .tags(tags)
                .custom_created_at(Timestamp::from(created_at))
                .sign_with_keys(&identity.keys)
                .map_err(|error| format!("sign bw operation: {error}"))
        })
    }

    fn submit<'a>(&'a mut self, event: &'a Event) -> PortFuture<'a, (), Self::Error> {
        Box::pin(async move {
            let identity = self
                .identities
                .get(&event.pubkey.to_hex())
                .cloned()
                .ok_or_else(|| "BW publishing identity was not resolved".to_string())?;
            submit_signed_event_with_keys(
                event,
                self.state,
                &identity.keys,
                identity.auth_tag.as_deref(),
            )
            .await
            .map(|_| ())
        })
    }

    fn readback<'a>(
        &'a mut self,
        id: &'a str,
    ) -> PortFuture<'a, Option<Event>, Self::Error> {
        Box::pin(async move {
            Ok(query_relay(self.state, &[json!({"ids":[id],"limit":1})])
                .await?
                .into_iter()
                .find(|event| event.id.to_hex() == id))
        })
    }

    fn remote_head<'a>(
        &'a mut self,
        repo: &'a str,
        stream: &'a str,
    ) -> PortFuture<'a, String, Self::Error> {
        Box::pin(async move {
            validate_stream_for_git(stream)?;
            let input = self
                .latest_history
                .as_ref()
                .filter(|input| input.trust.repo == repo)
                .ok_or_else(|| "BW history must be loaded before Git readback".to_string())?;
            let (consumer, _) = bw_projection::replay(input);
            let activation = consumer.activation().map_err(|error| error.to_string())?;
            let clone_url = genesis_clone_url(input, &activation.genesis)?;
            validate_workspace_clone_url(&clone_url, self.state)?;
            let auth = build_git_auth_config(self.state)?;
            let stream = stream.to_owned();
            tauri::async_runtime::spawn_blocking(move || {
                resolve_stream_head_blocking(&clone_url, &stream, &auth)
            })
            .await
            .map_err(|error| format!("git readback task failed: {error}"))?
        })
    }
}

fn outcome_json(outcome: OperationOutcome) -> Value {
    let event_id = outcome
        .steps
        .iter()
        .rev()
        .find_map(|step| step.event_id.clone());
    let steps = outcome
        .steps
        .into_iter()
        .map(|step| {
            json!({
                "name":step.name,
                "eventId":step.event_id,
                "status":step.status.as_str(),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "eventId":event_id,
        "issue":outcome.issue,
        "alreadyCompleted":outcome.already_completed,
        "steps":steps,
        "projection":outcome.projection,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueOperationInput {
    repo: String,
    issue_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptOperationInput {
    repo: String,
    issue_id: String,
    #[serde(default)]
    delegated: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignWriterOperationInput {
    repo: String,
    issue_id: String,
    writer: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveToReadyOperationInput {
    repo: String,
    issue_id: String,
    stream: String,
    #[serde(default)]
    rework_verdict_id: Option<String>,
    #[serde(default)]
    terminal_set_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkImplementedOperationInput {
    repo: String,
    issue_id: String,
    #[serde(default)]
    commit: Option<String>,
    tests: String,
}

#[tauri::command]
pub async fn enroll_project_bw_issue(
    input: IssueOperationInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut port = DesktopOperationPort::new(&app, &state);
    operations::enroll_issue(&mut port, IssueRequest::new(input.repo, input.issue_id))
        .await
        .map(outcome_json)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn accept_project_bw_issue(
    input: AcceptOperationInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut port = DesktopOperationPort::new(&app, &state);
    operations::accept_to_backlog(
        &mut port,
        AcceptToBacklogRequest {
            repo: input.repo,
            issue: input.issue_id,
            delegated: input.delegated,
        },
    )
    .await
    .map(outcome_json)
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn assign_project_bw_writer(
    input: AssignWriterOperationInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut port = DesktopOperationPort::new(&app, &state);
    operations::assign_writer(
        &mut port,
        AssignWriterRequest {
            repo: input.repo,
            issue: input.issue_id,
            writer: input.writer,
        },
    )
    .await
    .map(outcome_json)
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn move_project_bw_issue_to_ready(
    input: MoveToReadyOperationInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut port = DesktopOperationPort::new(&app, &state);
    operations::move_to_ready(
        &mut port,
        MoveToReadyRequest {
            repo: input.repo,
            issue: input.issue_id,
            stream: input.stream,
            rework: input.rework_verdict_id,
            terminal_set: input.terminal_set_id,
        },
    )
    .await
    .map(outcome_json)
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn start_project_bw_development(
    input: IssueOperationInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut port = DesktopOperationPort::new(&app, &state);
    operations::start_development(&mut port, IssueRequest::new(input.repo, input.issue_id))
        .await
        .map(outcome_json)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn mark_project_bw_implemented(
    input: MarkImplementedOperationInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut port = DesktopOperationPort::new(&app, &state);
    operations::mark_implemented(
        &mut port,
        MarkImplementedRequest {
            repo: input.repo,
            issue: input.issue_id,
            commit: input.commit,
            tests: input.tests,
        },
    )
    .await
    .map(outcome_json)
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn business_command_inputs_keep_camel_case_ipc_names() {
        let accept: AcceptOperationInput = serde_json::from_value(json!({
            "repo":"30617:owner:repo",
            "issueId":"issue",
            "delegated":true,
        }))
        .expect("accept input");
        assert!(accept.delegated);

        let ready: MoveToReadyOperationInput = serde_json::from_value(json!({
            "repo":"30617:owner:repo",
            "issueId":"issue",
            "stream":"windows",
            "reworkVerdictId":"verdict",
            "terminalSetId":"set",
        }))
        .expect("ready input");
        assert_eq!(ready.rework_verdict_id.as_deref(), Some("verdict"));
        assert_eq!(ready.terminal_set_id.as_deref(), Some("set"));
    }
}
