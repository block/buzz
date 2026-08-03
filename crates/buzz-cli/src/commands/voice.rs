use serde_json::json;

use crate::commands::agents::require_owner;
use crate::error::CliError;
use crate::voice_management::{build, VoiceAgentRef, VoiceRoomCommand};
use crate::{client::BuzzClient, VoiceCmd};

pub async fn dispatch(command: VoiceCmd, client: &BuzzClient) -> Result<(), CliError> {
    let (action, command) = match command {
        VoiceCmd::Join(agent) => (
            "join",
            VoiceRoomCommand::Join(VoiceAgentRef::try_from(agent)?),
        ),
        VoiceCmd::Remove(agent) => (
            "remove",
            VoiceRoomCommand::Remove(VoiceAgentRef::try_from(agent)?),
        ),
        VoiceCmd::Mute(agent) => (
            "set-muted",
            VoiceRoomCommand::SetMuted {
                agent: VoiceAgentRef::try_from(agent)?,
                muted: true,
            },
        ),
        VoiceCmd::Unmute(agent) => (
            "set-muted",
            VoiceRoomCommand::SetMuted {
                agent: VoiceAgentRef::try_from(agent)?,
                muted: false,
            },
        ),
        VoiceCmd::SetVoice { agent, voice } => (
            "set-voice",
            VoiceRoomCommand::SetVoice {
                agent: VoiceAgentRef::try_from(agent)?,
                voice: voice.as_str().to_owned(),
            },
        ),
        VoiceCmd::MuteOutput => (
            "set-output-muted",
            VoiceRoomCommand::SetOutputMuted { muted: true },
        ),
        VoiceCmd::UnmuteOutput => (
            "set-output-muted",
            VoiceRoomCommand::SetOutputMuted { muted: false },
        ),
    };
    let owner = require_owner(client, "voice-room commands")?;
    let built = build(client.keys(), &owner, command)?;
    let response = client.publish_ephemeral_event(built.event).await?;
    let relay: serde_json::Value = serde_json::from_str(&response)
        .map_err(|error| CliError::Other(format!("invalid relay response: {error}")))?;
    println!(
        "{}",
        json!({
            "accepted": relay["accepted"],
            "event_id": relay["event_id"],
            "request_id": built.request_id,
            "action": action,
            "message": "Voice-room command sent to Buzz Desktop.",
        })
    );
    Ok(())
}
