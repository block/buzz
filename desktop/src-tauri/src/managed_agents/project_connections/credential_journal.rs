use std::{collections::BTreeSet, fs, io};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::{
    atomic_write_json_restricted, delete_secrets, is_lower_hex, reject_unsafe_owner_file,
    workspace_connection_dir, ProjectConnectionScope, ProjectConnectionStore,
};

const CREDENTIAL_JOURNAL_VERSION: u32 = 1;
const MAX_JOURNALED_GENERATIONS: usize = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CredentialJournal {
    version: u32,
    connection_id: String,
    generations: Vec<String>,
}

fn journal_path(
    app: &AppHandle,
    scope: &ProjectConnectionScope,
) -> Result<std::path::PathBuf, String> {
    Ok(workspace_connection_dir(app, scope)?.join("credential-journal.json"))
}

fn validate_journal(journal: &CredentialJournal) -> Result<(), String> {
    if journal.version != CREDENTIAL_JOURNAL_VERSION
        || !is_lower_hex(&journal.connection_id, 32)
        || journal.generations.is_empty()
        || journal.generations.len() > MAX_JOURNALED_GENERATIONS
        || journal
            .generations
            .iter()
            .any(|generation| !is_lower_hex(generation, 32))
        || journal.generations.iter().collect::<BTreeSet<_>>().len() != journal.generations.len()
    {
        return Err("Project connection credential recovery data is invalid.".to_string());
    }
    Ok(())
}

fn generations_to_delete(
    journal: &CredentialJournal,
    store: &ProjectConnectionStore,
) -> Vec<String> {
    let referenced = store
        .connections
        .iter()
        .find(|connection| connection.id == journal.connection_id)
        .filter(|connection| !connection.env_keys.is_empty())
        .map(|connection| connection.credential_generation.as_str());
    journal
        .generations
        .iter()
        .filter(|generation| Some(generation.as_str()) != referenced)
        .cloned()
        .collect()
}

pub(super) fn begin(
    app: &AppHandle,
    scope: &ProjectConnectionScope,
    connection_id: &str,
    generations: Vec<String>,
) -> Result<(), String> {
    let journal = CredentialJournal {
        version: CREDENTIAL_JOURNAL_VERSION,
        connection_id: connection_id.to_string(),
        generations,
    };
    validate_journal(&journal)?;
    let path = journal_path(app, scope)?;
    reject_unsafe_owner_file(&path)?;
    let bytes = serde_json::to_vec_pretty(&journal)
        .map_err(|error| format!("failed to prepare credential recovery data: {error}"))?;
    atomic_write_json_restricted(&path, &bytes)
}

pub(super) fn complete(app: &AppHandle, scope: &ProjectConnectionScope) -> Result<(), String> {
    let path = journal_path(app, scope)?;
    complete_path(&path)
}

fn complete_path(path: &std::path::Path) -> Result<(), String> {
    reject_unsafe_owner_file(path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "failed to clear Project connection credential recovery data: {error}"
        )),
    }
}

pub(super) fn reconcile(
    app: &AppHandle,
    scope: &ProjectConnectionScope,
    store: &ProjectConnectionStore,
) -> Result<(), String> {
    let path = journal_path(app, scope)?;
    reconcile_path(&path, store, |connection_id, generation| {
        delete_secrets(app, scope, connection_id, generation)
    })
}

fn reconcile_path(
    path: &std::path::Path,
    store: &ProjectConnectionStore,
    mut delete_generation: impl FnMut(&str, &str) -> Result<(), String>,
) -> Result<(), String> {
    reject_unsafe_owner_file(path)?;
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(format!(
                "failed to read Project connection credential recovery data: {error}"
            ));
        }
    };
    let journal: CredentialJournal = serde_json::from_slice(&bytes)
        .map_err(|_| "Project connection credential recovery data is invalid.".to_string())?;
    validate_journal(&journal)?;
    for generation in generations_to_delete(&journal, store) {
        delete_generation(&journal.connection_id, &generation)?;
    }
    complete_path(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::project_connections::{
        next_generation, ProjectConnectionHealth, StoredProjectConnection, CONNECTION_STORE_VERSION,
    };

    fn connection(generation: &str) -> StoredProjectConnection {
        StoredProjectConnection {
            id: "c".repeat(32),
            project_scope: ProjectConnectionScope {
                relay_url: "ws://127.0.0.1:3000".to_string(),
                operator_pubkey: "a".repeat(64),
                project_address: format!("30621:{}:portable-agents", "a".repeat(64)),
            },
            name: "Test".to_string(),
            provider: "Fixture".to_string(),
            capability_ids: Vec::new(),
            command: "/usr/bin/true".to_string(),
            args: Vec::new(),
            env_keys: vec!["TOKEN".to_string()],
            discovered_tools: Vec::new(),
            health: ProjectConnectionHealth::default(),
            executable_sha256: "d".repeat(64),
            generation: next_generation(),
            credential_generation: generation.to_string(),
            created_at: "2026-08-05T00:00:00Z".to_string(),
            updated_at: "2026-08-05T00:00:00Z".to_string(),
        }
    }

    fn store(connection: Option<StoredProjectConnection>) -> ProjectConnectionStore {
        ProjectConnectionStore {
            version: CONNECTION_STORE_VERSION,
            connections: connection.into_iter().collect(),
        }
    }

    #[test]
    fn restart_reconciliation_deletes_only_unreferenced_generations() {
        let old = "a".repeat(32);
        let new = "b".repeat(32);
        let cases = [
            (
                "create interrupted after credential write",
                CredentialJournal {
                    version: CREDENTIAL_JOURNAL_VERSION,
                    connection_id: "c".repeat(32),
                    generations: vec![new.clone()],
                },
                store(None),
                vec![new.clone()],
            ),
            (
                "update interrupted before metadata swap",
                CredentialJournal {
                    version: CREDENTIAL_JOURNAL_VERSION,
                    connection_id: "c".repeat(32),
                    generations: vec![old.clone(), new.clone()],
                },
                store(Some(connection(&old))),
                vec![new.clone()],
            ),
            (
                "update interrupted after metadata swap",
                CredentialJournal {
                    version: CREDENTIAL_JOURNAL_VERSION,
                    connection_id: "c".repeat(32),
                    generations: vec![old.clone(), new.clone()],
                },
                store(Some(connection(&new))),
                vec![old.clone()],
            ),
            (
                "delete interrupted after metadata removal",
                CredentialJournal {
                    version: CREDENTIAL_JOURNAL_VERSION,
                    connection_id: "c".repeat(32),
                    generations: vec![old.clone()],
                },
                store(None),
                vec![old.clone()],
            ),
        ];

        for (name, journal, store, expected) in cases {
            assert_eq!(generations_to_delete(&journal, &store), expected, "{name}");
        }
    }

    #[test]
    fn reconciliation_retries_cleanup_before_removing_the_journal() {
        let old = "a".repeat(32);
        let new = "b".repeat(32);
        let cases = [
            (
                CredentialJournal {
                    version: CREDENTIAL_JOURNAL_VERSION,
                    connection_id: "c".repeat(32),
                    generations: vec![new.clone()],
                },
                store(None),
                new.clone(),
            ),
            (
                CredentialJournal {
                    version: CREDENTIAL_JOURNAL_VERSION,
                    connection_id: "c".repeat(32),
                    generations: vec![old.clone(), new.clone()],
                },
                store(Some(connection(&old))),
                new.clone(),
            ),
            (
                CredentialJournal {
                    version: CREDENTIAL_JOURNAL_VERSION,
                    connection_id: "c".repeat(32),
                    generations: vec![old.clone(), new.clone()],
                },
                store(Some(connection(&new))),
                old.clone(),
            ),
            (
                CredentialJournal {
                    version: CREDENTIAL_JOURNAL_VERSION,
                    connection_id: "c".repeat(32),
                    generations: vec![old.clone()],
                },
                store(None),
                old.clone(),
            ),
        ];

        for (journal, store, orphan) in cases {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("credential-journal.json");
            let bytes = serde_json::to_vec_pretty(&journal).unwrap();
            atomic_write_json_restricted(&path, &bytes).unwrap();

            let error =
                reconcile_path(&path, &store, |_, _| Err("keyring unavailable".to_string()))
                    .unwrap_err();
            assert_eq!(error, "keyring unavailable");
            assert!(path.exists(), "failed cleanup must preserve recovery data");

            let mut deleted = Vec::new();
            reconcile_path(&path, &store, |connection_id, generation| {
                deleted.push((connection_id.to_string(), generation.to_string()));
                Ok(())
            })
            .unwrap();
            assert_eq!(deleted, [(journal.connection_id.clone(), orphan)]);
            assert!(!path.exists(), "successful retry must clear recovery data");
        }
    }
}
