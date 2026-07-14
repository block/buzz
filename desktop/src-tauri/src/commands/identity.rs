use nostr::{
    nips::nip44, Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, Tag, Timestamp, ToBech32,
};
use tauri::Manager;
use tauri::State;

use crate::{
    app_state::AppState,
    models::IdentityInfo,
    nostr_bind,
    relay::{self, relay_api_base_url_with_override, relay_ws_url_with_override},
};

/// Encode `pubkey` as npub bech32 and truncate it for display: first 10 chars
/// + "…" + last 4 chars. Returns the full bech32 when it is 16 chars or fewer.
fn truncated_display_name(pubkey: &PublicKey) -> Result<String, String> {
    let bech32 = pubkey
        .to_bech32()
        .map_err(|error| format!("bech32 encode failed: {error}"))?;
    Ok(if bech32.len() > 16 {
        format!("{}…{}", &bech32[..10], &bech32[bech32.len() - 4..])
    } else {
        bech32
    })
}

#[tauri::command]
pub fn get_identity(state: State<'_, AppState>) -> Result<IdentityInfo, String> {
    let keys = state.keys.lock().map_err(|error| error.to_string())?;
    let pubkey = keys.public_key();
    let pubkey_hex = pubkey.to_hex();
    let display_name = truncated_display_name(&pubkey)?;
    let lost = state
        .identity_lost
        .load(std::sync::atomic::Ordering::Acquire);
    let locked = state
        .keyring_locked
        .load(std::sync::atomic::Ordering::Acquire);

    Ok(IdentityInfo {
        pubkey: pubkey_hex,
        display_name,
        lost,
        locked,
    })
}

#[tauri::command]
pub fn get_default_relay_url() -> String {
    relay::relay_ws_url()
}

#[tauri::command]
pub fn is_shared_identity() -> bool {
    std::env::var("BUZZ_SHARE_IDENTITY")
        .map(|v| v == "1")
        .unwrap_or(false)
        && std::env::var("BUZZ_PRIVATE_KEY")
            .ok()
            .and_then(|k| Keys::parse(k.trim()).ok())
            .is_some()
}

#[tauri::command]
pub fn get_relay_ws_url(state: State<'_, AppState>) -> String {
    relay_ws_url_with_override(&state)
}

#[tauri::command]
pub fn get_relay_http_url(state: State<'_, AppState>) -> String {
    relay_api_base_url_with_override(&state)
}

#[tauri::command]
pub fn get_media_proxy_port(state: State<'_, AppState>) -> u16 {
    state
        .media_proxy_port
        .load(std::sync::atomic::Ordering::Relaxed)
}

#[tauri::command]
pub async fn sign_event(
    kind: u16,
    content: String,
    created_at: Option<u64>,
    tags: Vec<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let keys = state.signing_keys()?;

    tauri::async_runtime::spawn_blocking(move || {
        let nostr_tags = tags
            .into_iter()
            .map(|tag| Tag::parse(tag).map_err(|error| format!("invalid tag: {error}")))
            .collect::<Result<Vec<_>, _>>()?;

        let mut builder = EventBuilder::new(Kind::Custom(kind), content).tags(nostr_tags);
        if let Some(created_at) = created_at {
            builder = builder.custom_created_at(Timestamp::from(created_at));
        }

        let event = builder
            .sign_with_keys(&keys)
            .map_err(|error| format!("sign failed: {error}"))?;

        Ok(event.as_json())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub fn decrypt_observer_event(
    event_json: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let keys = state.signing_keys()?;
    let event = Event::from_json(event_json).map_err(|error| format!("invalid event: {error}"))?;

    // Defense-in-depth: verify event ID and signature before decrypting.
    if !event.verify_id() {
        return Err("observer event has invalid ID".into());
    }
    if !event.verify_signature() {
        return Err("observer event has invalid signature".into());
    }

    buzz_core_pkg::observer::decrypt_observer_payload(&keys, &event)
        .map_err(|error| format!("decrypt observer event failed: {error}"))
}

#[tauri::command]
pub fn build_observer_control_event(
    agent_pubkey: String,
    payload: serde_json::Value,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let keys = state.signing_keys()?;
    let agent_pubkey = PublicKey::from_hex(agent_pubkey.trim())
        .map_err(|error| format!("invalid agent pubkey: {error}"))?;
    let agent_pubkey_hex = agent_pubkey.to_hex();
    let encrypted =
        buzz_core_pkg::observer::encrypt_observer_payload(&keys, &agent_pubkey, &payload)
            .map_err(|error| format!("encrypt observer control failed: {error}"))?;
    let builder = buzz_sdk_pkg::build_agent_observer_frame(
        &agent_pubkey_hex,
        &agent_pubkey_hex,
        buzz_core_pkg::observer::OBSERVER_FRAME_CONTROL,
        &encrypted,
    )
    .map_err(|error| format!("build observer control failed: {error}"))?;
    let event = builder
        .sign_with_keys(&keys)
        .map_err(|error| format!("sign observer control failed: {error}"))?;
    Ok(event.as_json())
}

#[tauri::command]
pub fn get_nsec(state: State<'_, AppState>) -> Result<String, String> {
    let keys = state.signing_keys()?;
    keys.secret_key()
        .to_bech32()
        .map_err(|error| format!("encode nsec: {error}"))
}

#[tauri::command]
pub async fn import_identity(
    nsec: String,
    app_handle: tauri::AppHandle,
) -> Result<IdentityInfo, String> {
    tokio::task::spawn_blocking(move || {
        let trimmed = nsec.trim();
        let keys = Keys::parse(trimmed).map_err(|e| format!("Invalid private key: {e}"))?;

        // Serialize against persist_current_identity: hold this guard for the
        // full function body so a concurrent stale persist can't overwrite
        // this import.
        let state = app_handle.state::<AppState>();
        let _mutation_guard = state.identity_mutation.lock().map_err(|e| e.to_string())?;

        let data_dir = app_handle
            .path()
            .app_data_dir()
            .map_err(|e| format!("app data dir: {e}"))?;
        std::fs::create_dir_all(&data_dir).map_err(|e| format!("create app data dir: {e}"))?;
        let key_path = data_dir.join("identity.key");

        // Persist into the OS keyring first (store → read-back verify → marker →
        // delete file). Falls back to the 0o600 file when the keyring is
        // unavailable; returns Err only when both backends fail.
        let store = crate::secret_store::SecretStore::shared(crate::app_state::keyring_service());
        crate::app_state::persist_imported_identity(store, &keys, &key_path, &data_dir)?;

        // Update in-memory keys BEFORE clearing recovery flags. The Release
        // stores below pair with Acquire loads in get_identity: a reader
        // observing false is guaranteed to see the updated keys.
        let pubkey = keys.public_key();
        *state.keys.lock().map_err(|e| e.to_string())? = keys;

        // Clear both recovery flags — an import is valid in either lost or
        // keyring-locked state and resolves both. In the locked case the
        // keyring is unreachable, so persist_imported_identity already fell
        // back to identity.key; on the next Unreachable boot the file is
        // loaded directly and when the keyring returns the adoption path
        // picks it up.
        state
            .identity_lost
            .store(false, std::sync::atomic::Ordering::Release);
        state
            .keyring_locked
            .store(false, std::sync::atomic::Ordering::Release);

        let pubkey_hex = pubkey.to_hex();
        let display_name = truncated_display_name(&pubkey)?;

        eprintln!("buzz-desktop: imported identity pubkey {}", pubkey_hex);

        Ok(IdentityInfo {
            pubkey: pubkey_hex,
            display_name,
            lost: false,
            locked: false,
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

/// Make the current ephemeral identity durable by persisting it to the OS
/// keyring (or falling back to identity.key). This is called when the user
/// chooses to start a new identity instead of re-importing their previous one
/// — it converts the transient lost-state key into a permanent identity.
///
/// **LOST-ONLY**: returns `Err` when `identity_lost` is false, and deliberately
/// does NOT accept `keyring_locked`. In locked state the user's real identity
/// still exists in the unreachable keyring; persisting the ephemeral key to
/// `identity.key` would make it appear as a "different key" on next boot,
/// and the mismatched-file adoption path would then clobber the real keyring
/// key once the keyring becomes reachable again. The correct action in locked
/// state is to unlock the keyring and relaunch — not to adopt the ephemeral key.
#[tauri::command]
pub async fn persist_current_identity(
    app_handle: tauri::AppHandle,
) -> Result<IdentityInfo, String> {
    tokio::task::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();

        // Acquire mutation lock before reading identity_lost so that a
        // concurrent import_identity cannot complete between our check and
        // our persist, which would let the stale ephemeral key overwrite the
        // imported one.
        let _mutation_guard = state.identity_mutation.lock().map_err(|e| e.to_string())?;

        if !state
            .identity_lost
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err("identity is not in a lost state".to_string());
        }

        // Clone current keys without holding the mutex across keyring I/O.
        let keys = state.keys.lock().map_err(|e| e.to_string())?.clone();

        let data_dir = app_handle
            .path()
            .app_data_dir()
            .map_err(|e| format!("app data dir: {e}"))?;
        std::fs::create_dir_all(&data_dir).map_err(|e| format!("create app data dir: {e}"))?;
        let key_path = data_dir.join("identity.key");

        let store = crate::secret_store::SecretStore::shared(crate::app_state::keyring_service());
        crate::app_state::persist_imported_identity(store, &keys, &key_path, &data_dir)?;

        // Keys are already the live identity — only clear identity_lost.
        // Release pairs with Acquire in get_identity so readers see
        // consistent state.
        state
            .identity_lost
            .store(false, std::sync::atomic::Ordering::Release);

        let pubkey = keys.public_key();
        let pubkey_hex = pubkey.to_hex();
        let display_name = truncated_display_name(&pubkey)?;

        Ok(IdentityInfo {
            pubkey: pubkey_hex,
            display_name,
            lost: false,
            locked: false,
        })
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

fn nostr_bind_tag(name: &str, value: &str) -> Result<Tag, String> {
    Tag::parse(vec![name, value]).map_err(|error| format!("{name} tag failed: {error}"))
}

fn build_nostr_identity_binding_event(
    keys: &Keys,
    challenge_id: &str,
    nonce: &str,
    verification_code: &str,
    origin: &str,
    expires_at: &str,
) -> Result<Event, String> {
    nostr_bind::validate_signing_request(
        challenge_id,
        nonce,
        verification_code,
        origin,
        expires_at,
    )?;

    let tags = vec![
        nostr_bind_tag("challenge_id", challenge_id)?,
        nostr_bind_tag("nonce", nonce)?,
        nostr_bind_tag("verification_code", verification_code)?,
        nostr_bind_tag("audience", nostr_bind::AUDIENCE)?,
        nostr_bind_tag("action", nostr_bind::ACTION)?,
        nostr_bind_tag("protocol", nostr_bind::PROTOCOL)?,
        nostr_bind_tag("version", nostr_bind::VERSION)?,
        nostr_bind_tag("origin", origin)?,
        nostr_bind_tag("expires_at", expires_at)?,
    ];

    EventBuilder::new(Kind::Custom(nostr_bind::KIND), nostr_bind::CONTENT)
        .tags(tags)
        .sign_with_keys(keys)
        .map_err(|error| format!("sign failed: {error}"))
}

#[tauri::command]
pub async fn sign_nostr_identity_binding(
    challenge_id: String,
    nonce: String,
    verification_code: String,
    origin: String,
    expires_at: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    nostr_bind::validate_signing_request(
        &challenge_id,
        &nonce,
        &verification_code,
        &origin,
        &expires_at,
    )?;

    let keys = state
        .keys
        .lock()
        .map_err(|error| error.to_string())?
        .clone();

    tauri::async_runtime::spawn_blocking(move || {
        let event = build_nostr_identity_binding_event(
            &keys,
            &challenge_id,
            &nonce,
            &verification_code,
            &origin,
            &expires_at,
        )?;

        Ok(event.as_json())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn create_auth_event(
    challenge: String,
    relay_url: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let keys = state.signing_keys()?;

    tauri::async_runtime::spawn_blocking(move || {
        let tags = vec![
            Tag::parse(vec!["relay", &relay_url])
                .map_err(|error| format!("relay tag failed: {error}"))?,
            Tag::parse(vec!["challenge", &challenge])
                .map_err(|error| format!("challenge tag failed: {error}"))?,
        ];

        let event = EventBuilder::new(Kind::Custom(22242), "")
            .tags(tags)
            .sign_with_keys(&keys)
            .map_err(|error| format!("sign failed: {error}"))?;

        Ok(event.as_json())
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn nip44_encrypt_to_self(
    plaintext: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let keys = state.signing_keys()?;

    tauri::async_runtime::spawn_blocking(move || {
        nip44::encrypt(
            keys.secret_key(),
            &keys.public_key(),
            &plaintext,
            nip44::Version::V2,
        )
        .map_err(|e| format!("nip44 encrypt failed: {e}"))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[tauri::command]
pub async fn nip44_decrypt_from_self(
    ciphertext: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let keys = state.signing_keys()?;

    tauri::async_runtime::spawn_blocking(move || {
        nip44::decrypt(keys.secret_key(), &keys.public_key(), &ciphertext)
            .map_err(|e| format!("nip44 decrypt failed: {e}"))
    })
    .await
    .map_err(|e| format!("spawn_blocking failed: {e}"))?
}

#[cfg(test)]
mod nostr_identity_binding_tests {
    use super::build_nostr_identity_binding_event;
    use crate::nostr_bind;
    use nostr::{JsonUtil, Keys};

    fn tag_values(event: &nostr::Event) -> Vec<Vec<String>> {
        event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect()
    }

    #[test]
    fn build_nostr_identity_binding_event_signs_exact_shape() {
        let keys = Keys::generate();
        let event = build_nostr_identity_binding_event(
            &keys,
            "550e8400-e29b-41d4-a716-446655440000",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567",
            "123456",
            "https://example.com",
            "2999-01-01T00:00:00Z",
        )
        .unwrap();

        assert_eq!(event.kind.as_u16(), nostr_bind::KIND);
        assert_eq!(event.content, nostr_bind::CONTENT);
        assert_eq!(event.pubkey, keys.public_key());
        assert!(event.verify_id());
        assert!(event.verify_signature());
        assert!(nostr::Event::from_json(event.as_json()).is_ok());

        let tags = tag_values(&event);
        assert!(tags.contains(&vec![
            "challenge_id".into(),
            "550e8400-e29b-41d4-a716-446655440000".into(),
        ]));
        assert!(tags.contains(&vec![
            "nonce".into(),
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567".into(),
        ]));
        assert!(tags.contains(&vec!["verification_code".into(), "123456".into(),]));
        assert!(tags.contains(&vec!["audience".into(), "buzz:nostr-identity".into()]));
        assert!(tags.contains(&vec!["action".into(), "bind_nostr_identity".into(),]));
        assert!(tags.contains(&vec!["protocol".into(), "buzz-nostr-identity".into(),]));
        assert!(tags.contains(&vec!["version".into(), "1".into(),]));
        assert!(tags.contains(&vec!["origin".into(), "https://example.com".into(),]));
        assert!(tags.contains(&vec!["expires_at".into(), "2999-01-01T00:00:00Z".into(),]));
    }

    #[test]
    fn build_nostr_identity_binding_event_rejects_malformed_verification_code() {
        let keys = Keys::generate();
        let error = build_nostr_identity_binding_event(
            &keys,
            "550e8400-e29b-41d4-a716-446655440000",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567",
            "12345a",
            "https://example.com",
            "2999-01-01T00:00:00Z",
        )
        .unwrap_err();

        assert_eq!(error, "verification_code must be exactly 6 digits");
    }

    #[test]
    fn build_nostr_identity_binding_event_rejects_expired_link() {
        let keys = Keys::generate();
        let error = build_nostr_identity_binding_event(
            &keys,
            "550e8400-e29b-41d4-a716-446655440000",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghi01234567",
            "123456",
            "https://example.com",
            "2000-01-01T00:00:00Z",
        )
        .unwrap_err();

        assert_eq!(error, "expires_at is expired");
    }
}
