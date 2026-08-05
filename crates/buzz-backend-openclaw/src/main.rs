//! One-shot OpenClaw enrollment provider bundled with Buzz Desktop.
//!
//! The provider creates a short-lived, signed enrollment command for the
//! operator to run on the OpenClaw server. It is never a runtime proxy.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Map, Value};
use std::io::Read;
use uuid::Uuid;

const CODE_PREFIX: &str = "buzz-enroll-v1";
const CODE_TTL_SECONDS: u64 = 10 * 60;

fn info() -> Value {
    json!({
        "ok": true,
        "name": "openclaw",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": 1,
        "description": "Generates a one-time OpenClaw enrollment command",
        "config_schema": {
            "type": "object",
            "properties": {
                "rooms": { "type": "string", "format": "buzz-room-picker", "description": "Buzz rooms selected in Desktop" }
            },
            "required": ["rooms"]
        },
        "enrollment": {
            "operation": "enroll",
            "one_time": true,
            "credential_fields": ["private_key_nsec", "auth_tag", "relay_url"]
        }
    })
}

fn error(message: impl Into<String>) -> Value {
    json!({"ok": false, "error": message.into()})
}

fn enrollment_code(agent: &Value, rooms: &str) -> Result<(String, String), String> {
    let relay_url = agent
        .get("relay_url")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "agent relay_url is required".to_string())?;
    let private_key = agent
        .get("private_key_nsec")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "agent private_key_nsec is required".to_string())?;
    let keys =
        Keys::parse(private_key).map_err(|_| "agent private_key_nsec is invalid".to_string())?;
    let agent_id = format!("openclaw-{}", keys.public_key().to_hex()[..16].to_string());
    let room_ids = rooms
        .split(',')
        .map(str::trim)
        .filter(|room| !room.is_empty())
        .map(|id| json!({"id": id, "enabled": true, "requireMention": false}))
        .collect::<Vec<_>>();
    if room_ids.is_empty() {
        return Err("provider_config.rooms is required".to_string());
    }

    let now = Timestamp::now().as_secs();
    let expires_at = (now + CODE_TTL_SECONDS) * 1000;
    let nonce = Uuid::new_v4().simple().to_string();
    let mut payload = Map::new();
    payload.insert("version".into(), json!(1));
    payload.insert("relayUrl".into(), json!(relay_url));
    payload.insert("privateKey".into(), json!(private_key));
    if let Some(auth_tag) = agent.get("auth_tag").and_then(Value::as_str) {
        if !auth_tag.is_empty() {
            payload.insert("authTag".into(), json!(auth_tag));
        }
    }
    payload.insert("rooms".into(), Value::Array(room_ids));
    payload.insert("accountId".into(), json!(agent_id));
    payload.insert("agentId".into(), json!(agent_id));
    payload.insert(
        "defaultTo".into(),
        json!(rooms
            .split(',')
            .map(str::trim)
            .find(|room| !room.is_empty())
            .unwrap_or_default()),
    );

    let unsigned = json!({
        "version": 1,
        "expiresAt": expires_at,
        "nonce": nonce,
        "payload": Value::Object(payload),
    });
    let transcript = serde_json::to_string(&unsigned)
        .map_err(|_| "could not encode enrollment code".to_string())?;
    let event = EventBuilder::new(Kind::Custom(27235), transcript)
        .tags([
            Tag::parse(["expiration", &(now + CODE_TTL_SECONDS).to_string()])
                .map_err(|_| "could not create enrollment expiration".to_string())?,
        ])
        .custom_created_at(Timestamp::from(now))
        .sign_with_keys(&keys)
        .map_err(|_| "could not sign enrollment code".to_string())?;
    let mut signed = unsigned;
    signed["signature"] = serde_json::to_value(event)
        .map_err(|_| "could not encode enrollment signature".to_string())?;
    let encoded = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&signed)
            .map_err(|_| "could not encode enrollment command".to_string())?,
    );
    Ok((
        agent_id,
        format!("openclaw buzz enroll --code '{CODE_PREFIX}.{encoded}'"),
    ))
}

fn enroll(request: &Value) -> Value {
    let config = match request.get("provider_config").and_then(Value::as_object) {
        Some(config) => config,
        None => return error("provider_config must be an object"),
    };
    let rooms = match config.get("rooms").and_then(Value::as_str) {
        Some(rooms) => rooms,
        None => return error("provider_config.rooms is required"),
    };
    let agent = match request.get("agent") {
        Some(agent) if agent.is_object() => agent,
        _ => return error("agent payload is required"),
    };
    match enrollment_code(agent, rooms) {
        Ok((agent_id, command)) => {
            json!({"ok": true, "agent_id": agent_id, "enrollment_command": command})
        }
        Err(message) => error(message),
    }
}

fn respond(request: Value) -> Value {
    match request.get("op").and_then(Value::as_str) {
        Some("info") => info(),
        Some("enroll")
            if request
                .get("enrollment")
                .and_then(|value| value.get("version"))
                .and_then(Value::as_u64)
                == Some(1) =>
        {
            enroll(&request)
        }
        Some("enroll") => error("unsupported enrollment version"),
        _ => error("unsupported operation"),
    }
}

fn main() {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        std::process::exit(1);
    }
    let response = serde_json::from_str::<Value>(&input)
        .map(respond)
        .unwrap_or_else(|_| error("request is not valid JSON"));
    println!("{}", response);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_declares_rooms_only_for_one_time_v1_enrollment() {
        assert_eq!(info()["protocol_version"], 1);
        assert_eq!(info()["enrollment"]["one_time"], true);
        assert_eq!(info()["config_schema"]["required"], json!(["rooms"]));
        assert_eq!(
            info()["config_schema"]["properties"]["rooms"]["format"],
            "buzz-room-picker"
        );
    }

    #[test]
    fn rejects_missing_rooms_without_creating_command() {
        let response = respond(json!({
            "op": "enroll",
            "enrollment": {"version": 1},
            "agent": {}
        }));
        assert_eq!(response["ok"], false);
        assert!(response["error"].as_str().unwrap().contains("rooms"));
    }

    #[test]
    fn generates_copyable_command_without_ssh_or_code_config() {
        let keys = Keys::generate();
        let request = json!({
            "op": "enroll",
            "enrollment": {"version": 1, "mode": "one-time"},
            "agent": {
                "relay_url": "wss://relay.example",
                "private_key_nsec": keys.secret_key().to_bech32().unwrap(),
                "auth_tag": "[\"auth\",\"owner\",\"\",\"sig\"]"
            },
            "provider_config": {"rooms": "room-a,room-b"}
        });
        let response = respond(request);
        assert_eq!(response["ok"], true);
        assert!(response["enrollment_command"]
            .as_str()
            .unwrap()
            .starts_with("openclaw buzz enroll --code 'buzz-enroll-v1."));
    }
}
