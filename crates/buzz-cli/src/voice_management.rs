//! Voice-room commands published through owner-encrypted observer frames.

use buzz_core::observer::{encrypt_observer_payload, OBSERVER_FRAME_TELEMETRY};
use nostr::{Event, Keys, PublicKey};
use serde::Serialize;

use crate::error::CliError;
use crate::VoiceAgentArgs;

const REQUEST_KIND: &str = "voice_room_command";
const MAX_NAME_CHARS: usize = 120;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VoiceAgentRef {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_pubkey: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum VoiceRoomCommand {
    Join(VoiceAgentRef),
    Remove(VoiceAgentRef),
    SetMuted {
        #[serde(flatten)]
        agent: VoiceAgentRef,
        muted: bool,
    },
    SetVoice {
        #[serde(flatten)]
        agent: VoiceAgentRef,
        voice: String,
    },
    SetOutputMuted {
        muted: bool,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct VoiceRoomRequest {
    #[serde(rename = "type")]
    request_type: &'static str,
    command: VoiceRoomCommand,
    request_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ObserverEvent {
    seq: u64,
    timestamp: String,
    kind: &'static str,
    agent_index: Option<usize>,
    channel_id: Option<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    payload: VoiceRoomRequest,
}

pub struct BuiltVoiceRequest {
    pub event: Event,
    pub request_id: String,
}

impl TryFrom<VoiceAgentArgs> for VoiceAgentRef {
    type Error = CliError;

    fn try_from(value: VoiceAgentArgs) -> Result<Self, Self::Error> {
        fn clean(
            value: Option<String>,
            label: &str,
            max: usize,
        ) -> Result<Option<String>, CliError> {
            value
                .map(|value| {
                    let value = value.trim();
                    if value.is_empty() {
                        return Err(CliError::Usage(format!("{label} cannot be empty")));
                    }
                    if value.chars().count() > max {
                        return Err(CliError::Usage(format!(
                            "{label} is too long (max {max} characters)"
                        )));
                    }
                    Ok(value.to_owned())
                })
                .transpose()
        }

        let reference = Self {
            agent_name: clean(value.agent_name, "agent name", MAX_NAME_CHARS)?,
            agent_pubkey: clean(value.agent_pubkey, "agent pubkey", 64)?,
            thread_id: clean(value.thread_id, "thread id", 128)?,
        };
        if reference.agent_name.is_none()
            && reference.agent_pubkey.is_none()
            && reference.thread_id.is_none()
        {
            return Err(CliError::Usage(
                "provide --agent-name, --agent-pubkey, or --thread-id".into(),
            ));
        }
        Ok(reference)
    }
}

pub fn build(
    keys: &Keys,
    owner: &PublicKey,
    command: VoiceRoomCommand,
) -> Result<BuiltVoiceRequest, CliError> {
    let request_id = uuid::Uuid::new_v4().to_string();
    let payload = ObserverEvent {
        seq: 0,
        timestamp: chrono::Utc::now().to_rfc3339(),
        kind: REQUEST_KIND,
        agent_index: None,
        channel_id: None,
        session_id: None,
        turn_id: None,
        payload: VoiceRoomRequest {
            request_type: REQUEST_KIND,
            command,
            request_id: request_id.clone(),
        },
    };
    let encrypted = encrypt_observer_payload(keys, owner, &payload)
        .map_err(|error| CliError::Other(format!("could not encrypt voice command: {error}")))?;
    let event = buzz_sdk::build_agent_observer_frame(
        &owner.to_hex(),
        &keys.public_key().to_hex(),
        OBSERVER_FRAME_TELEMETRY,
        &encrypted,
    )
    .map_err(|error| CliError::Other(format!("could not build voice command: {error}")))?
    .sign_with_keys(keys)
    .map_err(|error| CliError::Other(format!("could not sign voice command: {error}")))?;
    Ok(BuiltVoiceRequest { event, request_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::observer::decrypt_observer_payload;

    #[test]
    fn command_is_owner_encrypted_and_matches_desktop_contract() {
        let agent = Keys::generate();
        let owner = Keys::generate();
        let built = build(
            &agent,
            &owner.public_key(),
            VoiceRoomCommand::Join(VoiceAgentRef {
                agent_name: Some("Architect".into()),
                agent_pubkey: None,
                thread_id: None,
            }),
        )
        .unwrap();

        let payload: serde_json::Value = decrypt_observer_payload(&owner, &built.event).unwrap();
        assert_eq!(payload["kind"], REQUEST_KIND);
        assert_eq!(payload["payload"]["type"], REQUEST_KIND);
        assert_eq!(payload["payload"]["command"]["action"], "join");
        assert_eq!(payload["payload"]["command"]["agentName"], "Architect");
    }

    #[test]
    fn agent_reference_is_required() {
        let error = VoiceAgentRef::try_from(VoiceAgentArgs {
            agent_name: None,
            agent_pubkey: None,
            thread_id: None,
        })
        .unwrap_err();
        assert!(error.to_string().contains("provide --agent-name"));
    }
}
