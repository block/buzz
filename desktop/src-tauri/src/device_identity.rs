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
//! `device_label` is published in a world-readable kind:30177 event (see
//! [`crate::managed_agents::agent_events`]), so it starts **opaque** —
//! `device-<8 hex>`, derived from the id and saying nothing about the machine.
//! Host names routinely contain a real person's name, so the OS host name is
//! only ever *offered* as a suggestion ([`hostname_suggestion`]) and reaches the
//! relay solely when the owner applies it via [`set_device_label`]. Every label,
//! whichever boundary it arrives from — typed, loaded from disk, or received
//! from a peer — passes [`validate_device_label`], the same visible-text policy
//! that guards agent definition text. The opaque `device_id` alone is enough to
//! tell N devices apart.

use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::managed_agents::definition_validation::validate_device_label;
use crate::managed_agents::storage::{atomic_write_json_restricted, managed_agents_base_dir};

/// Stable identity of the computer this Buzz install runs on.
///
/// Distinguishes two devices signed into the same Buzz account. Minted
/// once at first run and never rotated; the label is user-editable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceIdentity {
    /// Opaque uuid v4 (simple hex, 32 chars). Never derived from hardware.
    pub device_id: String,
    /// Human label shown beside this device's agents on other devices. Starts
    /// opaque (`device-<8 hex>`); the owner may rename it, including to the OS
    /// host name, which is never applied without that explicit choice.
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

/// Normalize a user-supplied device label.
///
/// Trims, then enforces the shared visible-text policy
/// ([`validate_device_label`]) — which rejects not only `Cc` control
/// characters but the `Cf` format characters `char::is_control` misses, such as
/// zero-width spaces and bidi overrides. An over-long label is **refused, not
/// truncated**: silently publishing something other than what the owner typed
/// is worse than telling them it is too long.
fn sanitize_label(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    validate_device_label(trimmed)?;
    Ok(trimmed.to_string())
}

/// The opaque, id-derived label every device starts with.
///
/// Deliberately says nothing about the machine. See [`mint_identity`].
fn opaque_label(device_id: &str) -> String {
    format!("device-{}", device_id.chars().take(8).collect::<String>())
}

/// Validate a `device_id` against the shape [`mint_identity`] produces:
/// 32 lowercase hex digits (a uuid v4 in simple form).
///
/// Shared with the inbound relay path, which must not trust a peer's value.
pub(crate) fn validate_device_id(device_id: &str) -> Result<(), String> {
    if device_id.len() == 32
        && device_id
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_uppercase())
    {
        return Ok(());
    }
    Err("device id must be 32 lowercase hex characters".to_string())
}

/// Validate a whole identity, whichever boundary it arrived from.
fn validate_identity(identity: &DeviceIdentity) -> Result<(), String> {
    validate_device_id(&identity.device_id)?;
    validate_device_label(&identity.device_label)
}

/// The OS host name, offered to the owner as a suggested device name.
///
/// Returns `None` when the host name is unusable — empty, over-long, or
/// carrying characters the label policy refuses. This is only ever a
/// *suggestion*: nothing here reaches the relay until the owner applies it.
pub fn hostname_suggestion() -> Option<String> {
    let host = gethostname::gethostname();
    let host = host.to_string_lossy();
    sanitize_label(&host).ok()
}

/// Mint a brand-new identity with an **opaque** label.
///
/// The label is *not* seeded from the OS host name. Host names routinely carry
/// a real person's name ("marys-macbook"), and this label is published in a
/// world-readable kind:30177 event — so seeding from it would publish that name
/// before the owner had seen any warning or had a chance to edit it. The owner
/// opts into the host name explicitly, via [`hostname_suggestion`] surfaced in
/// the device-name settings card.
fn mint_identity() -> DeviceIdentity {
    let device_id = uuid::Uuid::new_v4().simple().to_string();
    let device_label = opaque_label(&device_id);
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
/// file is absent, unreadable, malformed, **or invalid**.
///
/// Deserializing proves only that the JSON has the right shape. The stored file
/// is on disk, editable by hand, and survives downgrades — so the contents are
/// revalidated here against the same policy [`set_label`] enforces. Without
/// that, a hand-edited `device.json` carrying a 5000-character label or a bidi
/// override would be published to the relay unchecked.
///
/// A file that fails either step is preserved as `device.json.corrupt` (best
/// effort) and replaced. Losing the identity only *relabels* a device — it
/// never touches agent data — so this path must never fail the caller.
fn load_or_create_at(path: &Path) -> Result<DeviceIdentity, String> {
    if path.exists() {
        match std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read device identity: {e}"))
            .and_then(|content| {
                serde_json::from_str::<DeviceIdentity>(&content)
                    .map_err(|e| format!("failed to parse device identity: {e}"))
            })
            .and_then(|identity| {
                validate_identity(&identity)
                    .map(|()| identity)
                    .map_err(|e| format!("stored device identity is invalid: {e}"))
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

/// Serializes every test that reads or writes [`CURRENT`].
///
/// `CURRENT` is process-global and Rust runs a binary's tests on parallel
/// threads, so without this a test that seeds a device would race one asserting
/// there is none. That is exactly the failure mode of the crate's known-flaky
/// `claude_spawn_uses_the_probed_cli_executable`, which mutates the global
/// `PATH`; do not reproduce it here.
#[cfg(test)]
pub(crate) static DEVICE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII seam letting a test exercise a code path that branches on a device
/// being cached. Holds [`DEVICE_TEST_LOCK`] and restores the previous value on
/// drop, so tests stay order-independent.
///
/// Test-only: `#[cfg(test)]` keeps it out of every release artifact, so the
/// cache stays writable only by [`ensure`] and [`set_label`] in production.
#[cfg(test)]
pub(crate) struct DeviceGuard {
    previous: Option<DeviceIdentity>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl DeviceGuard {
    /// Install `identity` as the cached device for the guard's lifetime.
    pub(crate) fn set(identity: Option<DeviceIdentity>) -> Self {
        let _lock = DEVICE_TEST_LOCK
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let previous = CURRENT
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = identity;
        Self { previous, _lock }
    }

    /// A deterministic device for assertions.
    pub(crate) fn sample() -> DeviceIdentity {
        DeviceIdentity {
            device_id: "0123456789abcdef0123456789abcdef".to_string(),
            device_label: "studio-mac".to_string(),
            created_at: "2026-01-01T00:00:00+00:00".to_string(),
        }
    }
}

#[cfg(test)]
impl Drop for DeviceGuard {
    fn drop(&mut self) {
        *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = self.previous.take();
    }
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

/// Return the OS host name as a *suggested* device name, or `None` when it is
/// unusable under the label policy.
///
/// Purely advisory: the settings card offers it, and nothing is published until
/// the owner applies it via [`set_device_label`]. See [`mint_identity`] for why
/// the host name is not the default.
#[tauri::command]
pub fn get_device_name_suggestion() -> Option<String> {
    hostname_suggestion()
}

/// Rename this device and republish the **active community's** local agents so
/// its members see the new label without waiting for the next app restart.
///
/// Scope, stated precisely because it is narrower than it looks: republishing
/// needs a retention scope, and a scope carries the owner keys for one
/// `(owner, relay)` pair — which are only resolved for the community currently
/// applied. Agents in the owner's *other* configured communities keep
/// publishing the old label until that community is next activated, at which
/// point `run_event_sync` reconciles them with the current label. So
/// propagation is eventual everywhere, immediate only here.
///
/// Republishing every scope up front would mean resolving owner keys for
/// communities that are not applied — a change to identity handling, not to
/// this command, and out of scope for Stage 0.
///
/// The republish is best-effort in the other direction too: a rename that
/// persists locally but cannot reach the retention store still succeeds, and
/// propagates on the next agent mutation or restart.
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

    /// Over-long labels are refused, never silently shortened: publishing
    /// something other than what the owner typed is the worse failure.
    #[test]
    fn sanitize_label_rejects_over_thirty_two_chars() {
        assert!(sanitize_label(&"a".repeat(33)).is_err());
        assert_eq!(sanitize_label(&"a".repeat(32)).unwrap(), "a".repeat(32));
    }

    /// `char::is_control` covers only category `Cc`. These are `Cf`, and a bidi
    /// override can visually reorder the text rendered around the label.
    #[test]
    fn sanitize_label_rejects_format_characters_is_control_would_miss() {
        assert!(!'\u{202E}'.is_control(), "precondition: RLO is not Cc");
        assert!(!'\u{200B}'.is_control(), "precondition: ZWSP is not Cc");
        assert!(sanitize_label("mfeth\u{202E}win").is_err(), "bidi override");
        assert!(
            sanitize_label("mfeth\u{200B}win").is_err(),
            "zero width space"
        );
        assert!(sanitize_label("mfeth\u{2066}win").is_err(), "bidi isolate");
    }

    #[test]
    fn opaque_label_is_derived_from_the_id() {
        assert_eq!(
            opaque_label("0123456789abcdef0123456789abcdef"),
            "device-01234567"
        );
    }

    /// A minted identity must not leak the OS host name — it is published
    /// world-readable before the owner has seen any warning.
    #[test]
    fn mint_identity_uses_an_opaque_label_not_the_hostname() {
        let identity = mint_identity();
        assert_eq!(identity.device_label, opaque_label(&identity.device_id));
        assert!(identity.device_label.starts_with("device-"));
        let host = gethostname::gethostname().to_string_lossy().to_string();
        if !host.trim().is_empty() {
            assert_ne!(identity.device_label, host.trim());
        }
        validate_identity(&identity).expect("a minted identity must be valid");
    }

    #[test]
    fn validate_device_id_accepts_only_lowercase_hex_of_length_32() {
        assert!(validate_device_id("0123456789abcdef0123456789abcdef").is_ok());
        assert!(validate_device_id("0123456789ABCDEF0123456789ABCDEF").is_err());
        assert!(validate_device_id("tooshort").is_err());
        assert!(validate_device_id(&"a".repeat(33)).is_err());
        assert!(validate_device_id("g123456789abcdef0123456789abcdef").is_err());
    }

    /// Deserializing proves shape, not validity. A hand-edited file must be
    /// quarantined and replaced rather than published.
    #[test]
    fn load_or_create_replaces_a_syntactically_valid_but_invalid_identity() {
        // Built through serde rather than written as a literal: an unescaped
        // U+202E in source trips rustc's own
        // `text_direction_codepoint_in_literal` lint — the same hazard this
        // validation exists to keep out of the relay.
        let bad_id = serde_json::json!({
            "deviceId": "nothex",
            "deviceLabel": "ok",
            "createdAt": "2026-01-01T00:00:00Z",
        })
        .to_string();
        let bad_label = serde_json::json!({
            "deviceId": "0123456789abcdef0123456789abcdef",
            "deviceLabel": "mfeth\u{202E}win",
            "createdAt": "2026-01-01T00:00:00Z",
        })
        .to_string();

        for bad in [bad_id.as_str(), bad_label.as_str()] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("device.json");
            std::fs::write(&path, bad).unwrap();

            let identity = load_or_create_at(&path)
                .expect("an invalid stored file must never fail the caller");
            validate_identity(&identity).expect("the replacement must be valid");
            assert!(path.with_extension("json.corrupt").exists(), "quarantined");
            // And the replacement sticks.
            assert_eq!(load_or_create_at(&path).unwrap(), identity);
        }
    }

    #[test]
    fn hostname_suggestion_is_valid_when_present() {
        if let Some(suggestion) = hostname_suggestion() {
            validate_device_label(&suggestion)
                .expect("a suggestion offered to the owner must already be valid");
        }
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
