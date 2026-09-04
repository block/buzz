//! Compatibility migration for the Bumble-to-Pollen built-in agent rename.

use std::path::Path;

use tauri::Manager;

use super::persona_version_from_record;

/// Rename the built-in research agent in persisted definitions and linked
/// instances without overwriting user-customized fields.
pub(super) fn migrate_pollen_agent_name(app: &tauri::AppHandle) {
    let Ok(dir) = app.path().app_data_dir() else {
        return;
    };
    let path = dir.join("agents/managed-agents.json");
    if path.exists() {
        migrate_pollen_agent_name_in_file(&path, &crate::util::now_iso());
    }
}

fn migrate_pollen_agent_name_in_file(path: &Path, now: &str) {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(mut records) = serde_json::from_str::<Vec<serde_json::Value>>(&contents) else {
        eprintln!(
            "buzz-desktop: migrate-pollen-agent-name: invalid JSON in {}",
            path.display()
        );
        return;
    };

    let mut version_updates = stock_version_updates(now);
    let has_stock_pollen_instance = records.iter().any(|record| {
        record
            .get("pubkey")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|key| !key.is_empty())
            && record.get("persona_id").and_then(serde_json::Value::as_str)
                == Some(crate::managed_agents::POLLEN_PERSONA_ID)
            && record.get("name").and_then(serde_json::Value::as_str)
                == Some(crate::managed_agents::POLLEN_LEGACY_DISPLAY_NAME)
    });
    let mut occupied_names = records
        .iter()
        .filter_map(|record| record.get("name").and_then(serde_json::Value::as_str))
        .map(|name| name.to_lowercase())
        .collect::<std::collections::HashSet<_>>();
    let mut profile_reconciliations = Vec::new();
    let mut changed = false;

    // Migrate the definition first so an in-sync linked instance can advance
    // its source version instead of surfacing a false out-of-date warning.
    for record in &mut records {
        let is_definition = record
            .get("pubkey")
            .and_then(serde_json::Value::as_str)
            .is_some_and(str::is_empty);
        let Some(persona_id) = record
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        if !is_definition {
            continue;
        }

        let old_version = persona_version_from_record(record);
        let Some(object) = record.as_object_mut() else {
            continue;
        };
        let record_changed = if persona_id == crate::managed_agents::POLLEN_PERSONA_ID {
            migrate_pollen_fields(object, true)
        } else if persona_id == "builtin:fizz" {
            remove_pollen_from_legacy_fizz_name_pool(object)
        } else {
            false
        };
        if !record_changed {
            continue;
        }

        object.insert(
            "updated_at".to_string(),
            serde_json::Value::String(now.to_string()),
        );
        changed = true;
        if let (Some(old_version), Some(new_version)) =
            (old_version, persona_version_from_record(record))
        {
            version_updates.insert(persona_id, (old_version, new_version));
        }
    }

    for record in &mut records {
        let is_instance = record
            .get("pubkey")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|pubkey| !pubkey.is_empty());
        let Some(persona_id) = record
            .get("persona_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let is_pollen_instance = persona_id == crate::managed_agents::POLLEN_PERSONA_ID;
        let is_legacy_fizz_pollen = has_stock_pollen_instance
            && persona_id == "builtin:fizz"
            && record.get("name").and_then(serde_json::Value::as_str)
                == Some(crate::managed_agents::POLLEN_DISPLAY_NAME);
        // Definition rows are absent on direct upgrades from the pre-unified
        // persona store. The stock hashes still let pristine linked instances
        // advance instead of appearing falsely out of date after seeding.
        let version_update = version_updates.get(&persona_id);
        if !is_instance || (!is_pollen_instance && version_update.is_none()) {
            continue;
        }

        let source_was_current = version_update.is_some_and(|(old, _)| {
            record
                .get("persona_source_version")
                .and_then(serde_json::Value::as_str)
                == Some(old.as_str())
        });
        let Some(object) = record.as_object_mut() else {
            continue;
        };
        let name_was_migrated = is_pollen_instance
            && object.get("name").and_then(serde_json::Value::as_str)
                == Some(crate::managed_agents::POLLEN_LEGACY_DISPLAY_NAME);
        let mut record_changed = is_pollen_instance && migrate_pollen_fields(object, false);
        if is_legacy_fizz_pollen && source_was_current {
            let replacement = unique_legacy_fizz_name(&occupied_names);
            occupied_names.insert(replacement.to_lowercase());
            object.insert(
                "name".to_string(),
                serde_json::Value::String(replacement.clone()),
            );
            if let Some(pubkey) = object
                .get("pubkey")
                .and_then(serde_json::Value::as_str)
                .filter(|pubkey| !pubkey.is_empty())
            {
                profile_reconciliations.push((pubkey.to_string(), replacement));
            }
            record_changed = true;
        }
        if name_was_migrated {
            if let Some(pubkey) = object
                .get("pubkey")
                .and_then(serde_json::Value::as_str)
                .filter(|pubkey| !pubkey.is_empty())
            {
                profile_reconciliations.push((
                    pubkey.to_string(),
                    crate::managed_agents::POLLEN_DISPLAY_NAME.to_string(),
                ));
            }
        }
        if source_was_current {
            if let Some((_, new_version)) = version_update {
                object.insert(
                    "persona_source_version".to_string(),
                    serde_json::Value::String(new_version.clone()),
                );
                record_changed = true;
            }
        }
        if record_changed {
            object.insert(
                "updated_at".to_string(),
                serde_json::Value::String(now.to_string()),
            );
            changed = true;
        }
    }

    if !profile_reconciliations.is_empty() {
        // Queue first: a crash after this write but before the agent-store write
        // leaves harmless stale items. The loader verifies each queued expected
        // name against the durable record before publishing.
        if let Err(error) = persist_profile_reconcile_queue(path, &profile_reconciliations) {
            eprintln!("buzz-desktop: migrate-pollen-agent-name: {error}");
            return;
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&records) {
            if let Err(error) = crate::managed_agents::atomic_write_json_restricted(path, &bytes) {
                eprintln!("buzz-desktop: migrate-pollen-agent-name: {error}");
            }
        }
    } else if changed {
        if let Ok(bytes) = serde_json::to_vec_pretty(&records) {
            if let Err(error) = crate::managed_agents::atomic_write_json_restricted(path, &bytes) {
                eprintln!("buzz-desktop: migrate-pollen-agent-name: {error}");
            }
        }
    }
}

fn unique_legacy_fizz_name(occupied_names: &std::collections::HashSet<String>) -> String {
    let base = "Pollen-Fizz";
    if !occupied_names.contains(&base.to_lowercase()) {
        return base.to_string();
    }
    for suffix in 2.. {
        let candidate = format!("{base}-{suffix}");
        if !occupied_names.contains(&candidate.to_lowercase()) {
            return candidate;
        }
    }
    unreachable!()
}

fn stock_version_updates(now: &str) -> std::collections::HashMap<String, (String, String)> {
    let mut updates = std::collections::HashMap::new();

    if let Some(mut legacy_pollen) = crate::managed_agents::built_in_persona_definition(
        crate::managed_agents::POLLEN_PERSONA_ID,
        now,
    ) {
        let current_pollen = persona_version(&legacy_pollen);
        legacy_pollen.display_name = crate::managed_agents::POLLEN_LEGACY_DISPLAY_NAME.to_string();
        legacy_pollen.system_prompt =
            crate::managed_agents::POLLEN_LEGACY_SYSTEM_PROMPT.to_string();
        legacy_pollen.name_pool =
            vec![crate::managed_agents::POLLEN_LEGACY_DISPLAY_NAME.to_string()];
        updates.insert(
            crate::managed_agents::POLLEN_PERSONA_ID.to_string(),
            (persona_version(&legacy_pollen), current_pollen),
        );
    }

    if let Some(mut legacy_fizz) =
        crate::managed_agents::built_in_persona_definition("builtin:fizz", now)
    {
        let current_fizz = persona_version(&legacy_fizz);
        legacy_fizz
            .name_pool
            .insert(4, crate::managed_agents::POLLEN_DISPLAY_NAME.to_string());
        updates.insert(
            "builtin:fizz".to_string(),
            (persona_version(&legacy_fizz), current_fizz),
        );
    }

    updates
}

fn persona_version(definition: &crate::managed_agents::AgentDefinition) -> String {
    crate::managed_agents::persona_events::persona_content_hash(
        &crate::managed_agents::persona_events::persona_event_content(definition),
    )
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub(crate) struct ProfileReconcileQueueEntry {
    pub(crate) pubkey: String,
    #[serde(default = "default_profile_reconcile_name")]
    pub(crate) expected_name: String,
    /// Canonical relay identities already repaired for this migrated agent.
    ///
    /// Keep the entry after success: Desktop does not persist its community
    /// list in Rust, so a community that is inactive (or re-added later) must
    /// still get one repair when it is next applied.
    #[serde(default)]
    pub(crate) reconciled_relays: Vec<String>,
}

fn default_profile_reconcile_name() -> String {
    crate::managed_agents::POLLEN_DISPLAY_NAME.to_string()
}

#[derive(serde::Deserialize)]
struct CurrentProfileReconcileQueueEntry {
    pubkey: String,
    #[serde(default = "default_profile_reconcile_name")]
    expected_name: String,
    #[serde(default)]
    reconciled_relays: Vec<String>,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum StoredProfileReconcileQueueEntry {
    Current(CurrentProfileReconcileQueueEntry),
    Legacy(String),
}

impl<'de> serde::Deserialize<'de> for ProfileReconcileQueueEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match StoredProfileReconcileQueueEntry::deserialize(deserializer)? {
            StoredProfileReconcileQueueEntry::Current(entry) => Ok(Self {
                pubkey: entry.pubkey,
                expected_name: entry.expected_name,
                reconciled_relays: entry.reconciled_relays,
            }),
            StoredProfileReconcileQueueEntry::Legacy(pubkey) => Ok(Self {
                pubkey,
                expected_name: default_profile_reconcile_name(),
                reconciled_relays: Vec::new(),
            }),
        }
    }
}

pub(crate) fn profile_reconcile_queue_path(agent_store_path: &Path) -> std::path::PathBuf {
    agent_store_path.with_file_name("profile-reconcile-pending.json")
}

fn persist_profile_reconcile_queue(
    path: &Path,
    reconciliations: &[(String, String)],
) -> Result<(), String> {
    let queue_path = profile_reconcile_queue_path(path);
    let mut pending = if queue_path.exists() {
        read_profile_reconcile_queue(&queue_path).unwrap_or_default()
    } else {
        Vec::new()
    };
    for (pubkey, expected_name) in reconciliations {
        if let Some(entry) = pending.iter_mut().find(|entry| entry.pubkey == *pubkey) {
            entry.expected_name.clone_from(expected_name);
            entry.reconciled_relays.clear();
        } else {
            pending.push(ProfileReconcileQueueEntry {
                pubkey: pubkey.clone(),
                expected_name: expected_name.clone(),
                reconciled_relays: Vec::new(),
            });
        }
    }
    pending.sort_by(|left, right| left.pubkey.cmp(&right.pubkey));
    write_profile_reconcile_queue(&queue_path, &pending)
}

pub(crate) const PROFILE_RECONCILE_QUEUE_MAX_BYTES: usize = 1024 * 1024;

pub(crate) fn read_profile_reconcile_queue(
    path: &Path,
) -> Result<Vec<ProfileReconcileQueueEntry>, String> {
    let metadata = std::fs::metadata(path)
        .map_err(|error| format!("failed to inspect profile reconcile queue: {error}"))?;
    if metadata.len() > PROFILE_RECONCILE_QUEUE_MAX_BYTES as u64 {
        return Err("profile reconcile queue exceeds its size limit".to_string());
    }
    let contents = std::fs::read_to_string(path)
        .map_err(|error| format!("failed to read profile reconcile queue: {error}"))?;
    serde_json::from_str(&contents)
        .map_err(|error| format!("failed to parse profile reconcile queue: {error}"))
}

pub(crate) fn write_profile_reconcile_queue(
    path: &Path,
    entries: &[ProfileReconcileQueueEntry],
) -> Result<(), String> {
    if entries.is_empty() {
        return match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(format!(
                "failed to remove empty profile reconcile queue {}: {error}",
                path.display()
            )),
        };
    }
    let bytes = serde_json::to_vec_pretty(entries)
        .map_err(|error| format!("failed to serialize profile reconcile queue: {error}"))?;
    if bytes.len() > PROFILE_RECONCILE_QUEUE_MAX_BYTES {
        return Err("profile reconcile queue exceeds its size limit".to_string());
    }
    crate::managed_agents::atomic_write_json_restricted(path, &bytes)
}

pub(crate) fn profile_reconcile_relay_key(relay_url: &str) -> Result<String, String> {
    buzz_core_pkg::relay::normalize_relay_url(relay_url)
        .map_err(|error| format!("invalid profile reconcile relay: {error}"))
}

#[cfg(test)]
pub(crate) fn profile_reconcile_is_pending(
    entries: &[ProfileReconcileQueueEntry],
    pubkey: &str,
    relay_key: &str,
) -> bool {
    entries.iter().any(|entry| {
        entry.pubkey == pubkey
            && !entry
                .reconciled_relays
                .iter()
                .any(|relay| relay == relay_key)
    })
}

pub(crate) fn record_profile_reconciled(
    entries: &mut [ProfileReconcileQueueEntry],
    pubkey: &str,
    relay_key: String,
) {
    if let Some(entry) = entries.iter_mut().find(|entry| entry.pubkey == pubkey) {
        if !entry.reconciled_relays.contains(&relay_key) {
            entry.reconciled_relays.push(relay_key);
            entry.reconciled_relays.sort();
        }
    }
}

fn migrate_pollen_fields(
    record: &mut serde_json::Map<String, serde_json::Value>,
    is_definition: bool,
) -> bool {
    let mut changed = false;
    for key in ["name", "display_name"] {
        if record.get(key).and_then(serde_json::Value::as_str)
            == Some(crate::managed_agents::POLLEN_LEGACY_DISPLAY_NAME)
        {
            record.insert(
                key.to_string(),
                serde_json::Value::String(crate::managed_agents::POLLEN_DISPLAY_NAME.to_string()),
            );
            changed = true;
        }
    }
    if record
        .get("system_prompt")
        .and_then(serde_json::Value::as_str)
        == Some(crate::managed_agents::POLLEN_LEGACY_SYSTEM_PROMPT)
    {
        record.insert(
            "system_prompt".to_string(),
            serde_json::Value::String(crate::managed_agents::POLLEN_SYSTEM_PROMPT.to_string()),
        );
        changed = true;
    }
    if is_definition
        && record
            .get("name_pool")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|names| {
                names.len() == 1
                    && names[0].as_str() == Some(crate::managed_agents::POLLEN_LEGACY_DISPLAY_NAME)
            })
    {
        record.insert(
            "name_pool".to_string(),
            serde_json::json!([crate::managed_agents::POLLEN_DISPLAY_NAME]),
        );
        changed = true;
    }
    changed
}

fn remove_pollen_from_legacy_fizz_name_pool(
    record: &mut serde_json::Map<String, serde_json::Value>,
) -> bool {
    const LEGACY_FIZZ_NAME_POOL: &[&str] = &[
        "Nectar", "Comet", "Bramble", "Clover", "Pollen", "Amber", "Daisy", "Mason", "Thistle",
        "Waxwing", "Hive", "Meadow", "Juniper", "Aster", "Sage", "Willow", "Orchard", "Buzz",
    ];
    let Some(names) = record
        .get("name_pool")
        .and_then(serde_json::Value::as_array)
    else {
        return false;
    };
    if !names
        .iter()
        .map(|name| name.as_str())
        .eq(LEGACY_FIZZ_NAME_POOL.iter().copied().map(Some))
    {
        return false;
    }

    let names_without_pollen = names
        .iter()
        .filter(|name| name.as_str() != Some(crate::managed_agents::POLLEN_DISPLAY_NAME))
        .cloned()
        .collect();
    record.insert(
        "name_pool".to_string(),
        serde_json::Value::Array(names_without_pollen),
    );
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::test_support::{read_agents_json, write_agents_json};

    #[test]
    fn pollen_name_migration_updates_seeded_fields_and_preserves_customizations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agents/managed-agents.json");
        let mut legacy_definition = crate::managed_agents::built_in_persona_definition(
            crate::managed_agents::POLLEN_PERSONA_ID,
            "before",
        )
        .unwrap();
        legacy_definition.display_name =
            crate::managed_agents::POLLEN_LEGACY_DISPLAY_NAME.to_string();
        legacy_definition.system_prompt =
            crate::managed_agents::POLLEN_LEGACY_SYSTEM_PROMPT.to_string();
        legacy_definition.name_pool =
            vec![crate::managed_agents::POLLEN_LEGACY_DISPLAY_NAME.to_string()];
        let old_version = crate::managed_agents::persona_events::persona_content_hash(
            &crate::managed_agents::persona_events::persona_event_content(&legacy_definition),
        );
        let mut current_definition = legacy_definition.clone();
        current_definition.display_name = crate::managed_agents::POLLEN_DISPLAY_NAME.to_string();
        current_definition.system_prompt = crate::managed_agents::POLLEN_SYSTEM_PROMPT.to_string();
        current_definition.name_pool = vec![crate::managed_agents::POLLEN_DISPLAY_NAME.to_string()];
        let new_version = crate::managed_agents::persona_events::persona_content_hash(
            &crate::managed_agents::persona_events::persona_event_content(&current_definition),
        );

        let mut definition_record =
            serde_json::to_value(legacy_definition.into_agent_record()).unwrap();
        definition_record["future_definition_field"] = serde_json::json!("preserved");
        let pristine_instance = serde_json::json!({
            "pubkey": "pristine-pubkey",
            "name": crate::managed_agents::POLLEN_LEGACY_DISPLAY_NAME,
            "persona_id": crate::managed_agents::POLLEN_PERSONA_ID,
            "system_prompt": crate::managed_agents::POLLEN_LEGACY_SYSTEM_PROMPT,
            "persona_source_version": old_version,
            "start_on_app_launch": false,
            "updated_at": "before",
            "future_instance_field": "preserved"
        });
        let customized_instance = serde_json::json!({
            "pubkey": "customized-pubkey",
            "name": "My researcher",
            "persona_id": crate::managed_agents::POLLEN_PERSONA_ID,
            "system_prompt": "User-edited instructions",
            "persona_source_version": "custom-version",
            "updated_at": "before"
        });
        let unrelated = serde_json::json!({
            "pubkey": "honey-pubkey",
            "name": "Honey",
            "persona_id": "builtin:honey",
            "system_prompt": "You are Honey.",
            "updated_at": "before"
        });
        write_agents_json(
            dir.path(),
            &serde_json::json!([
                definition_record,
                pristine_instance,
                customized_instance,
                unrelated
            ]),
        );

        migrate_pollen_agent_name_in_file(&path, "after");

        let records = read_agents_json(dir.path());
        assert_eq!(
            records[0]["slug"],
            crate::managed_agents::POLLEN_PERSONA_ID,
            "the persisted compatibility id must remain stable"
        );
        assert_eq!(
            records[0]["name"],
            crate::managed_agents::POLLEN_DISPLAY_NAME
        );
        assert_eq!(
            records[0]["display_name"],
            crate::managed_agents::POLLEN_DISPLAY_NAME
        );
        assert_eq!(
            records[0]["system_prompt"],
            crate::managed_agents::POLLEN_SYSTEM_PROMPT
        );
        assert_eq!(
            records[0]["name_pool"],
            serde_json::json!([crate::managed_agents::POLLEN_DISPLAY_NAME])
        );
        assert_eq!(records[0]["future_definition_field"], "preserved");
        assert_eq!(records[0]["updated_at"], "after");

        assert_eq!(
            records[1]["name"],
            crate::managed_agents::POLLEN_DISPLAY_NAME
        );
        assert_eq!(
            records[1]["system_prompt"],
            crate::managed_agents::POLLEN_SYSTEM_PROMPT
        );
        assert_eq!(records[1]["persona_source_version"], new_version);
        assert_eq!(records[1]["future_instance_field"], "preserved");
        assert_eq!(records[1]["updated_at"], "after");

        assert_eq!(records[2]["name"], "My researcher");
        assert_eq!(records[2]["system_prompt"], "User-edited instructions");
        assert_eq!(records[2]["persona_source_version"], "custom-version");
        assert_eq!(records[2]["updated_at"], "before");
        assert_eq!(records[3], unrelated);
        assert_eq!(
            read_profile_reconcile_queue(&profile_reconcile_queue_path(&path)).unwrap(),
            vec![ProfileReconcileQueueEntry {
                expected_name: crate::managed_agents::POLLEN_DISPLAY_NAME.to_string(),
                pubkey: "pristine-pubkey".to_string(),
                reconciled_relays: Vec::new(),
            }],
            "a stopped stock instance must retry its relay profile independently of startup"
        );

        let once = std::fs::read(&path).unwrap();
        migrate_pollen_agent_name_in_file(&path, "later");
        assert_eq!(
            std::fs::read(path).unwrap(),
            once,
            "migration is idempotent"
        );
    }

    #[test]
    fn pollen_name_migration_advances_stock_versions_without_definition_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agents/managed-agents.json");
        let updates = stock_version_updates("before");
        let (old_pollen, new_pollen) = updates
            .get(crate::managed_agents::POLLEN_PERSONA_ID)
            .unwrap();
        let (old_fizz, new_fizz) = updates.get("builtin:fizz").unwrap();
        write_agents_json(
            dir.path(),
            &serde_json::json!([
                {
                    "pubkey": "pollen-pubkey",
                    "name": crate::managed_agents::POLLEN_LEGACY_DISPLAY_NAME,
                    "persona_id": crate::managed_agents::POLLEN_PERSONA_ID,
                    "system_prompt": crate::managed_agents::POLLEN_LEGACY_SYSTEM_PROMPT,
                    "persona_source_version": old_pollen,
                    "start_on_app_launch": false,
                    "updated_at": "before"
                },
                {
                    "pubkey": "fizz-pubkey",
                    "name": "Fizz",
                    "persona_id": "builtin:fizz",
                    "persona_source_version": old_fizz,
                    "updated_at": "before"
                }
            ]),
        );

        migrate_pollen_agent_name_in_file(&path, "after");

        let records = read_agents_json(dir.path());
        assert_eq!(records[0]["persona_source_version"], *new_pollen);
        assert_eq!(records[1]["persona_source_version"], *new_fizz);
        assert_eq!(
            read_profile_reconcile_queue(&profile_reconcile_queue_path(&path)).unwrap(),
            vec![ProfileReconcileQueueEntry {
                expected_name: crate::managed_agents::POLLEN_DISPLAY_NAME.to_string(),
                pubkey: "pollen-pubkey".to_string(),
                reconciled_relays: Vec::new(),
            }]
        );
    }

    #[test]
    fn legacy_profile_reconcile_queue_remains_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile-reconcile-pending.json");
        std::fs::write(&path, r#"["pollen-pubkey"]"#).unwrap();

        assert_eq!(
            read_profile_reconcile_queue(&path).unwrap(),
            vec![ProfileReconcileQueueEntry {
                expected_name: crate::managed_agents::POLLEN_DISPLAY_NAME.to_string(),
                pubkey: "pollen-pubkey".to_string(),
                reconciled_relays: Vec::new(),
            }]
        );
    }

    #[test]
    fn profile_reconcile_queue_tracks_each_relay_without_dropping_other_communities() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile-reconcile-pending.json");
        let relay_a = profile_reconcile_relay_key("WSS://A.EXAMPLE:443/").unwrap();
        let relay_b = profile_reconcile_relay_key("wss://b.example").unwrap();
        let mut entries = vec![ProfileReconcileQueueEntry {
            pubkey: "pollen-pubkey".to_string(),
            expected_name: crate::managed_agents::POLLEN_DISPLAY_NAME.to_string(),
            reconciled_relays: Vec::new(),
        }];
        assert!(profile_reconcile_is_pending(
            &entries,
            "pollen-pubkey",
            &relay_a
        ));
        record_profile_reconciled(&mut entries, "pollen-pubkey", relay_a.clone());
        assert!(!profile_reconcile_is_pending(
            &entries,
            "pollen-pubkey",
            &relay_a
        ));
        assert!(profile_reconcile_is_pending(
            &entries,
            "pollen-pubkey",
            &relay_b
        ));

        write_profile_reconcile_queue(&path, &entries).unwrap();
        assert_eq!(read_profile_reconcile_queue(&path).unwrap(), entries);
        assert_eq!(
            profile_reconcile_relay_key("wss://a.example").unwrap(),
            profile_reconcile_relay_key("WSS://A.EXAMPLE:443/").unwrap(),
            "equivalent relay spellings must share one completion key"
        );
    }

    #[test]
    fn empty_profile_reconcile_queue_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile-reconcile-pending.json");
        write_profile_reconcile_queue(
            &path,
            &[ProfileReconcileQueueEntry {
                expected_name: crate::managed_agents::POLLEN_DISPLAY_NAME.to_string(),
                pubkey: "pollen-pubkey".to_string(),
                reconciled_relays: Vec::new(),
            }],
        )
        .unwrap();
        assert!(path.exists());

        write_profile_reconcile_queue(&path, &[]).unwrap();

        assert!(!path.exists());
    }

    #[test]
    fn pollen_name_migration_repairs_stock_fizz_collision_and_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agents/managed-agents.json");
        let updates = stock_version_updates("before");
        let old_pollen = &updates[crate::managed_agents::POLLEN_PERSONA_ID].0;
        let old_fizz = &updates["builtin:fizz"].0;
        write_agents_json(
            dir.path(),
            &serde_json::json!([
                {
                    "pubkey": "pollen-pubkey",
                    "name": crate::managed_agents::POLLEN_LEGACY_DISPLAY_NAME,
                    "persona_id": crate::managed_agents::POLLEN_PERSONA_ID,
                    "system_prompt": crate::managed_agents::POLLEN_LEGACY_SYSTEM_PROMPT,
                    "persona_source_version": old_pollen,
                    "updated_at": "before"
                },
                {
                    "pubkey": "fizz-pubkey",
                    "name": crate::managed_agents::POLLEN_DISPLAY_NAME,
                    "persona_id": "builtin:fizz",
                    "persona_source_version": old_fizz,
                    "updated_at": "before"
                },
                {
                    "pubkey": "occupied-pubkey",
                    "name": "pollen-fizz",
                    "persona_id": "custom:persona",
                    "updated_at": "before"
                },
                {
                    "pubkey": "custom-fizz-pubkey",
                    "name": crate::managed_agents::POLLEN_DISPLAY_NAME,
                    "persona_id": "builtin:fizz",
                    "persona_source_version": "custom-version",
                    "updated_at": "before"
                }
            ]),
        );

        migrate_pollen_agent_name_in_file(&path, "after");

        let records = read_agents_json(dir.path());
        assert_eq!(
            records[0]["name"],
            crate::managed_agents::POLLEN_DISPLAY_NAME
        );
        assert_eq!(records[1]["name"], "Pollen-Fizz-2");
        assert_eq!(records[2]["name"], "pollen-fizz");
        assert_eq!(
            records[3]["name"],
            crate::managed_agents::POLLEN_DISPLAY_NAME
        );
        assert_eq!(records[3]["updated_at"], "before");
        assert_eq!(
            read_profile_reconcile_queue(&profile_reconcile_queue_path(&path)).unwrap(),
            vec![
                ProfileReconcileQueueEntry {
                    pubkey: "fizz-pubkey".to_string(),
                    expected_name: "Pollen-Fizz-2".to_string(),
                    reconciled_relays: Vec::new(),
                },
                ProfileReconcileQueueEntry {
                    pubkey: "pollen-pubkey".to_string(),
                    expected_name: crate::managed_agents::POLLEN_DISPLAY_NAME.to_string(),
                    reconciled_relays: Vec::new(),
                },
            ]
        );

        let once = std::fs::read(&path).unwrap();
        migrate_pollen_agent_name_in_file(&path, "later");
        assert_eq!(std::fs::read(path).unwrap(), once);
    }

    #[test]
    fn pollen_name_migration_removes_the_new_name_from_the_legacy_fizz_pool() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agents/managed-agents.json");
        let mut legacy_fizz =
            crate::managed_agents::built_in_persona_definition("builtin:fizz", "before").unwrap();
        legacy_fizz
            .name_pool
            .insert(4, crate::managed_agents::POLLEN_DISPLAY_NAME.to_string());
        let old_version = crate::managed_agents::persona_events::persona_content_hash(
            &crate::managed_agents::persona_events::persona_event_content(&legacy_fizz),
        );
        let mut current_fizz = legacy_fizz.clone();
        current_fizz
            .name_pool
            .retain(|name| name != crate::managed_agents::POLLEN_DISPLAY_NAME);
        let new_version = crate::managed_agents::persona_events::persona_content_hash(
            &crate::managed_agents::persona_events::persona_event_content(&current_fizz),
        );
        write_agents_json(
            dir.path(),
            &serde_json::json!([
                serde_json::to_value(legacy_fizz.into_agent_record()).unwrap(),
                {
                    "pubkey": "fizz-pubkey",
                    "name": "Fizz",
                    "persona_id": "builtin:fizz",
                    "persona_source_version": old_version,
                    "updated_at": "before"
                }
            ]),
        );

        migrate_pollen_agent_name_in_file(&path, "after");

        let records = read_agents_json(dir.path());
        assert_eq!(
            records[0]["name_pool"],
            serde_json::json!(current_fizz.name_pool)
        );
        assert_eq!(records[0]["updated_at"], "after");
        assert_eq!(records[1]["persona_source_version"], new_version);
        assert_eq!(records[1]["updated_at"], "after");
    }
}
