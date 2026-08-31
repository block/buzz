//! Desktop self-registration. All keys remain in Rust/keychain, never IPC.
use buzz_core_pkg::host::{self, Report, Runtime};
use nostr::{Event, JsonUtil, Keys, Timestamp};
use serde::Serialize;
use tauri::State;

use crate::app_state::AppState;
use crate::managed_agents::AuthStatus;

#[derive(Serialize)]
pub struct LocalHost {
    pub host: String,
    pub report: Report,
}

pub(super) fn owner_keys(state: &AppState, expected_owner: &str) -> Result<Keys, String> {
    let keys = state.signing_keys()?;
    if keys.public_key().to_hex() != expected_owner {
        return Err("Identity changed during host registration".into());
    }
    Ok(keys)
}

pub(super) fn host_keys(owner: &Keys) -> Result<Keys, String> {
    let store = crate::secret_store::SecretStore::shared(crate::app_state::keyring_service());
    let secret = store.host_key(&format!("host:v1:{}", owner.public_key().to_hex()))?;
    Keys::parse(&secret).map_err(|e| format!("Invalid stored host key: {e}"))
}

fn parse(value: serde_json::Value) -> Result<Event, String> {
    Event::from_json(value.to_string()).map_err(|e| e.to_string())
}

fn value(event: Event) -> Result<serde_json::Value, String> {
    serde_json::to_value(event).map_err(|e| e.to_string())
}

fn catalog_string<T: Serialize>(value: T) -> Result<String, String> {
    serde_json::to_value(value)
        .map_err(|e| e.to_string())?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "Invalid runtime catalog status".into())
}

// AuthStatus is a tagged union, not a string enum. Never serialize its
// diagnostic: CLI errors may include credentials, paths, or configuration.
fn auth_status(value: &AuthStatus) -> &'static str {
    match value {
        AuthStatus::LoggedIn => "logged_in",
        AuthStatus::LoggedOut => "logged_out",
        AuthStatus::ConfigInvalid { .. } => "config_invalid",
        AuthStatus::NotApplicable => "not_applicable",
        AuthStatus::Unknown => "unknown",
    }
}

/// Real OS metadata and the existing runtime catalog, with a strict allowlist.
#[tauri::command]
pub async fn get_local_host(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    expected_owner: String,
) -> Result<LocalHost, String> {
    let owner = owner_keys(&state, &expected_owner)?;
    let relay = buzz_core_pkg::relay::normalize_relay_url(
        &crate::relay::relay_ws_url_with_override(&state),
    )
    .map_err(|_| "invalid host community")?;
    let catalog = super::discover_acp_providers(app.clone(), Some(false)).await?;
    let result = tokio::task::spawn_blocking(move || {
        let host = host_keys(&owner)?;
        let mut runtimes = catalog
            .into_iter()
            .map(|r| {
                Ok(Runtime {
                    id: r.id,
                    label: r.label,
                    availability: catalog_string(r.availability)?,
                    auth_status: auth_status(&r.auth_status).into(),
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        runtimes.sort_by(|a, b| a.id.cmp(&b.id));
        let provisioned: Vec<_> = crate::managed_agents::load_managed_agents(&app)?
            .iter()
            .filter(|r| {
                buzz_core_pkg::relay::normalize_relay_url(&r.relay_url)
                    .ok()
                    .as_deref()
                    == Some(relay.as_str())
                    && crate::managed_agents::execution_agent_owner(r, &owner.public_key().to_hex())
                        .is_ok()
            })
            .filter_map(|r| {
                crate::managed_agents::local_execution_config(&app, r)
                    .ok()
                    .map(|c| host::ProvisionedAgent {
                        agent: r.pubkey.clone(),
                        runtime: c.runtime,
                        revision: c.revision,
                    })
            })
            .filter(|c| {
                runtimes.iter().any(|r| {
                    r.id == c.runtime
                        && r.availability == "available"
                        && matches!(r.auth_status.as_str(), "logged_in" | "not_applicable")
                })
            })
            .collect();
        let report = Report {
            v: 3,
            name: gethostname::gethostname().to_string_lossy().into_owned(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            launcher_version: env!("CARGO_PKG_VERSION").into(),
            accepts_start: !provisioned.is_empty()
                && super::host_start::receiver_healthy(&app, &owner.public_key().to_hex(), &relay),
            runtimes,
            provisioned,
        };
        report.validate()?;
        Ok(LocalHost {
            host: host.public_key().to_hex(),
            report,
        })
    })
    .await
    .map_err(|e| e.to_string())?;
    owner_keys(&state, &expected_owner)?;
    result
}

/// Owner-signed binding for a persisted host key; caller first queries the relay.
#[tauri::command]
pub async fn create_host_registration(
    state: State<'_, AppState>,
    expected_owner: String,
) -> Result<serde_json::Value, String> {
    let owner = owner_keys(&state, &expected_owner)?;
    let result = tokio::task::spawn_blocking(move || {
        let host = host_keys(&owner)?;
        value(host::registration(
            &owner,
            host.public_key(),
            Timestamp::now().as_secs(),
        )?)
    })
    .await
    .map_err(|e| e.to_string())?;
    owner_keys(&state, &expected_owner)?;
    result
}

/// Produce a report from native discovery, not caller-provided machine metadata.
#[tauri::command]
pub async fn create_host_report(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    expected_owner: String,
    registration: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let local = get_local_host(app, state.clone(), expected_owner.clone()).await?;
    let owner = owner_keys(&state, &expected_owner)?;
    let result = tokio::task::spawn_blocking(move || {
        let reg = parse(registration)?;
        if host::validate(&reg)?.owner != owner.public_key() {
            return Err("Foreign host registration".into());
        }
        value(host::profile(
            &host_keys(&owner)?,
            &reg,
            &local.report,
            Timestamp::now().as_secs(),
        )?)
    })
    .await
    .map_err(|e| e.to_string())?;
    owner_keys(&state, &expected_owner)?;
    result
}

/// Verify a registration before displaying it or using it to suppress a write.
#[tauri::command]
pub async fn inspect_host_registration(
    state: State<'_, AppState>,
    expected_owner: String,
    registration: serde_json::Value,
) -> Result<String, String> {
    let owner = owner_keys(&state, &expected_owner)?;
    let reg = parse(registration)?;
    let env = host::validate(&reg)?;
    if env.label != "registration" || env.owner != owner.public_key() {
        return Err("Foreign host registration".into());
    }
    let text = nostr::nips::nip44::decrypt(owner.secret_key(), &owner.public_key(), &reg.content)
        .map_err(|e| e.to_string())?;
    let body: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if body != serde_json::json!({"v": 1}) {
        return Err("Unknown host registration version".into());
    }
    Ok(env.host.to_hex())
}

/// Verify signatures and owner/host binding before decrypting an incoming report.
#[tauri::command]
pub async fn decode_host_report(
    state: State<'_, AppState>,
    expected_owner: String,
    registration: serde_json::Value,
    report: serde_json::Value,
) -> Result<Report, String> {
    let owner = owner_keys(&state, &expected_owner)?;
    host::decrypt_report(&owner, &parse(registration)?, &parse(report)?)
}

/// Sign a liveness pulse without exporting keys or tying it to human idle state.
#[tauri::command]
pub async fn create_host_presence(
    state: State<'_, AppState>,
    expected_owner: String,
    registration: serde_json::Value,
    run: String,
    seq: u64,
    status: String,
) -> Result<serde_json::Value, String> {
    let owner = owner_keys(&state, &expected_owner)?;
    let result = tokio::task::spawn_blocking(move || {
        let host = host_keys(&owner)?;
        let reg = parse(registration)?;
        let binding = host::validate(&reg)?;
        if binding.label != "registration"
            || binding.owner != owner.public_key()
            || binding.host != host.public_key()
        {
            return Err("Foreign host registration".into());
        }
        value(buzz_core_pkg::run_presence::pulse(
            &host,
            &run,
            seq,
            &status,
            None,
            Some(&reg.id.to_hex()),
            Timestamp::now().as_secs(),
        )?)
    })
    .await
    .map_err(|e| e.to_string())?;
    owner_keys(&state, &expected_owner)?;
    result
}

/// Public location for local launches. Never export the private OS name by default.
pub(crate) fn local_launch_location(
    owner_hex: &str,
) -> Result<buzz_core_pkg::run_presence::Location, String> {
    let store = crate::secret_store::SecretStore::shared(crate::app_state::keyring_service());
    let secret = store.host_key(&format!("host:v1:{owner_hex}"))?;
    let host = Keys::parse(&secret)
        .map_err(|e| e.to_string())?
        .public_key()
        .to_hex();
    Ok(buzz_core_pkg::run_presence::Location {
        label: format!("Desktop {}", &host[..8]),
        host,
    })
}

/// Detailed presence snapshot. Missing protocol support and query errors stay unknown.
#[tauri::command]
pub async fn get_presence_runs(
    state: State<'_, AppState>,
    expected_owner: String,
    relay_url: String,
    pubkeys: Vec<String>,
) -> Result<std::collections::HashMap<String, Vec<buzz_core_pkg::run_presence::RunPresence>>, String>
{
    let owner = owner_keys(&state, &expected_owner)?;
    if pubkeys.len() > 256 {
        return Err("Too many presence subjects".into());
    }
    if pubkeys.is_empty() {
        return Ok(Default::default());
    }
    let relay_self = super::identity_archive::fetch_relay_self_at(&state, &relay_url)
        .await?
        .ok_or("Relay did not advertise a presence snapshot signer")?;
    let events = crate::relay::query_relay_at_with_keys(
        &state,
        &crate::relay::relay_http_base_url(&relay_url),
        &[serde_json::json!({ "kinds": [40902], "authors": pubkeys })],
        &owner,
        None,
    )
    .await?;
    owner_keys(&state, &expected_owner)?;
    decode_presence_snapshot(events, &pubkeys, &relay_self, Timestamp::now().as_secs())
}

fn decode_presence_snapshot(
    events: Vec<Event>,
    pubkeys: &[String],
    relay_self: &str,
    now: u64,
) -> Result<std::collections::HashMap<String, Vec<buzz_core_pkg::run_presence::RunPresence>>, String>
{
    let mut result = std::collections::HashMap::new();
    for event in events {
        buzz_core_pkg::verify_event(&event).map_err(|e| e.to_string())?;
        if event.kind.as_u16() != 20001 || event.pubkey.to_hex() != relay_self {
            return Err("Invalid presence snapshot authority".into());
        }
        let subject = event
            .tags
            .iter()
            .find_map(|t| {
                let t = t.as_slice();
                (t.len() == 2 && t[0] == "p").then(|| t[1].clone())
            })
            .ok_or("Missing presence subject")?;
        if !pubkeys.contains(&subject) {
            return Err("Unexpected presence subject".into());
        }
        let payload = event
            .tags
            .iter()
            .find_map(|t| {
                let t = t.as_slice();
                (t.len() == 3 && t[0] == "presence_runs" && t[1] == "1").then(|| t[2].clone())
            })
            .ok_or("Relay does not support live host locations")?;
        let runs: Vec<buzz_core_pkg::run_presence::RunPresence> =
            serde_json::from_str(&payload).map_err(|e| e.to_string())?;
        validate_snapshot_runs(&runs, now)?;
        if result.insert(subject, runs).is_some() {
            return Err("Duplicate presence snapshot subject".into());
        }
    }
    if result.len()
        != pubkeys
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
    {
        return Err("Incomplete presence snapshot".into());
    }
    Ok(result)
}

fn validate_snapshot_runs(
    runs: &[buzz_core_pkg::run_presence::RunPresence],
    now: u64,
) -> Result<(), String> {
    let mut ids = std::collections::HashSet::new();
    if runs.len() > 32 {
        return Err("Invalid presence snapshot".into());
    }
    for run in runs {
        if run.run.len() != 32
            || !run
                .run
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            || !ids.insert(&run.run)
            || run.seq > 9_007_199_254_740_991
            || !matches!(run.status.as_str(), "online" | "away")
            || run.expires_at > now.saturating_add(buzz_core_pkg::run_presence::LEASE_SECONDS)
        {
            return Err("Invalid presence run".into());
        }
        if let Some(location) = &run.location {
            location.validate()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::managed_agents::AcpAvailabilityStatus;

    #[test]
    fn snapshots_require_the_selected_relays_signature_and_complete_subjects() {
        let relay = Keys::generate();
        let impostor = Keys::generate();
        let subject = Keys::generate().public_key().to_hex();
        let snapshot = |signer: &Keys| {
            nostr::EventBuilder::new(nostr::Kind::Custom(20001), "offline")
                .tags([
                    nostr::Tag::parse(["p", subject.as_str()]).unwrap(),
                    nostr::Tag::parse(["presence_runs", "1", "[]"]).unwrap(),
                ])
                .sign_with_keys(signer)
                .unwrap()
        };
        let subjects = vec![subject.clone()];
        let signer = relay.public_key().to_hex();
        let good = snapshot(&relay);
        assert!(decode_presence_snapshot(vec![good.clone()], &subjects, &signer, 100).is_ok());
        assert!(
            decode_presence_snapshot(vec![snapshot(&impostor)], &subjects, &signer, 100).is_err()
        );
        assert!(decode_presence_snapshot(vec![], &subjects, &signer, 100).is_err());
        assert!(
            decode_presence_snapshot(vec![good.clone(), good], &subjects, &signer, 100).is_err()
        );
    }

    #[test]
    fn every_auth_status_projects_without_diagnostics() {
        let sensitive = "/private/credentials TOKEN=do-not-publish";
        for (status, expected) in [
            (AuthStatus::LoggedIn, "logged_in"),
            (AuthStatus::LoggedOut, "logged_out"),
            (
                AuthStatus::ConfigInvalid {
                    diagnostic: sensitive.into(),
                },
                "config_invalid",
            ),
            (AuthStatus::NotApplicable, "not_applicable"),
            (AuthStatus::Unknown, "unknown"),
        ] {
            // Pin the projection to the real native tagged-union contract.
            assert_eq!(serde_json::to_value(&status).unwrap()["status"], expected);
            let runtime = Runtime {
                id: "test-runtime".into(),
                label: "Test runtime".into(),
                availability: catalog_string(AcpAvailabilityStatus::Available).unwrap(),
                auth_status: auth_status(&status).into(),
            };
            let serialized = serde_json::to_value(runtime).unwrap();
            assert_eq!(serialized["auth_status"], expected);
            assert_eq!(serialized.as_object().unwrap().len(), 4);
            assert!(!serialized.to_string().contains(sensitive));
            assert!(!serialized.to_string().contains("diagnostic"));
        }
    }

    #[test]
    fn every_availability_is_a_string_enum() {
        for (status, expected) in [
            (AcpAvailabilityStatus::Available, "available"),
            (AcpAvailabilityStatus::AdapterMissing, "adapter_missing"),
            (AcpAvailabilityStatus::AdapterOutdated, "adapter_outdated"),
            (AcpAvailabilityStatus::CliMissing, "cli_missing"),
            (AcpAvailabilityStatus::NotInstalled, "not_installed"),
        ] {
            assert_eq!(catalog_string(status).unwrap(), expected);
        }
    }
}

/// Read one exact owner-private inventory page on the selected relay. The HTTP
/// bridge preserves the timestamp + event-ID cursor (ordinary WS REQ does not).
#[tauri::command]
pub async fn get_host_history_page(
    state: State<'_, AppState>,
    expected_owner: String,
    relay_url: String,
    filter: serde_json::Value,
) -> Result<Vec<Event>, String> {
    let owner = owner_keys(&state, &expected_owner)?;
    if filter.get("kinds") != Some(&serde_json::json!([50000]))
        || filter.get("#p") != Some(&serde_json::json!([expected_owner]))
        || filter.get("#L") != Some(&serde_json::json!(["buzz.host.v1"]))
        || filter.get("limit") != Some(&serde_json::json!(1000))
    {
        return Err("Invalid host history scope".into());
    }
    let events = crate::relay::query_private_host_at_with_keys(
        &state,
        &crate::relay::relay_http_base_url(&relay_url),
        &[filter],
        &owner,
        None,
    )
    .await
    .map_err(|_| "Host history query failed".to_string())?;
    owner_keys(&state, &expected_owner)?;
    Ok(events)
}
