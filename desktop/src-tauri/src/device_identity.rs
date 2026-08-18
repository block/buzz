//! Stable per-install device identity.
//!
//! Buzz agents carry device-local secrets: `apply_inbound_managed_agent` is a
//! deliberate no-op on no-match, so a persona synced to a second computer mints
//! a *fresh* keypair there. One name, N pubkeys, N computers — and nothing in
//! the UI says which computer an agent actually lives on. This module supplies
//! that missing noun.
//!
//! # What this is NOT
//!
//! It is not [`crate::managed_agents::runtime::current_instance_id`]. That
//! returns the Tauri *bundle identifier* — a build constant, identical on every
//! machine — and exists to keep a dev build from reaping a packaged build's
//! processes on the SAME computer. The two answer different questions and must
//! stay separate.
//!
//! # Storage
//!
//! `<app-data>/agents/device.json`, written `0o600` via
//! `atomic_write_json_restricted` — the same pattern the agent store and
//! `global-agent-config.json` use.
//!
//! # Privacy
//!
//! `device_label` is seeded from the OS host name and is published in a
//! world-readable kind:30177 event (see
//! [`crate::managed_agents::agent_events`]). Host names routinely contain a
//! real person's name, so the label is user-editable via [`set_device_label`]
//! and capped/sanitized by [`sanitize_label`]. The opaque `device_id` alone is
//! enough to tell N devices apart.

use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::managed_agents::storage::{atomic_write_json_restricted, managed_agents_base_dir};

/// Maximum length of a device label, in `char`s.
const MAX_DEVICE_LABEL_CHARS: usize = 32;

/// Stable identity of the computer this Buzz install runs on.
///
/// Distinguishes two devices signed into the same Buzz account. Minted
/// once at first run and never rotated; the label is user-editable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceIdentity {
    /// Opaque uuid v4 (simple hex, 32 chars). Never derived from hardware.
    pub device_id: String,
    /// Human label shown beside this device's agents on other devices.
    /// Seeded from the OS host name at first run.
    pub device_label: String,
    /// RFC 3339 first-run timestamp. Diagnostics only.
    pub created_at: String,
}

/// Process-wide cache of the resolved identity.
///
/// `None` until [`ensure`] runs, which only happens inside the Tauri `setup`
/// hook. Unit tests never boot the app, so every existing test observes `None`
/// and its published projections are byte-identical to before this module
/// existed.
static CURRENT: RwLock<Option<DeviceIdentity>> = RwLock::new(None);

/// Normalize a user-supplied or host-derived device label.
///
/// Trims, rejects an empty or control-character-bearing value, and caps the
/// result at [`MAX_DEVICE_LABEL_CHARS`] `char`s. Control characters are refused
/// rather than stripped because the label is published to a relay and rendered
/// in other clients' UI.
fn sanitize_label(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("device label must not be empty".to_string());
    }
    if trimmed.chars().any(char::is_control) {
        return Err("device label must not contain control characters".to_string());
    }
    let capped: String = trimmed.chars().take(MAX_DEVICE_LABEL_CHARS).collect();
    let capped = capped.trim_end();
    if capped.is_empty() {
        return Err("device label must not be empty".to_string());
    }
    Ok(capped.to_string())
}

/// Derive the first-run label from `seed` (the OS host name), falling back to
/// an id-derived placeholder when the seed sanitizes to nothing.
///
/// The fallback is deliberately opaque: a device with an unusable host name
/// still gets a stable, distinguishable label without inventing a plausible
/// but wrong name.
fn seed_label(seed: &str, device_id: &str) -> String {
    sanitize_label(seed)
        .unwrap_or_else(|_| format!("device-{}", device_id.chars().take(8).collect::<String>()))
}

/// Mint a brand-new identity, seeding the label from the OS host name.
fn mint_identity() -> DeviceIdentity {
    let device_id = uuid::Uuid::new_v4().simple().to_string();
    let host = gethostname::gethostname();
    let device_label = seed_label(&host.to_string_lossy(), &device_id);
    DeviceIdentity {
        device_id,
        device_label,
        created_at: chrono::Utc::now().to_rfc3339(),
    }
}

/// Persist `identity` to `path`, `0o600`, atomically.
fn write_identity_at(path: &Path, identity: &DeviceIdentity) -> Result<(), String> {
    let payload = serde_json::to_vec_pretty(identity)
        .map_err(|e| format!("failed to serialize device identity: {e}"))?;
    atomic_write_json_restricted(path, &payload)
}

/// Load the identity at `path`, minting and persisting a fresh one when the
/// file is absent, unreadable, or malformed.
///
/// A corrupt file is preserved as `device.json.corrupt` (best effort) and
/// replaced. Losing the identity only *relabels* a device — it never touches
/// agent data — so this path must never fail the caller.
fn load_or_create_at(path: &Path) -> Result<DeviceIdentity, String> {
    if path.exists() {
        match std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read device identity: {e}"))
            .and_then(|content| {
                serde_json::from_str::<DeviceIdentity>(&content)
                    .map_err(|e| format!("failed to parse device identity: {e}"))
            }) {
            Ok(identity) => return Ok(identity),
            Err(error) => {
                let corrupt = path.with_extension("json.corrupt");
                if let Err(rename_error) = std::fs::rename(path, &corrupt) {
                    tracing::warn!(
                        "device identity: could not preserve corrupt file: {rename_error}"
                    );
                }
                tracing::warn!("device identity: minting a fresh identity ({error})");
            }
        }
    }

    let identity = mint_identity();
    write_identity_at(path, &identity)?;
    Ok(identity)
}

/// Replace the label on the identity at `path`, minting one first if needed.
fn set_label_at(path: &Path, label: &str) -> Result<DeviceIdentity, String> {
    let device_label = sanitize_label(label)?;
    let mut identity = load_or_create_at(path)?;
    identity.device_label = device_label;
    write_identity_at(path, &identity)?;
    Ok(identity)
}

fn device_identity_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(managed_agents_base_dir(app)?.join("device.json"))
}

fn cache(identity: &DeviceIdentity) {
    let mut guard = CURRENT.write().unwrap_or_else(PoisonError::into_inner);
    *guard = Some(identity.clone());
}

/// Load or create this install's device identity and populate the process
/// cache read by [`current`].
///
/// Idempotent: safe to call more than once. Called once from the Tauri `setup`
/// hook, after boot migrations and before identity resolution.
pub fn ensure(app: &AppHandle) -> Result<DeviceIdentity, String> {
    let path = device_identity_path(app)?;
    let identity = load_or_create_at(&path)?;
    cache(&identity);
    Ok(identity)
}

/// The cached device identity, or `None` when [`ensure`] has not run.
///
/// `None` is a supported answer, not an error: unit tests and any code path
/// that runs before the Tauri `setup` hook simply publish no device stamp.
pub fn current() -> Option<DeviceIdentity> {
    CURRENT
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Rename this device, persisting and caching the result.
///
/// The new label reaches other devices on the next kind:30177 republish. The
/// [`set_device_label`] command triggers that republish immediately via the
/// managed-agent reconcile; calling this function directly leaves propagation
/// to the next agent mutation or app restart.
pub fn set_label(app: &AppHandle, label: &str) -> Result<DeviceIdentity, String> {
    let path = device_identity_path(app)?;
    let identity = set_label_at(&path, label)?;
    cache(&identity);
    Ok(identity)
}

/// Return this install's device identity, minting it on first call.
#[tauri::command]
pub fn get_device_identity(app: AppHandle) -> Result<DeviceIdentity, String> {
    ensure(&app)
}

/// Rename this device and republish every local agent's kind:30177 record so
/// other devices see the new label without waiting for the next app restart.
///
/// The republish is best-effort: a rename that persists locally but cannot
/// reach the retention store still succeeds, and propagates on the next agent
/// mutation or restart.
#[tauri::command]
pub fn set_device_label(app: AppHandle, label: String) -> Result<DeviceIdentity, String> {
    let identity = set_label(&app, &label)?;
    republish_agent_records(&app);
    Ok(identity)
}

/// Best-effort re-reconcile of every local managed-agent record so a changed
/// device label reaches the relay now. `retain_agent_record`'s content-equality
/// guard means records whose projection did not change stay untouched.
fn republish_agent_records(app: &AppHandle) {
    use tauri::Manager;

    let state = app.state::<crate::app_state::AppState>();
    match crate::managed_agents::retention::active_retention_scope(app, &state) {
        Ok(scope) => crate::managed_agents::reconcile::reconcile_agents_to_events(
            app,
            &scope.owner_keys,
            &scope.db_path,
        ),
        Err(error) => {
            tracing::warn!("device identity: label republish skipped: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_label_trims() {
        assert_eq!(sanitize_label("  mfeth-win \t").unwrap(), "mfeth-win");
    }

    #[test]
    fn sanitize_label_rejects_empty() {
        assert!(sanitize_label("").is_err());
        assert!(sanitize_label("   ").is_err());
    }

    #[test]
    fn sanitize_label_rejects_control_characters() {
        assert!(sanitize_label("mfeth\u{0}win").is_err());
        assert!(sanitize_label("mfeth\nwin").is_err());
    }

    #[test]
    fn sanitize_label_truncates_to_thirty_two_chars() {
        let long = "a".repeat(100);
        let sanitized = sanitize_label(&long).unwrap();
        assert_eq!(sanitized.chars().count(), 32);
        assert_eq!(sanitized, "a".repeat(32));
    }

    #[test]
    fn seed_label_falls_back_to_id_derived_label() {
        let device_id = "0123456789abcdef0123456789abcdef";
        assert_eq!(seed_label("  ", device_id), "device-01234567");
        assert_eq!(seed_label("\u{0}", device_id), "device-01234567");
    }

    #[test]
    fn seed_label_prefers_the_sanitized_seed() {
        let device_id = "0123456789abcdef0123456789abcdef";
        assert_eq!(seed_label(" mfeth-win ", device_id), "mfeth-win");
    }

    #[test]
    fn device_identity_round_trips_as_camel_case() {
        let identity = DeviceIdentity {
            device_id: "0123456789abcdef0123456789abcdef".to_string(),
            device_label: "mfeth-win".to_string(),
            created_at: "2026-08-18T00:00:00+00:00".to_string(),
        };
        let json = serde_json::to_string(&identity).unwrap();
        assert!(json.contains("\"deviceId\""), "{json}");
        assert!(json.contains("\"deviceLabel\""), "{json}");
        assert!(json.contains("\"createdAt\""), "{json}");
        assert!(!json.contains("device_id"), "{json}");

        let parsed: DeviceIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, identity);
    }

    #[test]
    fn load_or_create_mints_then_reuses() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.json");

        let first = load_or_create_at(&path).unwrap();
        assert_eq!(first.device_id.chars().count(), 32);
        assert!(!first.device_label.is_empty());
        assert!(path.exists());

        let second = load_or_create_at(&path).unwrap();
        assert_eq!(first, second, "identity must be stable across loads");
    }

    #[test]
    fn corrupt_file_is_preserved_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.json");
        std::fs::write(&path, "{ not json at all").unwrap();

        let identity = load_or_create_at(&path).expect("corrupt file must never fail the caller");
        assert_eq!(identity.device_id.chars().count(), 32);
        assert!(
            dir.path().join("device.json.corrupt").exists(),
            "the corrupt file must be preserved"
        );
        // The replacement is durable.
        assert_eq!(load_or_create_at(&path).unwrap(), identity);
    }

    #[test]
    fn set_label_persists_and_keeps_the_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.json");

        let minted = load_or_create_at(&path).unwrap();
        let renamed = set_label_at(&path, "  Studio Mac  ").unwrap();
        assert_eq!(renamed.device_label, "Studio Mac");
        assert_eq!(renamed.device_id, minted.device_id);
        assert_eq!(load_or_create_at(&path).unwrap(), renamed);
    }

    #[test]
    fn set_label_rejects_an_unusable_label() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.json");
        assert!(set_label_at(&path, "   ").is_err());
        assert!(set_label_at(&path, "bad\nlabel").is_err());
    }

    #[test]
    fn current_is_none_before_ensure_runs() {
        // Guards the zero-churn contract: every pre-existing unit test sees no
        // device stamp because the Tauri setup hook never ran, so no existing
        // published-projection assertion has to change.
        assert!(current().is_none());
    }
}
