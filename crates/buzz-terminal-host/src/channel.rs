use buzz_sdk::{
    build_add_member, build_create_channel, build_join, ChannelKind, MemberRole, Visibility,
};
use nostr::{EventBuilder, PublicKey};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChannelRequest {
    channel_id: Uuid,
    pubkey: Option<String>,
}

pub(super) fn build(
    method: &str,
    params: Value,
) -> Result<EventBuilder, (&'static str, String, Option<String>)> {
    let params: ChannelRequest = serde_json::from_value(params)
        .map_err(|_| ("invalid_params", "channelId must be a UUID".into(), None))?;
    let result = match method {
        "createChannel" => build_create_channel(
            params.channel_id,
            &format!("terminal-{}", params.channel_id),
            Some(Visibility::Private),
            Some(ChannelKind::Stream),
            None,
            None,
        ),
        "joinChannel" => build_join(params.channel_id),
        "addMember" => {
            let key = params
                .pubkey
                .as_deref()
                .and_then(|key| PublicKey::from_hex(key).ok())
                .ok_or((
                    "invalid_params",
                    "pubkey must be a hex public key".into(),
                    None,
                ))?;
            build_add_member(params.channel_id, &key.to_hex(), Some(MemberRole::Bot))
        }
        _ => return Err(("method_not_found", "unknown channel operation".into(), None)),
    };
    result.map_err(|error| ("invalid_params", error.to_string(), None))
}
