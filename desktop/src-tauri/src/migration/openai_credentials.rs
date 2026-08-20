use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::managed_agents::{
    effective_config::{resolve_effective_config, ConfigSource, EffectiveConfigResult},
    AgentDefinition, GlobalAgentConfig, ManagedAgentRecord,
};

const OPENAI_API_KEY: &str = "OPENAI_API_KEY";
const OPENAI_COMPAT_API_KEY: &str = "OPENAI_COMPAT_API_KEY";
const OPENAI_COMPAT_BASE_URL: &str = "OPENAI_COMPAT_BASE_URL";
const CANONICAL_OPENAI_ORIGIN: &str = "https://api.openai.com/v1";
const MARKER_FILE: &str = ".openai-credentials-v1.migrated";
const MANIFEST_FILE: &str = ".openai-credentials-v1.transaction.json";
const BACKUP_SUFFIX: &str = "pre-openai-credentials-v1.bak";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Owner {
    Global,
    Record(usize),
    Baked,
    Harness(String),
    BuiltinDefinition(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Identity {
    Official,
    Compatible,
}

type EnvLayer = (Owner, BTreeMap<String, String>);
type SerializedStore = (Option<Vec<u8>>, Option<Vec<u8>>);

#[derive(Clone, Debug)]
struct ResolvedValue {
    value: String,
    owner: Owner,
}

#[derive(Clone, Debug)]
struct Consumer {
    label: String,
    record_index: Option<usize>,
    provider: String,
    provider_owner: Owner,
    identity: Identity,
    endpoint: Option<ResolvedValue>,
    legacy_key: Option<ResolvedValue>,
    official_key: Option<ResolvedValue>,
}

#[derive(Clone, Debug)]
struct Store {
    global: GlobalAgentConfig,
    records: Vec<ManagedAgentRecord>,
}

#[derive(Debug, Serialize, Deserialize)]
struct FileHashes {
    source: Option<String>,
    target: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TransactionManifest {
    global: FileHashes,
    managed: FileHashes,
}

#[derive(Clone)]
struct PlannedStore {
    global: Option<Vec<u8>>,
    managed: Option<Vec<u8>>,
    consumers: Vec<Consumer>,
}

fn hash(bytes: Option<&[u8]>) -> Option<String> {
    bytes.map(|bytes| hex::encode(Sha256::digest(bytes)))
}

fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    crate::util::resolved_backup_path(path, &format!("{name}.{suffix}"))
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    if !path.exists() {
        return Ok(None);
    }
    std::fs::read(path)
        .map(Some)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))
}

fn parse_store(global: Option<&[u8]>, managed: Option<&[u8]>) -> Result<Store, String> {
    let global = match global {
        Some(bytes) => serde_json::from_slice(bytes)
            .map_err(|e| format!("failed to parse global agent config: {e}"))?,
        None => GlobalAgentConfig::default(),
    };
    let records = match managed {
        Some(bytes) => serde_json::from_slice(bytes)
            .map_err(|e| format!("failed to parse managed agent store: {e}"))?,
        None => Vec::new(),
    };
    Ok(Store { global, records })
}

fn definitions(store: &Store) -> Vec<AgentDefinition> {
    let mut defs: Vec<_> = store
        .records
        .iter()
        .filter(|r| r.pubkey.is_empty())
        .filter_map(ManagedAgentRecord::to_definition_view)
        .collect();
    for id in store.records.iter().filter_map(|r| r.persona_id.as_deref()) {
        if !defs.iter().any(|d| d.id == id) {
            if let Some(def) = crate::managed_agents::built_in_persona_definition(id, "") {
                defs.push(def);
            }
        }
    }
    defs
}

fn selected_definition<'a>(
    record: &ManagedAgentRecord,
    defs: &'a [AgentDefinition],
) -> Option<&'a AgentDefinition> {
    record
        .persona_id
        .as_deref()
        .and_then(|id| defs.iter().find(|d| d.id == id))
}

fn runtime_id<'a>(
    record: &'a ManagedAgentRecord,
    def: Option<&'a AgentDefinition>,
) -> Option<&'a str> {
    record
        .runtime
        .as_deref()
        .or_else(|| def.and_then(|d| d.runtime.as_deref()))
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

fn layers(
    store: &Store,
    record: &ManagedAgentRecord,
    def: Option<&AgentDefinition>,
) -> Result<Vec<EnvLayer>, String> {
    let mut result = vec![(Owner::Baked, crate::managed_agents::baked_build_env())];
    if let Some(id) = runtime_id(record, def) {
        if let Some(harness) =
            crate::managed_agents::custom_harnesses::lookup_loaded_harness_by_id(id)
        {
            result.push((Owner::Harness(id.to_string()), harness.env.clone()));
        } else if crate::managed_agents::known_acp_runtime_exact(id).is_none()
            && record
                .agent_command_override
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .is_none()
        {
            return Err(format!("unresolved custom harness {id:?}"));
        }
    }
    result.push((Owner::Global, store.global.env_vars.clone()));
    if let Some(def) = def {
        let owner = store
            .records
            .iter()
            .position(|r| r.pubkey.is_empty() && r.slug.as_deref() == Some(def.id.as_str()))
            .map(Owner::Record)
            .unwrap_or_else(|| Owner::BuiltinDefinition(def.id.clone()));
        result.push((owner, def.env_vars.clone()));
    }
    result.push((
        Owner::Record(
            store
                .records
                .iter()
                .position(|candidate| std::ptr::eq(candidate, record))
                .ok_or("consumer record is not in store")?,
        ),
        record.env_vars.clone(),
    ));
    Ok(result)
}

fn resolve_key(layers: &[(Owner, BTreeMap<String, String>)], key: &str) -> Option<ResolvedValue> {
    layers.iter().rev().find_map(|(owner, env)| {
        env.get(key).map(|value| ResolvedValue {
            value: value.clone(),
            owner: owner.clone(),
        })
    })
}

fn classify_endpoint(endpoint: Option<&str>) -> Result<Identity, String> {
    let Some(value) = endpoint.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(Identity::Official);
    };
    let url = url::Url::parse(value).map_err(|_| "invalid OpenAI endpoint".to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("invalid OpenAI endpoint".to_string());
    }
    let official = url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|h| h.eq_ignore_ascii_case("api.openai.com"))
        && url.port_or_known_default() == Some(443)
        && matches!(url.path().trim_end_matches('/'), "" | "/v1");
    Ok(if official {
        Identity::Official
    } else {
        Identity::Compatible
    })
}

fn is_buzz_agent_consumer(record: &ManagedAgentRecord, defs: &[AgentDefinition]) -> bool {
    crate::managed_agents::known_acp_runtime(&crate::managed_agents::record_agent_command(
        record, defs,
    ))
    .is_some_and(|runtime| runtime.id == "buzz-agent")
}

fn global_defaults_target_buzz_agent(store: &Store) -> bool {
    match store.global.preferred_runtime.as_deref().map(str::trim) {
        Some("") | None => true,
        Some(runtime_id) => crate::managed_agents::known_acp_runtime_exact(runtime_id)
            .is_some_and(|runtime| runtime.id == "buzz-agent"),
    }
}

fn collect_consumers(store: &Store) -> Result<Vec<Consumer>, String> {
    let defs = definitions(store);
    let mut consumers = Vec::new();
    if global_defaults_target_buzz_agent(store) {
        if let Some(provider) = store
            .global
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .filter(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "openai" | "openai-compat"
                )
            })
        {
            let endpoint = store
                .global
                .env_vars
                .get(OPENAI_COMPAT_BASE_URL)
                .map(|value| ResolvedValue {
                    value: value.clone(),
                    owner: Owner::Global,
                });
            consumers.push(Consumer {
                label: "global defaults".into(),
                record_index: None,
                provider: provider.to_ascii_lowercase(),
                provider_owner: Owner::Global,
                identity: classify_endpoint(endpoint.as_ref().map(|value| value.value.as_str()))?,
                endpoint,
                legacy_key: store
                    .global
                    .env_vars
                    .get(OPENAI_COMPAT_API_KEY)
                    .map(|value| ResolvedValue {
                        value: value.clone(),
                        owner: Owner::Global,
                    }),
                official_key: store.global.env_vars.get(OPENAI_API_KEY).map(|value| {
                    ResolvedValue {
                        value: value.clone(),
                        owner: Owner::Global,
                    }
                }),
            });
        }
    }
    for (index, record) in store.records.iter().enumerate() {
        if record.pubkey.is_empty() || !is_buzz_agent_consumer(record, &defs) {
            continue;
        }
        let def = selected_definition(record, &defs);
        if record.persona_id.is_some() && def.is_none() {
            return Err(format!(
                "cannot resolve linked definition {:?}",
                record.persona_id
            ));
        }
        let effective = resolve_effective_config(record, &defs, &store.global);
        let config = match effective {
            EffectiveConfigResult::Resolved(config) => config,
            EffectiveConfigResult::OrphanedInstance {
                missing_persona_id, ..
            } => {
                return Err(format!(
                    "cannot resolve linked definition {missing_persona_id:?}"
                ))
            }
        };
        let Some(provider) = config
            .provider
            .value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        else {
            continue;
        };
        if !matches!(
            provider.to_ascii_lowercase().as_str(),
            "openai" | "openai-compat"
        ) {
            continue;
        }
        let provider_owner = match config.provider.source {
            ConfigSource::Global => Owner::Global,
            ConfigSource::Definition => {
                let id = record
                    .persona_id
                    .as_deref()
                    .or(record.slug.as_deref())
                    .ok_or("definition provider has no owner")?;
                store
                    .records
                    .iter()
                    .position(|r| r.pubkey.is_empty() && r.slug.as_deref() == Some(id))
                    .map(Owner::Record)
                    .unwrap_or_else(|| Owner::BuiltinDefinition(id.to_string()))
            }
            ConfigSource::InstanceLegacy => Owner::Record(index),
        };
        let resolved_layers = layers(store, record, def)?;
        let endpoint = resolve_key(&resolved_layers, OPENAI_COMPAT_BASE_URL);
        let identity = classify_endpoint(endpoint.as_ref().map(|v| v.value.as_str()))?;
        consumers.push(Consumer {
            label: record
                .slug
                .clone()
                .or_else(|| (!record.pubkey.is_empty()).then(|| record.pubkey.clone()))
                .unwrap_or_else(|| format!("record-{index}")),
            record_index: Some(index),
            provider: provider.to_ascii_lowercase(),
            provider_owner,
            identity,
            endpoint,
            legacy_key: resolve_key(&resolved_layers, OPENAI_COMPAT_API_KEY),
            official_key: resolve_key(&resolved_layers, OPENAI_API_KEY),
        });
    }
    if consumers.is_empty()
        && global_defaults_target_buzz_agent(store)
        && [OPENAI_COMPAT_API_KEY, OPENAI_COMPAT_BASE_URL]
            .iter()
            .any(|key| store.global.env_vars.contains_key(*key))
    {
        return Err("global OpenAI-compatible state has no provider owner".into());
    }
    Ok(consumers)
}

fn env_mut(store: &mut Store, owner: Owner) -> Result<&mut BTreeMap<String, String>, String> {
    match owner {
        Owner::Global => Ok(&mut store.global.env_vars),
        Owner::Record(i) => Ok(&mut store
            .records
            .get_mut(i)
            .ok_or("record owner is missing")?
            .env_vars),
        Owner::Baked | Owner::Harness(_) | Owner::BuiltinDefinition(_) => {
            Err("required mutation is owned by a read-only harness/build layer".into())
        }
    }
}

fn set_provider(store: &mut Store, owner: Owner, provider: &str) -> Result<(), String> {
    match owner {
        Owner::Global => {
            store.global.provider = Some(provider.into());
            Ok(())
        }
        Owner::Record(i) => {
            store
                .records
                .get_mut(i)
                .ok_or("record owner is missing")?
                .provider = Some(provider.into());
            Ok(())
        }
        Owner::Baked | Owner::Harness(_) | Owner::BuiltinDefinition(_) => {
            Err("required provider mutation is read-only".into())
        }
    }
}

fn shared_with_non_buzz_consumer(store: &Store, owner: &Owner) -> bool {
    if owner != &Owner::Global {
        return false;
    }
    let defs = definitions(store);
    store.records.iter().any(|record| {
        !record.pubkey.is_empty()
            && !is_buzz_agent_consumer(record, &defs)
            && resolve_effective_config(record, &defs, &store.global)
                .require_resolved()
                .is_ok_and(|config| config.provider.source == ConfigSource::Global)
    })
}

fn plan_store(mut store: Store) -> Result<(Store, Vec<Consumer>), String> {
    let consumers = collect_consumers(&store)?;
    let mut provider_targets: HashMap<Owner, (Option<Identity>, bool)> = HashMap::new();
    for consumer in &consumers {
        let expected = if consumer.identity == Identity::Official {
            "openai"
        } else {
            "openai-compat"
        };
        match provider_targets.entry(consumer.provider_owner.clone()) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert((Some(consumer.identity), consumer.provider != expected));
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if entry.get().0 != Some(consumer.identity) {
                    entry.get_mut().0 = None;
                } else {
                    entry.get_mut().1 |= consumer.provider != expected;
                }
            }
        }
    }
    for (owner, (identity, needs_change)) in provider_targets {
        if let (Some(identity), true) = (identity, needs_change) {
            if !shared_with_non_buzz_consumer(&store, &owner) {
                set_provider(
                    &mut store,
                    owner,
                    if identity == Identity::Official {
                        "openai"
                    } else {
                        "openai-compat"
                    },
                )?;
            }
        }
    }
    for consumer in &consumers {
        let provider = if consumer.identity == Identity::Official {
            "openai"
        } else {
            "openai-compat"
        };
        let owner = consumer
            .record_index
            .map(Owner::Record)
            .unwrap_or(Owner::Global);
        env_mut(&mut store, owner)?.insert(
            crate::managed_agents::openai_env::MIGRATED_OPENAI_PROVIDER_ENV.into(),
            provider.into(),
        );
    }

    let mut official_legacy_targets = Vec::new();
    for consumer in &consumers {
        if consumer.identity != Identity::Official
            || consumer
                .official_key
                .as_ref()
                .is_some_and(|v| !v.value.is_empty())
        {
            continue;
        }
        if let Some(legacy) = &consumer.legacy_key {
            official_legacy_targets.push(legacy.owner.clone());
        }
    }
    official_legacy_targets.sort_by_key(|owner| format!("{owner:?}"));
    official_legacy_targets.dedup();
    for owner in official_legacy_targets {
        let env = env_mut(&mut store, owner)?;
        if env.get(OPENAI_API_KEY).is_none_or(|value| value.is_empty()) {
            if let Some(value) = env.get(OPENAI_COMPAT_API_KEY).cloned() {
                env.insert(OPENAI_API_KEY.into(), value);
            }
        }
    }

    // Official aliases and inherited custom origins must be shadowed with the one
    // spelling accepted by both readiness and buzz-agent. Plan this by the
    // shared endpoint owner: changing a lower alias is forbidden when it also
    // routes a custom/non-OpenAI consumer.
    let mut alias_owners = Vec::new();
    for consumer in &consumers {
        if consumer.identity != Identity::Official {
            continue;
        }
        if let Some(endpoint) = &consumer.endpoint {
            if endpoint.value.trim().trim_end_matches('/') != CANONICAL_OPENAI_ORIGIN {
                alias_owners.push(endpoint.owner.clone());
            }
        }
    }
    alias_owners.sort_by_key(|owner| format!("{owner:?}"));
    alias_owners.dedup();
    for owner in alias_owners {
        env_mut(&mut store, owner)?.insert(
            OPENAI_COMPAT_BASE_URL.into(),
            CANONICAL_OPENAI_ORIGIN.into(),
        );
    }

    // A canonical local endpoint may be the only thing hiding a lower custom
    // route. Removing it would make official OpenAI fail closed after restart,
    // so leave every already-canonical shadow intact.
    Ok((store, consumers))
}

fn serialize_store(
    store: &Store,
    source_global: Option<&[u8]>,
    source_managed: Option<&[u8]>,
) -> Result<SerializedStore, String> {
    let global = if source_global.is_some() || store.global != GlobalAgentConfig::default() {
        Some(serde_json::to_vec_pretty(&store.global).map_err(|e| e.to_string())?)
    } else {
        None
    };
    let managed = if source_managed.is_some() || !store.records.is_empty() {
        Some(serde_json::to_vec_pretty(&store.records).map_err(|e| e.to_string())?)
    } else {
        None
    };
    Ok((global, managed))
}

fn verify_post_split(before: &[Consumer], after: &Store) -> Result<(), String> {
    let post = collect_consumers(after)?;
    let defs = definitions(after);
    for old in before {
        let new = post
            .iter()
            .find(|c| c.record_index == old.record_index)
            .ok_or_else(|| format!("consumer {:?} disappeared", old.label))?;
        if new.identity != old.identity {
            return Err(format!(
                "consumer {:?} changed endpoint identity",
                old.label
            ));
        }
        let expected_provider = if old.identity == Identity::Official {
            "openai"
        } else {
            "openai-compat"
        };
        if old.record_index.is_none() {
            if new.provider != expected_provider {
                return Err(format!(
                    "consumer {:?} resolves provider {:?}, expected {expected_provider}",
                    old.label, new.provider
                ));
            }
            let expected_key = if old.identity == Identity::Official {
                old.official_key
                    .as_ref()
                    .filter(|value| !value.value.is_empty())
                    .or(old.legacy_key.as_ref())
            } else {
                old.legacy_key.as_ref()
            };
            let selected_key = if old.identity == Identity::Official {
                new.official_key.as_ref()
            } else {
                new.legacy_key.as_ref()
            };
            if expected_key.map(|value| &value.value) != selected_key.map(|value| &value.value) {
                return Err(format!(
                    "consumer {:?} changed selected credential",
                    old.label
                ));
            }
            continue;
        }
        let record_index = old
            .record_index
            .ok_or("managed consumer has no record index")?;
        let descriptor = crate::managed_agents::resolve_effective_harness_descriptor(
            &after.records[record_index],
            &defs,
            &after.global,
        )
        .map_err(|e| format!("consumer {:?} harness resolution failed: {e}", old.label))?;
        let old_selected = if old.provider == "openai" {
            old.official_key.as_ref().or(old.legacy_key.as_ref())
        } else {
            old.legacy_key.as_ref()
        };
        let effective = crate::managed_agents::resolve_effective_agent_env(
            &after.records[record_index],
            &defs,
            crate::managed_agents::known_acp_runtime(&descriptor.command),
            &after.global,
        );
        let readiness = crate::managed_agents::agent_readiness(&effective);
        let readiness_failure = match &readiness {
            crate::managed_agents::AgentReadiness::Ready => false,
            crate::managed_agents::AgentReadiness::NotReady { requirements } => {
                requirements.iter().any(|requirement| match requirement {
                    crate::managed_agents::Requirement::ConfigInvalid { .. } => true,
                    crate::managed_agents::Requirement::EnvKey { key } => {
                        key == if old.identity == Identity::Official {
                            OPENAI_API_KEY
                        } else {
                            OPENAI_COMPAT_BASE_URL
                        }
                    }
                    _ => false,
                })
            }
        };
        if old_selected.is_some() && readiness_failure {
            return Err(format!(
                "consumer {:?} fails post-split readiness",
                old.label
            ));
        }
        let selected = if old.identity == Identity::Official {
            descriptor.env.get(OPENAI_API_KEY)
        } else {
            descriptor.env.get(OPENAI_COMPAT_API_KEY)
        };
        let expected = if old.identity == Identity::Official {
            old.official_key
                .as_ref()
                .filter(|v| !v.value.is_empty())
                .or(old.legacy_key.as_ref())
                .map(|v| &v.value)
        } else {
            old.legacy_key.as_ref().map(|v| &v.value)
        };
        if old_selected.is_some() && selected != expected {
            return Err(format!(
                "consumer {:?} changed selected credential",
                old.label
            ));
        }
        if old.identity == Identity::Official {
            let origin = descriptor
                .env
                .get(OPENAI_COMPAT_BASE_URL)
                .map(String::as_str);
            if origin.is_some_and(|v| v.trim().trim_end_matches('/') != CANONICAL_OPENAI_ORIGIN) {
                return Err(format!(
                    "consumer {:?} retains rejected official origin",
                    old.label
                ));
            }
        } else if classify_endpoint(
            descriptor
                .env
                .get(OPENAI_COMPAT_BASE_URL)
                .map(String::as_str),
        )? != Identity::Compatible
        {
            return Err(format!("consumer {:?} lost its custom origin", old.label));
        }
    }
    Ok(())
}

fn make_plan(global: Option<&[u8]>, managed: Option<&[u8]>) -> Result<PlannedStore, String> {
    let source = parse_store(global, managed)?;
    let (store, consumers) = plan_store(source)?;
    verify_post_split(&consumers, &store)?;
    let (global, managed) = serialize_store(&store, global, managed)?;
    Ok(PlannedStore {
        global,
        managed,
        consumers,
    })
}

fn current_hash(path: &Path) -> Result<Option<String>, String> {
    Ok(hash(read_optional(path)?.as_deref()))
}

fn apply_target(path: &Path, target: Option<&[u8]>) -> Result<(), String> {
    match target {
        Some(bytes) => crate::managed_agents::atomic_write_json_restricted(path, bytes),
        None if path.exists() => std::fs::remove_file(path)
            .map_err(|e| format!("failed to remove {}: {e}", path.display())),
        None => Ok(()),
    }
}

fn verify_written_targets(
    expected: &TransactionManifest,
    consumers: &[Consumer],
    global: Option<&[u8]>,
    managed: Option<&[u8]>,
) -> Result<(), String> {
    if hash(global) != expected.global.target || hash(managed) != expected.managed.target {
        return Err("migration target hash mismatch after write".into());
    }
    let reread = parse_store(global, managed)?;
    verify_post_split(consumers, &reread)
}

fn migrate_pair(agents_dir: &Path) -> Result<(), String> {
    let marker = agents_dir.join(MARKER_FILE);
    if marker.is_file()
        && std::fs::read(&marker)
            .ok()
            .as_deref()
            .is_some_and(|bytes| bytes == b"1\n")
    {
        return Ok(());
    }
    let global_path = agents_dir.join("global-agent-config.json");
    let managed_path = agents_dir.join("managed-agents.json");
    let manifest_path = agents_dir.join(MANIFEST_FILE);

    let (source_global, source_managed, manifest) = if manifest_path.exists() {
        let manifest: TransactionManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("invalid migration manifest: {e}"))?;
        for (path, hashes) in [
            (&global_path, &manifest.global),
            (&managed_path, &manifest.managed),
        ] {
            let current = current_hash(path)?;
            if current != hashes.source && current != hashes.target {
                return Err(format!(
                    "external edit detected during OpenAI credential migration: {}",
                    path.display()
                ));
            }
        }
        let source_global = if manifest.global.source.is_some() {
            read_optional(&sibling_path(&global_path, BACKUP_SUFFIX))?
        } else {
            None
        };
        let source_managed = if manifest.managed.source.is_some() {
            read_optional(&sibling_path(&managed_path, BACKUP_SUFFIX))?
        } else {
            None
        };
        (source_global, source_managed, Some(manifest))
    } else {
        (
            read_optional(&global_path)?,
            read_optional(&managed_path)?,
            None,
        )
    };
    if source_global.is_none() && source_managed.is_none() {
        return Ok(());
    }
    let plan = make_plan(source_global.as_deref(), source_managed.as_deref())?;
    let expected = TransactionManifest {
        global: FileHashes {
            source: hash(source_global.as_deref()),
            target: hash(plan.global.as_deref()),
        },
        managed: FileHashes {
            source: hash(source_managed.as_deref()),
            target: hash(plan.managed.as_deref()),
        },
    };
    if let Some(existing) = manifest {
        if existing.global.source != expected.global.source
            || existing.global.target != expected.global.target
            || existing.managed.source != expected.managed.source
            || existing.managed.target != expected.managed.target
        {
            return Err("migration manifest does not match pristine backups".into());
        }
    } else {
        if let Some(bytes) = &source_global {
            let backup_path = sibling_path(&global_path, BACKUP_SUFFIX);
            crate::util::create_restricted_backup_once(&backup_path, bytes)
                .map_err(|e| e.to_string())?;
            if read_optional(&backup_path)?.as_deref() != Some(bytes.as_slice()) {
                return Err(format!(
                    "existing migration backup does not match pristine source: {}",
                    backup_path.display()
                ));
            }
        }
        if let Some(bytes) = &source_managed {
            let backup_path = sibling_path(&managed_path, BACKUP_SUFFIX);
            crate::util::create_restricted_backup_once(&backup_path, bytes)
                .map_err(|e| e.to_string())?;
            if read_optional(&backup_path)?.as_deref() != Some(bytes.as_slice()) {
                return Err(format!(
                    "existing migration backup does not match pristine source: {}",
                    backup_path.display()
                ));
            }
        }
        let bytes = serde_json::to_vec_pretty(&expected).map_err(|e| e.to_string())?;
        crate::managed_agents::atomic_write_json_restricted(&manifest_path, &bytes)?;
    }
    if current_hash(&global_path)? == expected.global.source {
        apply_target(&global_path, plan.global.as_deref())?;
    }
    if current_hash(&managed_path)? == expected.managed.source {
        apply_target(&managed_path, plan.managed.as_deref())?;
    }
    let reread_global = read_optional(&global_path)?;
    let reread_managed = read_optional(&managed_path)?;
    verify_written_targets(
        &expected,
        &plan.consumers,
        reread_global.as_deref(),
        reread_managed.as_deref(),
    )?;
    crate::managed_agents::atomic_write_json_restricted(&marker, b"1\n")
}

pub(super) fn migrate_openai_credentials(app: &tauri::AppHandle) {
    let Ok(agents_dir) = crate::managed_agents::managed_agents_base_dir(app) else {
        return;
    };
    let custom_dir = agents_dir.parent().map(|p| p.join("custom_harnesses"));
    crate::managed_agents::custom_harnesses::warm_harness_registry_from_dir(custom_dir.as_deref());
    if let Err(error) = migrate_pair(&agents_dir) {
        eprintln!("buzz-desktop: openai-credential-migration: {error}");
    }
}

#[cfg(test)]
#[path = "openai_credentials_tests.rs"]
mod tests;
