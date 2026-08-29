//! OS-keyring storage for catalog-declared runtime setup secrets.
//!
//! Runtime setup values whose catalog field kind is `Secret` never persist in
//! `managed-agents.json` and never cross a renderer read boundary. The owning
//! definition/instance is represented in the keyring key, while the catalog
//! remains the sole source of which environment keys are secret capabilities.

use std::collections::{BTreeMap, HashMap};

use serde::ser::SerializeMap as _;

use crate::{
    app_state::keyring_service,
    managed_agents::{
        runtime_setup_fields, AgentDefinition, GlobalAgentConfig, ManagedAgentRecord,
        RuntimeSetupField, RuntimeSetupFieldKind, RuntimeSetupFieldValidation,
    },
    secret_store::SecretStore,
};

const SETUP_SECRET_PREFIX: &str = "agent-setup";

trait SetupSecretStore {
    fn load(&self, key: &str) -> Result<Option<String>, String>;
    fn store_all(&self, entries: &HashMap<String, String>) -> Result<(), String>;
    fn verify_stored_raw(&self, key: &str, expected: &str) -> Result<bool, String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

impl SetupSecretStore for SecretStore {
    fn load(&self, key: &str) -> Result<Option<String>, String> {
        SecretStore::load(self, key)
    }

    fn store_all(&self, entries: &HashMap<String, String>) -> Result<(), String> {
        SecretStore::store_all(self, entries)
    }

    fn verify_stored_raw(&self, key: &str, expected: &str) -> Result<bool, String> {
        SecretStore::verify_stored_raw(self, key, expected)
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        SecretStore::delete(self, key)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SetupSecretScope {
    Definition(String),
    Instance(String),
}

impl SetupSecretScope {
    fn key(&self, env_key: &str) -> String {
        match self {
            Self::Definition(id) => {
                format!("{SETUP_SECRET_PREFIX}:definition:{id}:{env_key}")
            }
            Self::Instance(pubkey) => {
                format!("{SETUP_SECRET_PREFIX}:instance:{pubkey}:{env_key}")
            }
        }
    }
}

fn setup_secret_store() -> Result<&'static SecretStore, String> {
    if !cfg!(feature = "system-keyring") {
        return Err(
            "secure credential storage is unavailable in this build; refusing to store runtime setup secrets"
                .to_string(),
        );
    }
    Ok(SecretStore::shared(keyring_service()))
}

fn record_scope(record: &ManagedAgentRecord) -> Result<SetupSecretScope, String> {
    if record.pubkey.is_empty() {
        let id = record
            .slug
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                "cannot persist runtime setup secret for a definition without an id".to_string()
            })?;
        Ok(SetupSecretScope::Definition(id.to_string()))
    } else {
        Ok(SetupSecretScope::Instance(record.pubkey.clone()))
    }
}

fn all_setup_secret_fields() -> Vec<RuntimeSetupField> {
    crate::managed_agents::known_runtime_ids()
        .flat_map(runtime_setup_fields)
        .filter(|field| field.kind == RuntimeSetupFieldKind::Secret)
        .fold(Vec::<RuntimeSetupField>::new(), |mut fields, field| {
            if !fields
                .iter()
                .any(|existing| existing.env_key.eq_ignore_ascii_case(&field.env_key))
            {
                fields.push(field);
            }
            fields
        })
}

pub(crate) fn runtime_setup_secret_fields(runtime_id: &str) -> Vec<RuntimeSetupField> {
    runtime_setup_fields(runtime_id)
        .into_iter()
        .filter(|field| field.kind == RuntimeSetupFieldKind::Secret)
        .collect()
}

pub(crate) fn is_setup_secret_env_key(key: &str) -> bool {
    all_setup_secret_fields()
        .iter()
        .any(|field| field.env_key.eq_ignore_ascii_case(key))
}

pub(crate) fn strip_setup_secret_env_values(env: &mut BTreeMap<String, String>) {
    env.retain(|key, _| !is_setup_secret_env_key(key));
}

/// Serialize an env map for a renderer-facing type while omitting every
/// catalog-declared setup secret. The configured state has a separate typed
/// IPC response and is never represented by a token-shaped placeholder.
pub(crate) fn serialize_env_without_setup_secrets<S>(
    env: &BTreeMap<String, String>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let visible = env.iter().filter(|(key, _)| !is_setup_secret_env_key(key));
    let mut map = serializer.serialize_map(None)?;
    for (key, value) in visible {
        map.serialize_entry(key, value)?;
    }
    map.end()
}

fn validate_setup_secret(field: &RuntimeSetupField, value: &str) -> Result<(), String> {
    let valid = match field.validation {
        RuntimeSetupFieldValidation::AppPassword => value.trim().len() >= 32,
        RuntimeSetupFieldValidation::TailnetHttpsOrigin => false,
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "runtime setup secret {} does not satisfy the catalog validation policy",
            field.env_key
        ))
    }
}

fn persist_and_strip_record_with(
    store: &impl SetupSecretStore,
    record: &mut ManagedAgentRecord,
) -> Result<bool, String> {
    let fields = all_setup_secret_fields();
    let present: Vec<(RuntimeSetupField, String)> = fields
        .into_iter()
        .filter_map(|field| {
            record
                .env_vars
                .get(&field.env_key)
                .cloned()
                .map(|value| (field, value))
        })
        .collect();
    if present.is_empty() {
        return Ok(false);
    }

    let scope = record_scope(record)?;
    let mut entries = HashMap::new();
    for (field, value) in &present {
        validate_setup_secret(field, value)?;
        entries.insert(scope.key(&field.env_key), value.clone());
    }
    store.store_all(&entries).map_err(|error| {
        format!("secure credential storage is unavailable; runtime setup was not saved: {error}")
    })?;
    for (key, expected) in &entries {
        let verified = store.verify_stored_raw(key, expected).map_err(|error| {
            format!(
                "secure credential storage verification failed; runtime setup was not saved: {error}"
            )
        })?;
        if !verified {
            return Err(
                "secure credential storage verification failed; runtime setup was not saved"
                    .to_string(),
            );
        }
    }
    for (field, _) in present {
        record.env_vars.remove(&field.env_key);
    }
    Ok(true)
}

/// Persist and strip setup secrets from save-local record clones. A keyring
/// failure aborts the caller before the JSON store is replaced.
pub(crate) fn persist_and_strip_setup_secrets(
    records: &mut [ManagedAgentRecord],
) -> Result<bool, String> {
    if !records.iter().any(|record| {
        record
            .env_vars
            .keys()
            .any(|key| is_setup_secret_env_key(key))
    }) {
        return Ok(false);
    }
    let store = setup_secret_store()?;
    let mut changed = false;
    for record in records {
        changed |= persist_and_strip_record_with(store, record)?;
    }
    Ok(changed)
}

fn definition_views(records: &[ManagedAgentRecord]) -> Vec<AgentDefinition> {
    records
        .iter()
        .filter(|record| record.pubkey.is_empty())
        .filter_map(ManagedAgentRecord::to_definition_view)
        .collect()
}

fn record_runtime_id(
    record: &ManagedAgentRecord,
    definitions: &[AgentDefinition],
) -> Option<String> {
    if record.pubkey.is_empty() {
        return record.runtime.clone();
    }
    crate::managed_agents::resolve_effective_harness_descriptor(
        record,
        definitions,
        &GlobalAgentConfig::default(),
    )
    .ok()
    .and_then(|descriptor| descriptor.runtime_id)
}

fn hydrate_record_with(
    store: &impl SetupSecretStore,
    record: &mut ManagedAgentRecord,
    runtime_id: Option<&str>,
) -> Result<(), String> {
    let Some(runtime_id) = runtime_id else {
        return Ok(());
    };
    let fields = runtime_setup_secret_fields(runtime_id);
    if fields.is_empty() {
        return Ok(());
    }
    let scope = record_scope(record)?;
    for field in fields {
        if let Some(value) = store.load(&scope.key(&field.env_key)).map_err(|error| {
            format!(
                "secure credential storage is unavailable for runtime {runtime_id}; refusing to project or start it: {error}"
            )
        })? {
            validate_setup_secret(&field, &value)?;
            record.env_vars.insert(field.env_key, value);
        }
    }
    Ok(())
}

/// Hydrate only the records whose effective catalog runtime declares secret
/// setup fields. An unavailable keyring therefore blocks the affected runtime
/// instead of injecting an empty/fake credential or exposing legacy plaintext.
pub(crate) fn hydrate_setup_secrets(records: &mut [ManagedAgentRecord]) -> Result<(), String> {
    let definitions = definition_views(records);
    let runtime_ids: Vec<Option<String>> = records
        .iter()
        .map(|record| record_runtime_id(record, &definitions))
        .collect();
    if !runtime_ids
        .iter()
        .flatten()
        .any(|runtime_id| !runtime_setup_secret_fields(runtime_id).is_empty())
    {
        return Ok(());
    }
    let store = setup_secret_store()?;
    for (record, runtime_id) in records.iter_mut().zip(runtime_ids.iter()) {
        hydrate_record_with(store, record, runtime_id.as_deref())?;
    }
    Ok(())
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RuntimeSetupSecretStatus {
    pub configured_env_keys: Vec<String>,
    pub inherited_env_keys: Vec<String>,
}

fn configured_keys_for_scope(
    store: &impl SetupSecretStore,
    runtime_id: &str,
    scope: SetupSecretScope,
) -> Result<Vec<String>, String> {
    let mut configured = Vec::new();
    for field in runtime_setup_secret_fields(runtime_id) {
        if let Some(value) = store.load(&scope.key(&field.env_key)).map_err(|error| {
            format!("secure credential storage is unavailable for runtime {runtime_id}: {error}")
        })? {
            validate_setup_secret(&field, &value)?;
            configured.push(field.env_key);
        }
    }
    Ok(configured)
}

/// Return key names only. The secret bytes stay in Rust/keyring and never
/// enter the renderer response.
pub(crate) fn runtime_setup_secret_status(
    runtime_id: &str,
    definition_id: Option<&str>,
    agent_pubkey: Option<&str>,
) -> Result<RuntimeSetupSecretStatus, String> {
    if runtime_setup_secret_fields(runtime_id).is_empty() {
        return Ok(RuntimeSetupSecretStatus::default());
    }
    let store = setup_secret_store()?;
    let configured_env_keys = match agent_pubkey {
        Some(pubkey) => configured_keys_for_scope(
            store,
            runtime_id,
            SetupSecretScope::Instance(pubkey.to_string()),
        )?,
        None => match definition_id {
            Some(id) => configured_keys_for_scope(
                store,
                runtime_id,
                SetupSecretScope::Definition(id.to_string()),
            )?,
            None => Vec::new(),
        },
    };
    let inherited_env_keys = if agent_pubkey.is_some() {
        match definition_id {
            Some(id) => configured_keys_for_scope(
                store,
                runtime_id,
                SetupSecretScope::Definition(id.to_string()),
            )?,
            None => Vec::new(),
        }
    } else {
        Vec::new()
    };
    Ok(RuntimeSetupSecretStatus {
        configured_env_keys,
        inherited_env_keys,
    })
}

fn delete_scope_with(store: &impl SetupSecretStore, scope: SetupSecretScope) -> Result<(), String> {
    for field in all_setup_secret_fields() {
        store.delete(&scope.key(&field.env_key))?;
    }
    Ok(())
}

pub(crate) fn delete_instance_setup_secrets(pubkey: &str) {
    let Ok(store) = setup_secret_store() else {
        return;
    };
    if let Err(error) = delete_scope_with(store, SetupSecretScope::Instance(pubkey.to_string())) {
        eprintln!("buzz-desktop: failed to delete agent runtime setup secrets: {error}");
    }
}

pub(crate) fn delete_instance_credentials(pubkey: &str) {
    crate::managed_agents::delete_agent_key(pubkey);
    delete_instance_setup_secrets(pubkey);
}

pub(crate) fn delete_definition_setup_secrets(id: &str) {
    let Ok(store) = setup_secret_store() else {
        return;
    };
    if let Err(error) = delete_scope_with(store, SetupSecretScope::Definition(id.to_string())) {
        eprintln!("buzz-desktop: failed to delete definition runtime setup secrets: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeStore {
        values: Mutex<HashMap<String, String>>,
        unavailable: bool,
        fail_verify: bool,
    }

    impl SetupSecretStore for FakeStore {
        fn load(&self, key: &str) -> Result<Option<String>, String> {
            if self.unavailable {
                return Err("keyring unavailable".to_string());
            }
            Ok(self
                .values
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(key)
                .cloned())
        }

        fn store_all(&self, entries: &HashMap<String, String>) -> Result<(), String> {
            if self.unavailable {
                return Err("keyring unavailable".to_string());
            }
            self.values
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .extend(entries.clone());
            Ok(())
        }

        fn verify_stored_raw(&self, key: &str, expected: &str) -> Result<bool, String> {
            if self.unavailable {
                return Err("keyring unavailable".to_string());
            }
            if self.fail_verify {
                return Ok(false);
            }
            Ok(self
                .values
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .get(key)
                .is_some_and(|value| value == expected))
        }

        fn delete(&self, key: &str) -> Result<(), String> {
            self.values
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(key);
            Ok(())
        }
    }

    fn definition_record() -> ManagedAgentRecord {
        AgentDefinition {
            id: "definition-id".to_string(),
            display_name: "Remote".to_string(),
            avatar_url: None,
            system_prompt: String::new(),
            runtime: Some("remote-agent-computer".to_string()),
            model: None,
            provider: None,
            name_pool: Vec::new(),
            is_builtin: false,
            is_active: true,
            shared: false,
            source_team: None,
            source_team_persona_slug: None,
            catalog_source: None,
            env_vars: BTreeMap::from([
                (
                    "MANUAL_AGENT_BASE_URL".to_string(),
                    "https://mac.tailnet.ts.net:8787".to_string(),
                ),
                (
                    "MANUAL_AGENT_TOKEN".to_string(),
                    "test-app-password-0123456789-abcdef".to_string(),
                ),
            ]),
            respond_to: None,
            respond_to_allowlist: Vec::new(),
            parallelism: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
        .into_agent_record()
    }

    #[test]
    fn persists_verified_secret_and_strips_only_secret_field() {
        let store = FakeStore::default();
        let mut record = definition_record();
        assert!(persist_and_strip_record_with(&store, &mut record).unwrap());
        assert!(!record.env_vars.contains_key("MANUAL_AGENT_TOKEN"));
        assert!(record.env_vars.contains_key("MANUAL_AGENT_BASE_URL"));
        assert_eq!(
            store
                .load("agent-setup:definition:definition-id:MANUAL_AGENT_TOKEN")
                .unwrap()
                .as_deref(),
            Some("test-app-password-0123456789-abcdef")
        );
    }

    #[test]
    fn keyring_failure_keeps_secret_in_save_local_record_and_errors() {
        let store = FakeStore {
            unavailable: true,
            ..Default::default()
        };
        let mut record = definition_record();
        let error = persist_and_strip_record_with(&store, &mut record).unwrap_err();
        assert!(error.contains("secure credential storage is unavailable"));
        assert!(record.env_vars.contains_key("MANUAL_AGENT_TOKEN"));
    }

    #[test]
    fn failed_read_back_never_strips_secret() {
        let store = FakeStore {
            fail_verify: true,
            ..Default::default()
        };
        let mut record = definition_record();
        assert!(persist_and_strip_record_with(&store, &mut record).is_err());
        assert!(record.env_vars.contains_key("MANUAL_AGENT_TOKEN"));
    }

    #[test]
    fn configured_status_returns_key_names_never_values() {
        let store = FakeStore::default();
        store
            .store_all(&HashMap::from([(
                "agent-setup:definition:definition-id:MANUAL_AGENT_TOKEN".to_string(),
                "test-app-password-0123456789-abcdef".to_string(),
            )]))
            .unwrap();
        let keys = configured_keys_for_scope(
            &store,
            "remote-agent-computer",
            SetupSecretScope::Definition("definition-id".to_string()),
        )
        .unwrap();
        assert_eq!(keys, vec!["MANUAL_AGENT_TOKEN"]);
        assert!(!serde_json::to_string(&keys)
            .unwrap()
            .contains("test-app-password"));
    }

    #[test]
    fn invalid_stored_secret_is_not_reported_as_configured() {
        let store = FakeStore::default();
        store
            .store_all(&HashMap::from([(
                "agent-setup:definition:definition-id:MANUAL_AGENT_TOKEN".to_string(),
                "short".to_string(),
            )]))
            .unwrap();
        let error = configured_keys_for_scope(
            &store,
            "remote-agent-computer",
            SetupSecretScope::Definition("definition-id".to_string()),
        )
        .unwrap_err();
        assert!(error.contains("does not satisfy the catalog validation policy"));
    }

    #[test]
    fn renderer_serializer_omits_catalog_secret_without_placeholder() {
        #[derive(serde::Serialize)]
        struct Projection {
            #[serde(serialize_with = "serialize_env_without_setup_secrets")]
            env: BTreeMap<String, String>,
        }
        let env = definition_record().env_vars;
        let serialized = serde_json::to_string(&Projection { env }).unwrap();
        assert!(serialized.contains("MANUAL_AGENT_BASE_URL"));
        assert!(!serialized.contains("MANUAL_AGENT_TOKEN"));
        assert!(!serialized.contains("test-app-password"));
    }
}
