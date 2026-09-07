//! Named next-launch settings. Identity, persona and memory remain on the agent.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{readiness::EffectiveHarnessDescriptor, ManagedAgentRecord};

/// One destination-local configuration; references contain names, never credentials.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeConfiguration {
    pub id: String,
    pub revision: String,
    pub name: String,
    pub host: String,
    pub runtime: String,
    pub model: String,
    pub provider: Option<String>,
    pub workspace: Option<String>,
    /// Target environment key -> key in the existing host-local configuration.
    #[serde(default)]
    pub credential_refs: BTreeMap<String, String>,
}

pub use buzz_core_pkg::desktop_lifecycle::RuntimeConfigurationRef;
mod store;
pub(crate) use store::selected_reference;
pub use store::RuntimeConfigurationStore;

impl RuntimeConfiguration {
    pub(crate) fn reference(&self) -> RuntimeConfigurationRef {
        RuntimeConfigurationRef {
            id: self.id.clone(),
            revision: self.revision.clone(),
        }
    }
}

/// Native-only immutable inputs. Never serialize resolved environment or identity keys.
pub(crate) struct PreparedLaunch {
    pub(super) record: ManagedAgentRecord,
    pub(super) descriptor: EffectiveHarnessDescriptor,
    pub(super) effective: super::effective_config::EffectiveAgentConfig,
    host: String,
    scope: (String, String),
}

impl PreparedLaunch {
    pub(crate) fn check_scope(&self, owner: Option<&str>, community: &str) -> Result<(), String> {
        if Some(self.scope.0.as_str()) != owner || self.scope.1 != community {
            return Err("Prepared runtime launch belongs to another owner or community".into());
        }
        Ok(())
    }

    pub(crate) fn record(&self) -> &ManagedAgentRecord {
        &self.record
    }
    pub(crate) fn configuration(&self) -> Option<RuntimeConfigurationRef> {
        selected(&self.record)
            .ok()
            .flatten()
            .map(RuntimeConfiguration::reference)
    }

    /// Fail closed if the record or any resolved launch prerequisite changed.
    pub(crate) fn revalidate(
        &self,
        record: &ManagedAgentRecord,
        personas: &[super::AgentDefinition],
        global: &super::GlobalAgentConfig,
    ) -> Result<(), String> {
        // Ignore lifecycle bookkeeping and other scopes, not launch inputs. A confirmed
        // Stop changes timestamps but must not invalidate an otherwise exact plan.
        let mut comparable = record.clone();
        comparable.runtime_configurations = self.record.runtime_configurations.clone();
        comparable.updated_at = self.record.updated_at.clone();
        comparable.runtime_pid = self.record.runtime_pid;
        comparable.last_started_at = self.record.last_started_at.clone();
        comparable.last_stopped_at = self.record.last_stopped_at.clone();
        comparable.last_exit_code = self.record.last_exit_code;
        comparable.last_error = self.record.last_error.clone();
        comparable.last_error_code = self.record.last_error_code;
        if comparable != self.record {
            return Err("Agent changed during runtime preflight; retry Start".into());
        }
        let current = prepare(
            record,
            self.configuration().as_ref(),
            personas,
            global,
            &self.host,
            &self.scope.0,
            &self.scope.1,
        )?;
        if selected(&current.record)? != selected(&self.record)?
            || current.descriptor != self.descriptor
            || current.effective != self.effective
        {
            return Err("Launch settings changed during runtime preflight; retry Start".into());
        }
        Ok(())
    }
}

/// Resolve an explicit reference without mutating the agent's durable selection.
/// `None` explicitly means legacy Default, never "read selection later".
pub(crate) fn prepare(
    record: &ManagedAgentRecord,
    reference: Option<&RuntimeConfigurationRef>,
    personas: &[super::AgentDefinition],
    global: &super::GlobalAgentConfig,
    host: &str,
    owner: &str,
    community: &str,
) -> Result<PreparedLaunch, String> {
    verify_owner(record, owner)?;
    let mut projected = record.clone();
    let configurations = record.runtime_configurations.get(owner, community);
    configurations.validate()?;
    projected.runtime_configurations.launch = reference.map(|reference| {
        configurations.entries.iter()
            .find(|c| c.id == reference.id && c.revision == reference.revision && c.host == host)
            .cloned().ok_or_else(|| "Runtime configuration is unavailable in this Desktop scope or its revision changed".to_string())
    }).transpose()?;
    let descriptor = super::resolve_effective_harness_descriptor(&projected, personas, global)?;
    preflight(&projected, &descriptor, host)?;
    let effective = super::effective_config::resolve_effective_config(&projected, personas, global)
        .require_resolved()?;
    Ok(PreparedLaunch {
        record: projected,
        descriptor,
        effective,
        host: host.into(),
        scope: (owner.into(), community.into()),
    })
}

/// Absent selection is the legacy Default configuration, with unchanged inheritance.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeConfigurations {
    pub selected: Option<String>,
    pub entries: Vec<RuntimeConfiguration>,
}

impl RuntimeConfigurations {
    pub(crate) fn selected(&self) -> Result<Option<&RuntimeConfiguration>, String> {
        self.selected
            .as_ref()
            .map(|id| {
                self.entries
                    .iter()
                    .find(|entry| &entry.id == id)
                    .ok_or_else(|| "Selected runtime configuration is missing".to_string())
            })
            .transpose()
    }

    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.entries.len() > 32 {
            return Err("At most 32 runtime configurations are supported".into());
        }
        self.selected()?;
        let mut ids = std::collections::BTreeSet::new();
        for entry in &self.entries {
            if !ids.insert(&entry.id)
                || uuid::Uuid::parse_str(&entry.id).is_err()
                || uuid::Uuid::parse_str(&entry.revision).is_err()
                || entry.host.trim().is_empty()
                || entry.host.len() > 128
                || entry.name.trim().is_empty()
                || entry.name.len() > 120
                || entry.model.trim().is_empty()
                || entry.model.len() > 512
                || entry.runtime.trim().is_empty()
                || entry.runtime.len() > 128
                || [&entry.name, &entry.runtime, &entry.model]
                    .iter()
                    .any(|text| text.chars().any(char::is_control))
                || entry.provider.as_ref().is_some_and(|text| {
                    text.trim().is_empty() || text.len() > 128 || text.chars().any(char::is_control)
                })
                || entry.credential_refs.len() > 32
                || entry
                    .workspace
                    .as_ref()
                    .is_some_and(|p| p.len() > 4096 || !std::path::Path::new(p).is_absolute())
                || entry
                    .credential_refs
                    .iter()
                    .any(|(key, source)| !reference_name(key) || !reference_name(source))
            {
                return Err("Invalid destination-local runtime configuration".into());
            }
        }
        Ok(())
    }
}

fn reference_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        && !super::env_vars::is_reserved_env_key(value)
}

pub(crate) fn selected(
    record: &ManagedAgentRecord,
) -> Result<Option<&RuntimeConfiguration>, String> {
    Ok(record.runtime_configurations.launch.as_ref())
}

/// Apply pins after inherited env: saved environment cannot silently override the pick.
pub(crate) fn apply_descriptor(
    record: &ManagedAgentRecord,
    descriptor: &mut EffectiveHarnessDescriptor,
) -> Result<(), String> {
    let Some(config) = selected(record)? else {
        return Ok(());
    };
    let runtime = super::known_acp_runtime(&descriptor.command)
        .ok_or("Named configuration requires a catalogued runtime")?;
    if runtime.provider_locked && config.provider.is_some() {
        return Err("Selected provider is not supported by this runtime".into());
    }
    let references = config
        .credential_refs
        .iter()
        .map(|(target, source)| {
            if !reference_name(target) || !reference_name(source) {
                return Err("Invalid credential reference".to_string());
            }
            descriptor
                .env
                .get(source)
                .filter(|v| !v.trim().is_empty())
                .cloned()
                .map(|value| (target.clone(), value))
                .ok_or_else(|| "A local credential reference is unavailable".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    descriptor.env.extend(references);
    for (key, value) in super::runtime::runtime_metadata_env_vars(
        runtime.model_env_var,
        runtime.provider_env_var,
        runtime.provider_locked,
        Some(&config.model),
        config.provider.as_deref(),
    ) {
        descriptor.env.insert(key.into(), value.into());
    }
    Ok(())
}

/// Recheck before launch, never substitute a setup listener for the selected runtime.
pub(crate) fn preflight(
    record: &ManagedAgentRecord,
    descriptor: &EffectiveHarnessDescriptor,
    host: &str,
) -> Result<(), String> {
    let config = selected(record)?;
    if config.is_some_and(|config| config.host != host) {
        return Err("Runtime configuration belongs to another Desktop".into());
    }
    if let Some(error) = super::storage::spawn_key_refusal(record) {
        return Err(error);
    }
    let keys = nostr::Keys::parse(record.private_key_nsec.trim())
        .map_err(|_| "Local identity access is unavailable")?;
    if keys.public_key().to_hex() != record.pubkey {
        return Err("Local identity does not match this agent".into());
    }
    if super::resolve_command(&record.acp_command).is_none() {
        return Err("ACP runtime is unavailable".into());
    }
    if super::resolve_command(&descriptor.command).is_none() {
        return Err("Selected runtime is unavailable on this Desktop".into());
    }
    if config
        .and_then(|c| c.workspace.as_ref())
        .is_some_and(|path| !std::path::Path::new(path).is_dir())
    {
        return Err("Selected workspace is unavailable on this Desktop".into());
    }
    let effective = super::readiness::EffectiveAgentEnv {
        env: descriptor.env.clone(),
        effective_command: descriptor.command.clone(),
        config_file_path: super::known_acp_runtime(&descriptor.command)
            .and_then(|r| r.config_file_path),
    };
    if !matches!(
        super::agent_readiness(&effective),
        super::AgentReadiness::Ready
    ) {
        return Err("Selected configuration is not ready on this Desktop; check runtime and local credentials".into());
    }
    Ok(())
}

fn verify_owner(record: &ManagedAgentRecord, owner: &str) -> Result<(), String> {
    let verified = record.auth_tag.as_deref().and_then(|tag| {
        let agent = nostr::PublicKey::from_hex(&record.pubkey).ok()?;
        buzz_sdk_pkg::nip_oa::verify_auth_tag(tag, &agent).ok()
    });
    if record.backend != super::BackendKind::Local
        || !verified.is_some_and(|key| key.to_hex() == owner)
    {
        return Err("Agent ownership is unavailable in this scope".into());
    }
    Ok(())
}

/// Resolve on this owner/community's Desktop using the canonical local host identity.
pub(crate) fn prepare_for_app<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    record: &ManagedAgentRecord,
    reference: Option<&RuntimeConfigurationRef>,
    owner: &str,
    community: &str,
) -> Result<PreparedLaunch, String> {
    let host = local_host(app, owner, community)?;
    prepare(
        record,
        reference,
        &super::load_personas(app)?,
        &super::load_global_agent_config(app)?,
        &host,
        owner,
        community,
    )
}

fn local_host<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    owner: &str,
    community: &str,
) -> Result<String, String> {
    use tauri::Manager;
    let state = app.state::<crate::app_state::AppState>();
    let keys = state.signing_keys()?;
    if keys.public_key().to_hex() != owner {
        return Err("Desktop owner changed".into());
    }
    let scope = super::retention::RetentionScope {
        db_path: super::retention::scoped_retention_db_path(
            &super::managed_agents_base_dir(app)?,
            community,
            owner,
        ),
        relay_url: community.into(),
        owner_keys: keys,
    };
    crate::commands::desktop_stop::local_id(
        &mut super::retention::open_retention_db(&scope.db_path)?,
        &scope,
    )
}

/// Owner-private launch choices; deliberately excludes workspace, key references and env.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeConfigurationSummary {
    pub configuration: Option<RuntimeConfigurationRef>,
    pub name: String,
    pub host: String,
    pub runtime: String,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub eligible: bool,
}

/// Shared readiness projection. A missing/unknown prerequisite is never eligible.
pub(crate) fn catalog(
    record: &ManagedAgentRecord,
    personas: &[super::AgentDefinition],
    global: &super::GlobalAgentConfig,
    host: &str,
    owner: &str,
    community: &str,
) -> Vec<RuntimeConfigurationSummary> {
    let configurations = record.runtime_configurations.get(owner, community);
    std::iter::once(None)
        .chain(configurations.entries.iter().map(Some))
        .map(|config| {
            let reference = config.map(RuntimeConfiguration::reference);
            let mut projected = record.clone();
            projected.runtime_configurations.launch = config.cloned();
            let effective =
                super::effective_config::resolve_effective_config(&projected, personas, global)
                    .require_resolved()
                    .ok();
            RuntimeConfigurationSummary {
                configuration: reference.clone(),
                name: config
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| "Default".into()),
                host: config
                    .map(|c| c.host.clone())
                    .unwrap_or_else(|| host.into()),
                runtime: config
                    .map(|c| c.runtime.clone())
                    .unwrap_or_else(|| super::record_agent_command(&projected, personas)),
                model: effective.as_ref().and_then(|e| e.model.value.clone()),
                provider: effective.and_then(|e| e.provider.value),
                eligible: prepare(
                    record,
                    reference.as_ref(),
                    personas,
                    global,
                    host,
                    owner,
                    community,
                )
                .is_ok(),
            }
        })
        .collect()
}

/// Validate or resolve before any existing pair is reaped. The shared spawn checks again.
pub(crate) fn prepare_selected<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    record: &ManagedAgentRecord,
    owner: Option<&str>,
    community: &str,
) -> Result<Option<PreparedLaunch>, String> {
    selected_reference(record, owner, community)?
        .map(|reference| {
            prepare_for_app(
                app,
                record,
                Some(&reference),
                owner.ok_or("Desktop owner unavailable")?,
                community,
            )
        })
        .transpose()
}

/// Scoped safe catalog for lifecycle consumers; never exposes the global configuration store.
pub(crate) fn catalog_for_app<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    record: &ManagedAgentRecord,
    owner: &str,
    community: &str,
) -> Result<Vec<RuntimeConfigurationSummary>, String> {
    verify_owner(record, owner)?;
    let host = local_host(app, owner, community)?;
    Ok(catalog(
        record,
        &super::load_personas(app)?,
        &super::load_global_agent_config(app)?,
        &host,
        owner,
        community,
    ))
}

/// Ordinary async mesh readiness, outside the transition lock. Caller revalidates
/// owner/community after this await and the immutable plan under its admission lock.
pub(crate) async fn preflight_prepared(
    app: &tauri::AppHandle,
    plan: &PreparedLaunch,
    owner: &str,
    community: &str,
) -> Result<(), String> {
    plan.check_scope(Some(owner), community)?;
    #[cfg(feature = "mesh-llm")]
    crate::commands::ensure_relay_mesh_for_record(
        app,
        plan.effective.relay_mesh_model_id().as_deref(),
        false,
    )
    .await?;
    #[cfg(not(feature = "mesh-llm"))]
    let _ = app;
    Ok(())
}

#[cfg(test)]
mod tests;
