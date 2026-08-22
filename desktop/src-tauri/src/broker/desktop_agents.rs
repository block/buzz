//! The Desktop-backed [`AgentService`] and [`AgentRoster`].
//!
//! This is the only place in the broker that knows about Tauri. It translates a
//! validated capability request into the same commands the UI calls —
//! `create_managed_agent`, `update_managed_agent`, `delete_managed_agent` — so a
//! brokered mutation and a hand-driven one take one code path. Reimplementing
//! the mutation here would mean two agent-creation routines drifting apart, and
//! the broker's would be the one nobody looks at.
//!
//! # Where the credential boundary sits
//!
//! Nothing here returns secret material. `create_managed_agent` hands back a
//! `CreateManagedAgentResponse` that *does* contain the minted
//! `private_key_nsec`; [`DesktopAgentService::create`] reads only the summary
//! and drops the response, and [`AgentsCreateOutcome`] has no field that could
//! hold it. That is the narrowing point: the secret exists on one stack frame
//! inside this file and never reaches the pipeline, the store, or the wire.
//!
//! # Why the request translation is a separate pure function
//!
//! [`create_request`] and [`update_request`] are pure and tested directly.
//! Everything they decide — which optional fields become `None`, that a
//! brokered create never spawns a process — is policy worth pinning in a test,
//! and none of it needs an `AppHandle`.

use buzz_sdk_pkg::broker::{
    AgentTarget, AgentsCreateArgs, AgentsCreateOutcome, AgentsDeleteArgs, AgentsDeleteOutcome,
    AgentsUpdateArgs, AgentsUpdateOutcome,
};
use tauri::{AppHandle, Manager};

use crate::app_state::AppState;
use crate::managed_agents::{
    load_managed_agents, CreateManagedAgentRequest, ManagedAgentRecord, RespondTo,
    UpdateManagedAgentRequest, DEFAULT_ACP_COMMAND,
};

use super::agents_policy::AgentRoster;
use super::handlers::{AgentService, ServiceError};
use super::pipeline::BoxFuture;

/// Build the create request the Desktop command expects.
///
/// A brokered create is deliberately *not* spawned (`spawn_after_create:
/// false`) and does not auto-start (`start_on_app_launch: false`). A remote
/// requester asking for an agent to exist has not asked for a local process to
/// be running, and starting one would consume host resources on a schedule
/// nobody at the keyboard chose. The owner starts it from the UI.
///
/// `persona_id` and `team_id` stay `None`: linking a definition pins a snapshot
/// of someone else's config onto the new agent, which is a second decision the
/// request never made.
fn create_request(args: &AgentsCreateArgs) -> Result<CreateManagedAgentRequest, String> {
    let respond_to = args
        .respond_to
        .as_deref()
        .map(RespondTo::parse_wire)
        .transpose()?;

    Ok(CreateManagedAgentRequest {
        name: args.display_name.clone(),
        persona_id: None,
        team_id: None,
        relay_url: None,
        acp_command: Some(DEFAULT_ACP_COMMAND.to_string()),
        // `agent_command` is the harness pin. A requested runtime is resolved to
        // its command here rather than passed through raw, so an arbitrary
        // string can never become an executed command.
        agent_command: args
            .runtime
            .as_deref()
            .map(resolve_runtime_command)
            .transpose()?,
        harness_override: args.runtime.is_some(),
        agent_args: Vec::new(),
        mcp_command: None,
        turn_timeout_seconds: None,
        idle_timeout_seconds: None,
        max_turn_duration_seconds: None,
        parallelism: None,
        system_prompt: Some(args.system_prompt.clone()),
        avatar_url: None,
        model: args.model.clone(),
        provider: args.provider.clone(),
        env_vars: std::collections::BTreeMap::new(),
        spawn_after_create: false,
        start_on_app_launch: false,
        backend: crate::managed_agents::BackendKind::Local,
        respond_to,
        respond_to_allowlist: Vec::new(),
        relay_mesh: None,
    })
}

/// Resolve a requested runtime id to its harness command.
///
/// Refuses an id the catalog does not know instead of storing it: an
/// unresolvable pin produces an agent that cannot start, and the failure would
/// surface much later as a spawn error with no connection to this request.
fn resolve_runtime_command(runtime: &str) -> Result<String, String> {
    crate::managed_agents::command_for_runtime_id(runtime)
        .ok_or_else(|| format!("unknown runtime \"{runtime}\""))
}

/// Build the update request, and report which fields it will change.
///
/// The field list is derived from the request rather than from a diff of the
/// record, because the requester asked for these fields specifically. The
/// outcome names exactly what was asked for and applied.
fn update_request(
    pubkey: &str,
    args: &AgentsUpdateArgs,
) -> Result<(UpdateManagedAgentRequest, Vec<String>), String> {
    let mut changed = Vec::new();
    if args.display_name.is_some() {
        changed.push("displayName".to_string());
    }
    if args.system_prompt.is_some() {
        changed.push("systemPrompt".to_string());
    }
    if args.runtime.is_some() {
        changed.push("runtime".to_string());
    }
    if args.provider.is_some() {
        changed.push("provider".to_string());
    }
    if args.model.is_some() {
        changed.push("model".to_string());
    }
    if args.respond_to.is_some() {
        changed.push("respondTo".to_string());
    }
    changed.sort();

    let respond_to = args
        .respond_to
        .as_deref()
        .map(RespondTo::parse_wire)
        .transpose()?;

    let request = UpdateManagedAgentRequest {
        pubkey: pubkey.to_string(),
        name: args.display_name.clone(),
        // `Option<Option<T>>`: absent leaves the stored value alone. The broker
        // has no "clear to default" verb, so it never sends `Some(None)` — a
        // requester cannot blank a field it did not name.
        model: args.model.clone().map(Some),
        system_prompt: args.system_prompt.clone().map(Some),
        env_vars: None,
        parallelism: None,
        turn_timeout_seconds: None,
        relay_url: None,
        acp_command: None,
        agent_command: args
            .runtime
            .as_deref()
            .map(resolve_runtime_command)
            .transpose()?,
        harness_override: args.runtime.is_some(),
        agent_args: None,
        mcp_command: None,
        provider: args.provider.clone().map(Some),
        respond_to,
        // Allowlist mode is not reachable through this capability (the SDK
        // rejects the mode), so there is never a list to replace.
        respond_to_allowlist: None,
    };
    Ok((request, changed))
}

/// Add `agent_pubkey` to `channel_id` as a bot member.
///
/// The same owner-signed kind-9000 event `add_channel_members` publishes. It is
/// spelled out here rather than routed through that command because the command
/// swallows per-pubkey failures into a result array, and a broker create must
/// treat a failed attachment as a reportable outcome, not a field in a summary.
async fn attach_to_channel(
    app: &AppHandle,
    channel_id: &str,
    agent_pubkey: &str,
) -> Result<(), String> {
    let uuid = uuid::Uuid::parse_str(channel_id)
        .map_err(|_| format!("invalid channel UUID: {channel_id}"))?;
    let builder = crate::events::build_add_member(uuid, agent_pubkey, Some("bot"))?;
    let state = app.state::<AppState>();
    crate::relay::submit_event(builder, &state).await?;
    Ok(())
}

/// Resolve a target selector against the owner's roster.
///
/// Name matching is case-insensitive and must be unambiguous: two agents
/// sharing a name is a real state, and picking one arbitrarily would delete or
/// rewrite the wrong agent.
fn resolve_target(
    records: &[ManagedAgentRecord],
    target: &AgentTarget,
) -> Result<ManagedAgentRecord, String> {
    match target {
        AgentTarget::Pubkey(pubkey) => records
            .iter()
            .find(|record| record.pubkey.eq_ignore_ascii_case(pubkey))
            .cloned()
            .ok_or_else(|| format!("no managed agent with pubkey {pubkey}")),
        AgentTarget::Name(name) => {
            let mut matches = records
                .iter()
                .filter(|record| record.name.eq_ignore_ascii_case(name));
            let first = matches
                .next()
                .cloned()
                .ok_or_else(|| format!("no managed agent named \"{name}\""))?;
            if matches.next().is_some() {
                return Err(format!(
                    "more than one managed agent is named \"{name}\"; target it by pubkey"
                ));
            }
            Ok(first)
        }
    }
}

/// [`AgentService`] over the Desktop agent commands.
pub struct DesktopAgentService {
    app: AppHandle,
}

impl DesktopAgentService {
    /// Bind the service to a running app.
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    /// Snapshot the owner's agent records.
    fn records(&self) -> Result<Vec<ManagedAgentRecord>, String> {
        let state = self.app.state::<AppState>();
        let _guard = state
            .managed_agents_store_lock
            .lock()
            .map_err(|error| error.to_string())?;
        load_managed_agents(&self.app)
    }
}

impl AgentService for DesktopAgentService {
    fn create<'a>(
        &'a self,
        _owner_pubkey: &'a str,
        args: &'a AgentsCreateArgs,
    ) -> BoxFuture<'a, Result<AgentsCreateOutcome, ServiceError>> {
        Box::pin(async move {
            let request = create_request(args).map_err(ServiceError::Failed)?;
            let response = crate::commands::create_managed_agent(
                request,
                self.app.clone(),
                self.app.state::<AppState>(),
            )
            .await
            .map_err(ServiceError::Failed)?;

            let agent_pubkey = response.agent.pubkey.clone();
            let display_name = response.agent.name.clone();
            // The minted nsec dies with `response` at the end of this scope.
            drop(response);

            // Attachment is a second, separately failable step. The agent
            // already exists at this point, so a failure here is
            // `Unknown`, not `Failed`: reporting "failed" would tell the
            // requester nothing happened while an agent sits in the roster.
            attach_to_channel(&self.app, &args.channel_id, &agent_pubkey)
                .await
                .map_err(|error| {
                    ServiceError::Unknown(format!(
                        "agent {agent_pubkey} was created but could not be added to channel \
                     {}: {error}",
                        args.channel_id
                    ))
                })?;

            Ok(AgentsCreateOutcome {
                agent_pubkey,
                display_name,
                channel_id: args.channel_id.clone(),
            })
        })
    }

    fn update<'a>(
        &'a self,
        _owner_pubkey: &'a str,
        args: &'a AgentsUpdateArgs,
    ) -> BoxFuture<'a, Result<AgentsUpdateOutcome, ServiceError>> {
        Box::pin(async move {
            let records = self.records().map_err(ServiceError::Failed)?;
            let record = resolve_target(&records, &args.target).map_err(ServiceError::Failed)?;
            let (request, updated_fields) =
                update_request(&record.pubkey, args).map_err(ServiceError::Failed)?;

            let response = crate::commands::update_managed_agent(
                request,
                self.app.clone(),
                self.app.state::<AppState>(),
            )
            .await
            .map_err(ServiceError::Failed)?;

            Ok(AgentsUpdateOutcome {
                agent_pubkey: response.agent.pubkey,
                display_name: response.agent.name,
                updated_fields,
            })
        })
    }

    fn delete<'a>(
        &'a self,
        _owner_pubkey: &'a str,
        args: &'a AgentsDeleteArgs,
    ) -> BoxFuture<'a, Result<AgentsDeleteOutcome, ServiceError>> {
        Box::pin(async move {
            let records = self.records().map_err(ServiceError::Failed)?;
            let record = resolve_target(&records, &args.target).map_err(ServiceError::Failed)?;
            let agent_pubkey = record.pubkey.clone();
            let display_name = record.name.clone();

            // `force_remote_delete: None` keeps the deployed-remote guard in
            // force. Orphaning provisioned remote infrastructure is a decision
            // for a human looking at the warning, not something a brokered
            // request may force.
            crate::commands::delete_managed_agent(agent_pubkey.clone(), None, self.app.clone())
                .await
                .map_err(ServiceError::Failed)?;

            Ok(AgentsDeleteOutcome {
                agent_pubkey,
                display_name,
            })
        })
    }
}

/// [`AgentRoster`] reading the authoritative Desktop agent store.
pub struct DesktopAgentRoster {
    app: AppHandle,
}

impl DesktopAgentRoster {
    /// Bind the roster to a running app.
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl AgentRoster for DesktopAgentRoster {
    fn agent_pubkeys<'a>(
        &'a self,
        owner_pubkey: &'a str,
    ) -> BoxFuture<'a, Result<Vec<String>, String>> {
        Box::pin(async move {
            // The store holds exactly one owner's agents — the owner whose keys
            // this app instance holds. A request naming a different owner
            // cannot be answered from here, and answering it from this roster
            // anyway would authorize against the wrong set.
            let state = self.app.state::<AppState>();
            let host_owner = state
                .keys
                .lock()
                .map_err(|error| error.to_string())?
                .public_key()
                .to_hex();
            if !host_owner.eq_ignore_ascii_case(owner_pubkey) {
                return Err(format!(
                    "this host holds credentials for {host_owner}, not {owner_pubkey}"
                ));
            }

            let _guard = state
                .managed_agents_store_lock
                .lock()
                .map_err(|error| error.to_string())?;
            Ok(load_managed_agents(&self.app)?
                .into_iter()
                .map(|record| record.pubkey)
                .collect())
        })
    }
}

#[cfg(test)]
mod tests;
