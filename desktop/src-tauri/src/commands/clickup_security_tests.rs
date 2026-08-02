use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    },
};

use serde_json::json;

use super::{
    enforce_task_scope, ensure_credential_key_unchanged, persist_token_transactionally,
    ClickUpTask, CredentialBackend,
};

#[derive(Default)]
struct FakeCredentialBackend {
    values: Mutex<HashMap<String, String>>,
    store_failures: AtomicUsize,
    verify_errors: AtomicUsize,
    verify_mismatches: AtomicUsize,
}

impl FakeCredentialBackend {
    fn take(counter: &AtomicUsize) -> bool {
        counter
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_sub(1)
            })
            .is_ok()
    }
}

impl CredentialBackend for FakeCredentialBackend {
    fn load_value(&self, key: &str) -> Result<Option<String>, ()> {
        Ok(self.values.lock().unwrap().get(key).cloned())
    }

    fn store_value(&self, key: &str, value: &str) -> Result<(), ()> {
        if Self::take(&self.store_failures) {
            return Err(());
        }
        self.values
            .lock()
            .unwrap()
            .insert(key.to_owned(), value.to_owned());
        Ok(())
    }

    fn verify_value(&self, key: &str, expected: &str) -> Result<bool, ()> {
        if Self::take(&self.verify_errors) {
            return Err(());
        }
        if Self::take(&self.verify_mismatches) {
            return Ok(false);
        }
        Ok(self.values.lock().unwrap().get(key).map(String::as_str) == Some(expected))
    }

    fn delete_value(&self, key: &str) -> Result<(), ()> {
        self.values.lock().unwrap().remove(key);
        Ok(())
    }
}

#[test]
fn token_verification_mismatch_restores_previous_credential() {
    let backend = FakeCredentialBackend::default();
    backend
        .values
        .lock()
        .unwrap()
        .insert("identity".to_owned(), "pk_previous-token".to_owned());
    backend.verify_mismatches.store(1, Ordering::SeqCst);

    assert!(persist_token_transactionally(&backend, "identity", "pk_new-token").is_err());
    assert_eq!(
        backend.values.lock().unwrap().get("identity").cloned(),
        Some("pk_previous-token".to_owned())
    );
}

#[test]
fn token_verification_error_removes_new_credential_when_none_existed() {
    let backend = FakeCredentialBackend::default();
    backend.verify_errors.store(1, Ordering::SeqCst);

    assert!(persist_token_transactionally(&backend, "identity", "pk_new-token").is_err());
    assert!(!backend.values.lock().unwrap().contains_key("identity"));
}

#[test]
fn token_store_failure_preserves_previous_credential() {
    let backend = FakeCredentialBackend::default();
    backend
        .values
        .lock()
        .unwrap()
        .insert("identity".to_owned(), "pk_previous-token".to_owned());
    backend.store_failures.store(1, Ordering::SeqCst);

    assert!(persist_token_transactionally(&backend, "identity", "pk_new-token").is_err());
    assert_eq!(
        backend.values.lock().unwrap().get("identity").cloned(),
        Some("pk_previous-token".to_owned())
    );
}

#[test]
fn task_scope_requires_workspace_assignment_and_open_state() {
    let mut task: ClickUpTask = serde_json::from_value(json!({
        "id": "task-1",
        "name": "Scoped task",
        "team_id": "workspace-1",
        "status": { "status": "open", "type": "custom" },
        "assignees": [{ "id": 42, "username": "Mikes" }]
    }))
    .unwrap();

    assert!(enforce_task_scope(&task, "workspace-1", 42).is_ok());
    assert!(enforce_task_scope(&task, "workspace-2", 42).is_err());
    assert!(enforce_task_scope(&task, "workspace-1", 7).is_err());
    task.status.kind = Some("closed".to_owned());
    assert!(enforce_task_scope(&task, "workspace-1", 42).is_err());
}

#[test]
fn identity_change_is_rejected_before_token_persistence() {
    assert!(ensure_credential_key_unchanged("clickup:identity-a", "clickup:identity-a").is_ok());
    let error =
        ensure_credential_key_unchanged("clickup:identity-a", "clickup:identity-b").unwrap_err();
    assert!(error.contains("identity_changed"));
}
