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

#[tauri::command]
pub fn get_identity(state: State<'_, AppState>) -> Result<IdentityInfo, String> {
    let keys = state.keys.lock().map_err(|error| error.to_string())?;
    let pubkey = keys.public_key();
    let pubkey_hex = pubkey.to_hex();
    let bech32 = pubkey
        .to_bech32()
        .map_err(|error| format!("bech32 encode failed: {error}"))?;
    let display_name = if bech32.len() > 16 {
        format!("{}…{}", &bech32[..10], &bech32[bech32.len() - 4..])
    } else {
        bech32
    };

    Ok(IdentityInfo {
        pubkey: pubkey_hex,
        display_name,
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
    let keys = state
        .keys
        .lock()
        .map_err(|error| error.to_string())?
        .clone();

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
    let nsec = {
        let keys = state.keys.lock().map_err(|error| error.to_string())?;
        keys.secret_key()
            .to_bech32()
            .map_err(|error| format!("encode nsec: {error}"))?
    };
    let keys = Keys::parse(&nsec).map_err(|error| format!("parse nsec: {error}"))?;
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
    let nsec = {
        let keys = state.keys.lock().map_err(|error| error.to_string())?;
        keys.secret_key()
            .to_bech32()
            .map_err(|error| format!("encode nsec: {error}"))?
    };
    let keys = Keys::parse(&nsec).map_err(|error| format!("parse nsec: {error}"))?;
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
    let keys = state.keys.lock().map_err(|error| error.to_string())?;
    keys.secret_key()
        .to_bech32()
        .map_err(|error| format!("encode nsec: {error}"))
}

#[tauri::command]
pub fn import_identity(
    nsec: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<IdentityInfo, String> {
    let trimmed = nsec.trim();
    let keys = Keys::parse(trimmed).map_err(|e| format!("Invalid private key: {e}"))?;

    // Persist to identity.key
    let data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| format!("app data dir: {e}"))?;
    std::fs::create_dir_all(&data_dir).map_err(|e| format!("create app data dir: {e}"))?;
    let key_path = data_dir.join("identity.key");
    crate::app_state::save_key_file(&key_path, &keys)?;

    // Update in-memory keys
    let pubkey = keys.public_key();
    *state.keys.lock().map_err(|e| e.to_string())? = keys;

    let pubkey_hex = pubkey.to_hex();
    let bech32 = pubkey
        .to_bech32()
        .map_err(|error| format!("bech32 encode failed: {error}"))?;
    let display_name = if bech32.len() > 16 {
        format!("{}…{}", &bech32[..10], &bech32[bech32.len() - 4..])
    } else {
        bech32
    };

    eprintln!("buzz-desktop: imported identity pubkey {}", pubkey_hex);

    Ok(IdentityInfo {
        pubkey: pubkey_hex,
        display_name,
    })
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
    let keys = state
        .keys
        .lock()
        .map_err(|error| error.to_string())?
        .clone();

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
    let keys = state.keys.lock().map_err(|e| e.to_string())?.clone();

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
    let keys = state.keys.lock().map_err(|e| e.to_string())?.clone();

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
