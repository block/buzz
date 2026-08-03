use super::{ProjectConnectionStore, StoredProjectConnection};

pub(super) struct UpdateTransaction<'a> {
    pub(super) index: usize,
    pub(super) previous: &'a StoredProjectConnection,
    pub(super) updated: &'a StoredProjectConnection,
    pub(super) secrets_changed: bool,
}

pub(super) fn commit_update<WriteNew, SaveMetadata, DeleteGeneration>(
    store: &mut ProjectConnectionStore,
    transaction: UpdateTransaction<'_>,
    mut write_new: WriteNew,
    mut save_metadata: SaveMetadata,
    mut delete_generation: DeleteGeneration,
) -> Result<(), String>
where
    WriteNew: FnMut() -> Result<(), String>,
    SaveMetadata: FnMut(&ProjectConnectionStore) -> Result<(), String>,
    DeleteGeneration: FnMut(&str) -> Result<(), String>,
{
    if transaction.secrets_changed {
        write_new()?;
    }
    store.connections[transaction.index] = transaction.updated.clone();
    if let Err(error) = save_metadata(store) {
        store.connections[transaction.index] = transaction.previous.clone();
        if transaction.secrets_changed {
            if let Err(cleanup_error) =
                delete_generation(&transaction.updated.credential_generation)
            {
                return Err(format!(
                    "{error} Buzz also could not remove the unreferenced credentials: {cleanup_error}"
                ));
            }
        }
        return Err(error);
    }
    if transaction.secrets_changed && !transaction.previous.env_keys.is_empty() {
        delete_generation(&transaction.previous.credential_generation).map_err(|error| {
            format!(
                "The connection was updated, but Buzz could not remove its superseded credentials: {error}"
            )
        })?;
    }
    Ok(())
}

pub(super) fn commit_delete<SaveMetadata, DeleteGeneration>(
    store: &mut ProjectConnectionStore,
    index: usize,
    mut save_metadata: SaveMetadata,
    mut delete_generation: DeleteGeneration,
) -> Result<(), String>
where
    SaveMetadata: FnMut(&ProjectConnectionStore) -> Result<(), String>,
    DeleteGeneration: FnMut(&str) -> Result<(), String>,
{
    let removed = store.connections.remove(index);
    save_metadata(store)?;
    if removed.env_keys.is_empty() {
        return Ok(());
    }
    if let Err(error) = delete_generation(&removed.credential_generation) {
        store.connections.insert(index, removed);
        return match save_metadata(store) {
            Ok(()) => Err(format!(
                "Buzz could not remove the saved credentials, so the connection was restored: {error}"
            )),
            Err(restore_error) => Err(format!(
                "Buzz could not remove the saved credentials, and could not restore the connection metadata: {error}; {restore_error}"
            )),
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::project_connections::{
        next_generation, ProjectConnectionHealth, ProjectConnectionScope,
    };

    fn connection(env_keys: &[&str]) -> StoredProjectConnection {
        StoredProjectConnection {
            id: "c".repeat(32),
            project_scope: ProjectConnectionScope {
                relay_url: "ws://127.0.0.1:3000".to_string(),
                operator_pubkey: "a".repeat(64),
                repo_address: format!("30617:{}:portable-agents", "a".repeat(64)),
            },
            name: "Test".to_string(),
            provider: "Fixture".to_string(),
            capability_ids: Vec::new(),
            command: "/usr/bin/true".to_string(),
            args: Vec::new(),
            env_keys: env_keys.iter().map(|key| (*key).to_string()).collect(),
            discovered_tools: Vec::new(),
            health: ProjectConnectionHealth::default(),
            executable_sha256: "d".repeat(64),
            generation: next_generation(),
            credential_generation: next_generation(),
            created_at: "2026-08-03T00:00:00Z".to_string(),
            updated_at: "2026-08-03T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn update_metadata_failure_removes_only_the_new_secret_generation() {
        let previous = connection(&["TOKEN"]);
        let mut updated = previous.clone();
        updated.credential_generation = next_generation();
        let mut store = ProjectConnectionStore {
            version: 1,
            connections: vec![previous.clone()],
        };
        let mut deleted = Vec::new();

        let error = commit_update(
            &mut store,
            UpdateTransaction {
                index: 0,
                previous: &previous,
                updated: &updated,
                secrets_changed: true,
            },
            || Ok(()),
            |_| Err("metadata failed".to_string()),
            |generation| {
                deleted.push(generation.to_string());
                Ok(())
            },
        )
        .unwrap_err();

        assert_eq!(error, "metadata failed");
        assert_eq!(store.connections[0], previous);
        assert_eq!(deleted, [updated.credential_generation]);
    }

    #[test]
    fn update_reports_failed_cleanup_without_repointing_metadata() {
        let previous = connection(&["TOKEN"]);
        let mut updated = previous.clone();
        updated.credential_generation = next_generation();
        let mut store = ProjectConnectionStore {
            version: 1,
            connections: vec![previous.clone()],
        };

        let error = commit_update(
            &mut store,
            UpdateTransaction {
                index: 0,
                previous: &previous,
                updated: &updated,
                secrets_changed: true,
            },
            || Ok(()),
            |_| Err("metadata failed".to_string()),
            |_| Err("cleanup failed".to_string()),
        )
        .unwrap_err();

        assert!(error.contains("cleanup failed"));
        assert_eq!(store.connections[0], previous);
    }

    #[test]
    fn failed_secret_delete_restores_connection_metadata() {
        let previous = connection(&["TOKEN"]);
        let mut store = ProjectConnectionStore {
            version: 1,
            connections: vec![previous.clone()],
        };
        let mut saves = 0;

        let error = commit_delete(
            &mut store,
            0,
            |_| {
                saves += 1;
                Ok(())
            },
            |_| Err("keyring failed".to_string()),
        )
        .unwrap_err();

        assert!(error.contains("connection was restored"));
        assert_eq!(saves, 2);
        assert_eq!(store.connections, [previous]);
    }

    #[test]
    fn secretless_delete_never_touches_the_credential_backend() {
        let mut store = ProjectConnectionStore {
            version: 1,
            connections: vec![connection(&[])],
        };
        let mut credential_calls = 0;

        commit_delete(
            &mut store,
            0,
            |_| Ok(()),
            |_| {
                credential_calls += 1;
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(credential_calls, 0);
        assert!(store.connections.is_empty());
    }
}
