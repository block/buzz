//! Contract tests for the broker envelope, actions, and client trait.
//!
//! Split by concern; the fixtures and helpers every part shares stay here.
//!
//! These stay `#[cfg(test)]` inside `broker` to keep the split a pure move: the
//! file already lived here, and relocating it out-of-crate in the same change
//! would mix a move with a scope change. It is *not* because the tests need
//! private access — every item they name is public API, and they compile and
//! pass unchanged as an out-of-crate integration test. What they assert about
//! the private wire intermediaries (`WireResponse` in `broker/mod.rs`,
//! `StrictEvent` in `broker/actions/outcomes.rs`) is reached through the public
//! `Deserialize` impls, which is the same door a host author uses.
//!
//! One guard does depend on privacy, but on the *crate's*, not this module's:
//! `Dispatch`'s field is private to `client`, so no caller anywhere outside it —
//! in-crate test included — can forge the token and bypass `execute`. Tests here
//! implement `send` and never call it, which is why that holds.

mod client;
mod identities;
mod schema;
mod validation;
mod wire_schema;

use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag};

const CHANNEL: &str = "b2c38ca8-9ec3-411e-bab5-f9deab34d52e";
const PUBKEY: &str = "a02c4e0850e5e612b4ddf95dbe2f5c56467cf27c6552203bc833ff438fb31971";
const EVENT: &str = "78d47c4f36a2d048f45b57a31d964a3ce239f0fc46162c5d7c90db2b5aa52bc6";

fn pubkey() -> PubkeyHex {
    PubkeyHex::parse(PUBKEY).expect("fixture pubkey is valid hex")
}

/// A genuinely signed event, so read fixtures exercise real verification rather
/// than a hand-built value that could never verify.
fn signed_message(keys: &Keys) -> BrokerMessage {
    let event = EventBuilder::new(Kind::Custom(9), "hello")
        .tags([
            Tag::parse(["h", CHANNEL]).expect("h tag"),
            Tag::parse(["e", EVENT, "", "root"]).expect("e tag"),
            Tag::parse(["p", PUBKEY]).expect("p tag"),
        ])
        .sign_with_keys(keys)
        .expect("fixture event signs");
    BrokerMessage(event)
}

/// Every [`BrokerErrorCode`] variant, so code-driven tables cannot silently skip
/// one: [`error_codes_have_stable_wire_strings`] pins this list against the enum.
fn all_error_codes() -> [BrokerErrorCode; 11] {
    use BrokerErrorCode as E;
    [
        E::InvalidRequest,
        E::UnsupportedProtocolVersion,
        E::UnknownAction,
        E::UnsupportedActionVersion,
        E::Unsupported,
        E::Unauthenticated,
        E::Unauthorized,
        E::RequestIdConflict,
        E::ActionFailed,
        E::OutcomeUnknown,
        E::Internal,
    ]
}

/// One valid `args` value per action, so table-driven tests cannot silently
/// skip an action: [`fixtures_cover_every_action`] pins the coverage.
fn action_fixtures() -> Vec<ActionArgs> {
    vec![
        ActionArgs::ChannelRead(ChannelReadArgs {
            channel_id: CHANNEL.into(),
            root_event_id: Some(EVENT.into()),
            mentions_only: true,
            cursor: Some("opaque-host-cursor-v1".into()),
            limit: Some(50),
        }),
        ActionArgs::MessagePost(MessagePostArgs {
            channel_id: CHANNEL.into(),
            content: "shipping the contract".into(),
            mentions: vec![pubkey()],
        }),
        ActionArgs::MessageReply(MessageReplyArgs {
            channel_id: CHANNEL.into(),
            reply_to_event_id: EVENT.into(),
            content: "agreed".into(),
            mentions: vec![pubkey()],
        }),
        ActionArgs::ReactionAdd(ReactionAddArgs {
            channel_id: CHANNEL.into(),
            target_event_id: EVENT.into(),
            reaction: "🎉".into(),
        }),
        ActionArgs::ProfileSet(ProfileSetArgs {
            display_name: Some("ss-dev-00".into()),
            about: Some("implementation".into()),
            picture: Some("https://example.invalid/avatar.png".into()),
        }),
        ActionArgs::StorageAddress(StorageAddressArgs {
            slug: "mem/broker-foundation".into(),
        }),
        ActionArgs::AgentsCreate(AgentsCreateArgs {
            channel_id: CHANNEL.into(),
            display_name: "Research helper".into(),
            system_prompt: "Find sources.".into(),
            runtime: Some("buzz-acp".into()),
            provider: Some("anthropic".into()),
            model: Some("claude-sonnet-4-5".into()),
            respond_to: Some("owner-only".into()),
        }),
        ActionArgs::AgentsUpdate(AgentsUpdateArgs {
            target: AgentTarget::Pubkey(pubkey()),
            display_name: Some("Research helper v2".into()),
            system_prompt: Some("Find better sources.".into()),
            runtime: Some("buzz-acp".into()),
            provider: Some("anthropic".into()),
            model: Some("claude-sonnet-4-5".into()),
            respond_to: Some("anyone".into()),
        }),
        ActionArgs::AgentsDelete(AgentsDeleteArgs {
            target: AgentTarget::Name("Research helper".into()),
        }),
    ]
}

/// One outcome per action, matching the fixture order above.
fn outcome_fixtures(keys: &Keys) -> Vec<ActionOutcome> {
    let page = MessagePage {
        messages: vec![signed_message(keys)],
        next_cursor: Some("opaque-host-cursor-v2".into()),
    };
    let published = EventPublished {
        event_id: EVENT.into(),
        kind: 9,
        created_at: 1_764_000_003,
    };
    vec![
        ActionOutcome::ChannelRead(page),
        ActionOutcome::MessagePost(published.clone()),
        ActionOutcome::MessageReply(published.clone()),
        ActionOutcome::ReactionAdd(published.clone()),
        ActionOutcome::ProfileSet(published),
        ActionOutcome::StorageAddress(StorageAddress {
            author_pubkey: pubkey(),
            kind: 30174,
            d_tag: EVENT.into(),
        }),
        ActionOutcome::AgentsCreate(AgentsCreateOutcome {
            agent_pubkey: pubkey(),
            display_name: "Research helper".into(),
            channel_id: CHANNEL.into(),
        }),
        ActionOutcome::AgentsUpdate(AgentsUpdateOutcome {
            agent_pubkey: pubkey(),
            display_name: "Research helper v2".into(),
            updated_fields: vec!["displayName".into()],
        }),
        ActionOutcome::AgentsDelete(AgentsDeleteOutcome {
            agent_pubkey: pubkey(),
            display_name: "Research helper".into(),
        }),
    ]
}

fn prepared(args: ActionArgs) -> PreparedRequest {
    BrokerRequest::new("req-1", args)
        .expect("fixture request builds")
        .prepare()
        .expect("fixture request prepares")
}

/// Sorted JSON object keys of `value`, for exact-schema assertions.
fn keys_of(value: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .expect("expected a JSON object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}
