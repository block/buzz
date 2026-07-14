use nostr::PublicKey;

use crate::agent_management::{build_create, build_update, CreateAgentDraft, UpdateAgentDraft};
use crate::client::BuzzClient;
use crate::error::CliError;
use crate::validate::read_or_stdin;
use crate::{AgentsCmd, RespondToArg};

pub async fn dispatch(command: AgentsCmd, client: &BuzzClient) -> Result<(), CliError> {
    let owner = client
        .auth_tag_owner_hex()
        .ok_or_else(|| CliError::Auth("agent draft requests require BUZZ_AUTH_TAG".into()))?;
    let owner = PublicKey::parse(&owner)
        .map_err(|error| CliError::Auth(format!("invalid owner attestation: {error}")))?;

    let built = match command {
        AgentsCmd::DraftCreate {
            channel,
            display_name,
            system_prompt,
        } => build_create(
            client.keys(),
            &owner,
            CreateAgentDraft {
                channel_id: channel,
                display_name,
                system_prompt: read_or_stdin(&system_prompt)?,
            },
        )?,
        AgentsCmd::DraftUpdate {
            channel,
            agent_name,
            display_name,
            system_prompt,
            runtime,
            provider,
            model,
            respond_to,
        } => build_update(
            client.keys(),
            &owner,
            UpdateAgentDraft {
                channel_id: channel,
                agent_name,
                display_name,
                system_prompt: system_prompt
                    .map(|value| read_or_stdin(&value))
                    .transpose()?,
                runtime,
                provider,
                model,
                respond_to: respond_to.map(RespondToArg::to_wire),
            },
        )?,
    };

    let response = client.publish_ephemeral_event(built.event).await?;
    let mut output: serde_json::Value = serde_json::from_str(&response)
        .map_err(|error| CliError::Other(format!("invalid relay response: {error}")))?;
    if let Some(object) = output.as_object_mut() {
        object.insert("request_id".into(), built.request_id.into());
        object.insert("action".into(), built.action.into());
        object.insert("saved".into(), false.into());
        object.insert(
            "message".into(),
            "Draft sent to Buzz Desktop for owner review. Nothing changes until the owner saves it."
                .into(),
        );
    }
    println!("{output}");
    Ok(())
}
