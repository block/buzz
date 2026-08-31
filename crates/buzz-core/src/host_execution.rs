//! Private, owner-authorized executor protocol. Registration is NOT host login.
//!
//! The caller must supply a freshly fetched, nondeleted registration from the
//! selected community, never a cached/caller-provided assertion of revocation.
//! This module is a protocol foundation; relays/executors must explicitly wire
//! authorization and a durable operation ledger before advertising Start.
use nostr::{nips::nip44, Event, EventBuilder, Keys, Kind, PublicKey, Tag, Timestamp};
use serde::{Deserialize, Serialize};

use crate::{
    host,
    kind::{KIND_HOST_COMMAND, KIND_HOST_RECEIPT},
};

/// Maximum command lifetime; expiry is not evidence of process termination.
pub const COMMAND_TTL: u64 = 300;
/// Executor protocol namespace.
pub const NAMESPACE: &str = "buzz.host.execution.v1";

/// Explicit destination-local configuration, or one exact run to stop.
/// No source paths, shell strings, environment or credentials are accepted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    /// Start only a pre-provisioned agent with this exact destination revision.
    Start {
        /// Rust runtime catalog ID, checked again on the destination.
        runtime: String,
        /// SHA-256 of destination launch configuration, not source configuration.
        revision: String,
    },
    /// Stop only the clicked launcher generation; never agent-wide shutdown.
    Stop {
        /// Public run ID, identical to the launcher's persisted generation.
        run: String,
    },
}

/// Encrypted immutable request. Retries must reuse the same signed event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Command {
    /// Protocol version.
    pub v: u8,
    /// Random 128-bit lowercase hex operation ID, also Start's run generation.
    pub operation: String,
    /// Canonical selected community URL. A command cannot cross communities.
    pub relay: String,
    /// Agent identity, independent of its placements.
    pub agent: String,
    /// Bounded execution deadline in Unix seconds.
    pub expires_at: u64,
    /// Requested transition.
    pub action: Action,
}

/// Process observations, never inferred from relay acceptance or online presence.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Intent persisted; no process claim.
    Accepted,
    /// A child was created, not necessarily listening or ready.
    Spawned,
    /// Authenticated same-generation harness lifecycle report.
    Listening,
    /// Authenticated same-generation harness readiness report.
    Ready,
    /// Selected root and its group exited, but separately grouped descendants
    /// have not been proven terminated. This MUST NOT authorize replacement.
    RootExited,
    /// Controller authenticated completion of the supported owned-work boundary
    /// and reaped the selected root. Not a universal arbitrary-daemon guarantee.
    Stopped,
    /// Proven pre-side-effect rejection (safe enum, no private diagnostics).
    Rejected,
    /// Side effect cannot be proved; must block replacement.
    Unknown,
}

/// Encrypted host-signed result, correlated to an exact immutable command.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    /// Protocol version.
    pub v: u8,
    /// Signed command event ID, not just a caller-selected operation ID.
    pub command: String,
    /// Original immutable request. Contains identifiers only, no launch secrets.
    pub request: Command,
    /// Exact process generation the observation describes.
    pub run: String,
    /// Original observation time, retained on receipt retransmission.
    pub observed_at: u64,
    /// Observed state; Unknown is not Stopped.
    pub outcome: Outcome,
}

/// Validate a fixed-length lowercase hex identifier.
pub fn hex_id(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Command {
    /// Validate immutable structure; use `decrypt_command` for execution freshness.
    pub fn validate(&self) -> Result<(), String> {
        if self.v != 1
            || !hex_id(&self.operation, 32)
            || !hex_id(&self.agent, 64)
            || crate::relay::normalize_relay_url(&self.relay)
                .ok()
                .as_deref()
                != Some(&self.relay)
        {
            return Err("invalid execution command".into());
        }
        PublicKey::from_hex(&self.agent).map_err(|_| "invalid agent key")?;
        match &self.action {
            Action::Start { runtime, revision }
                if runtime.is_empty()
                    || runtime.len() > 128
                    || !runtime
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                    || !hex_id(revision, 64) =>
            {
                Err("invalid destination configuration reference".into())
            }
            Action::Stop { run } if !hex_id(run, 32) => Err("invalid stop generation".into()),
            _ => Ok(()),
        }
    }

    /// Generation this operation is allowed to affect.
    pub fn run(&self) -> &str {
        match &self.action {
            Action::Start { .. } => &self.operation,
            Action::Stop { run } => run,
        }
    }
}

fn registration(reg: &Event) -> Result<host::Envelope, String> {
    let binding = host::validate(reg)?;
    if binding.label != "registration" {
        return Err("expected current registration".into());
    }
    Ok(binding)
}

fn build(
    signer: &Keys,
    recipient: PublicKey,
    reg: &Event,
    kind: u32,
    body: &impl Serialize,
    now: u64,
) -> Result<Event, String> {
    let binding = registration(reg)?;
    let content = nip44::encrypt(
        signer.secret_key(),
        &recipient,
        serde_json::to_string(body).map_err(|_| "invalid execution payload")?,
        nip44::Version::V2,
    )
    .map_err(|_| "execution encryption failed")?;
    let tags = [
        ["L".to_owned(), NAMESPACE.into()],
        ["p".into(), binding.owner.to_hex()],
        ["x".into(), binding.host.to_hex()],
        ["e".into(), reg.id.to_hex()],
    ]
    .into_iter()
    .map(Tag::parse)
    .collect::<Result<Vec<_>, _>>()
    .map_err(|_| "invalid routing")?;
    EventBuilder::new(Kind::Custom(kind as u16), content)
        .allow_self_tagging()
        .tags(tags)
        .custom_created_at(Timestamp::from(now))
        .sign_with_keys(signer)
        .map_err(|_| "execution signing failed".into())
}

fn envelope(event: &Event, reg: &Event, kind: u32, signer: PublicKey) -> Result<(), String> {
    crate::verify_event(event).map_err(|_| "invalid execution signature")?;
    let binding = registration(reg)?;
    let expected = [
        ("L", NAMESPACE.to_owned()),
        ("p", binding.owner.to_hex()),
        ("x", binding.host.to_hex()),
        ("e", reg.id.to_hex()),
    ];
    if event.kind.as_u16() as u32 != kind
        || event.pubkey != signer
        || event.tags.len() != expected.len()
        || event.content.len() > 16_384
    {
        return Err("invalid execution envelope".into());
    }
    for (key, value) in expected {
        if event
            .tags
            .iter()
            .filter(|tag| tag.as_slice() == [key, value.as_str()])
            .count()
            != 1
        {
            return Err("execution audience or registration mismatch".into());
        }
    }
    Ok(())
}

/// Encrypt to one registered host and sign as its owner. Existing owner transport
/// is required; possession of the host key grants no broader relay authority.
pub fn command(owner: &Keys, reg: &Event, request: &Command, now: u64) -> Result<Event, String> {
    let binding = registration(reg)?;
    if binding.owner != owner.public_key() {
        return Err("foreign execution owner".into());
    }
    request.validate()?;
    freshness(request, now, now)?;
    build(owner, binding.host, reg, KIND_HOST_COMMAND, request, now)
}

fn freshness(request: &Command, created: u64, now: u64) -> Result<(), String> {
    if created > now.saturating_add(30)
        || request.expires_at <= now
        || request.expires_at <= created
        || request.expires_at > created.saturating_add(COMMAND_TTL)
    {
        return Err("execution command expired or lifetime invalid".into());
    }
    Ok(())
}

/// Authenticate destination/owner/current registration, decrypt and check deadline
/// and community before consulting the durable dedup ledger or doing any work.
pub fn decrypt_command(
    host: &Keys,
    current_reg: &Event,
    event: &Event,
    relay: &str,
    now: u64,
) -> Result<Command, String> {
    let binding = registration(current_reg)?;
    if binding.host != host.public_key() {
        return Err("wrong executor".into());
    }
    envelope(event, current_reg, KIND_HOST_COMMAND, binding.owner)?;
    let text = nip44::decrypt(host.secret_key(), &binding.owner, &event.content)
        .map_err(|_| "invalid command ciphertext")?;
    let request: Command = serde_json::from_str(&text).map_err(|_| "invalid execution payload")?;
    request.validate()?;
    if request.relay != relay {
        return Err("wrong execution community".into());
    }
    freshness(&request, event.created_at.as_secs(), now)?;
    Ok(request)
}

/// Sign only an observed, durably recorded result; this helper does not turn an
/// online pulse or successful publish into an execution observation.
pub fn receipt(host: &Keys, reg: &Event, result: &Receipt, now: u64) -> Result<Event, String> {
    let binding = registration(reg)?;
    if binding.host != host.public_key() {
        return Err("wrong receipt signer".into());
    }
    validate_receipt(result)?;
    if result.observed_at > now {
        return Err("receipt observation is in the future".into());
    }
    build(host, binding.owner, reg, KIND_HOST_RECEIPT, result, now)
}

fn validate_receipt(result: &Receipt) -> Result<(), String> {
    result.request.validate()?;
    if result.v != 1 || !hex_id(&result.command, 64) || result.run != result.request.run() {
        return Err("invalid execution receipt correlation".into());
    }
    let valid = match result.request.action {
        Action::Start { .. } => !matches!(result.outcome, Outcome::Stopped | Outcome::RootExited),
        Action::Stop { .. } => matches!(
            result.outcome,
            Outcome::Accepted
                | Outcome::RootExited
                | Outcome::Stopped
                | Outcome::Rejected
                | Outcome::Unknown
        ),
    };
    if !valid {
        return Err("invalid receipt outcome for action".into());
    }
    Ok(())
}

/// Verify host authority and exact command/generation correlation. Late receipts
/// may resolve their original operation, never a newer operation. Unlike commands,
/// persisted results remain readable after expiry; expiry is not termination.
pub fn decrypt_receipt(
    owner: &Keys,
    reg: &Event,
    event: &Event,
    command_event: &Event,
    request: &Command,
) -> Result<Receipt, String> {
    let binding = registration(reg)?;
    if binding.owner != owner.public_key() {
        return Err("foreign receipt owner".into());
    }
    envelope(command_event, reg, KIND_HOST_COMMAND, binding.owner)?;
    // Bind the supplied request to the signed bytes, not caller-provided metadata.
    let sent = nip44::decrypt(owner.secret_key(), &binding.host, &command_event.content)
        .map_err(|_| "invalid command ciphertext")?;
    let sent: Command = serde_json::from_str(&sent).map_err(|_| "invalid command payload")?;
    if &sent != request {
        return Err("request differs from signed command".into());
    }
    envelope(event, reg, KIND_HOST_RECEIPT, binding.host)?;
    let text = nip44::decrypt(owner.secret_key(), &binding.host, &event.content)
        .map_err(|_| "invalid receipt ciphertext")?;
    let result: Receipt = serde_json::from_str(&text).map_err(|_| "invalid receipt payload")?;
    validate_receipt(&result)?;
    if result.observed_at > event.created_at.as_secs() {
        return Err("receipt observation is in the future".into());
    }
    if result.command != command_event.id.to_hex() || &result.request != request {
        return Err("receipt belongs to another operation".into());
    }
    Ok(result)
}

/// Validate the public routing envelope without decrypting execution payloads.
/// The caller must fetch this exact nondeleted registration in its community.
/// This grants only owner transport, never host login privileges.
pub fn validate_transport(event: &Event, reg: &Event, owner: PublicKey) -> Result<(), String> {
    let binding = registration(reg)?;
    if binding.owner != owner {
        return Err("foreign execution transport owner".into());
    }
    let kind = event.kind.as_u16() as u32;
    let signer = match kind {
        KIND_HOST_COMMAND => binding.owner,
        KIND_HOST_RECEIPT => binding.host,
        _ => return Err("invalid execution kind".into()),
    };
    envelope(event, reg, kind, signer)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Command {
        Command {
            v: 1,
            operation: "ab".repeat(16),
            relay: "wss://one.example".into(),
            agent: Keys::generate().public_key().to_hex(),
            expires_at: 200,
            action: Action::Start {
                runtime: "goose".into(),
                revision: "cd".repeat(32),
            },
        }
    }
    #[test]
    fn encrypted_start_and_exact_authenticated_result() {
        let owner = Keys::generate();
        let host = Keys::generate();
        let reg = host::registration(&owner, host.public_key(), 99).unwrap();
        let req = request();
        let cmd = command(&owner, &reg, &req, 100).unwrap();
        assert!(!cmd.content.contains(&req.agent));
        assert!(!cmd
            .tags
            .iter()
            .any(|t| t.as_slice().iter().any(|v| v.contains(&req.agent))));
        assert_eq!(
            decrypt_command(&host, &reg, &cmd, &req.relay, 101).unwrap(),
            req
        );
        let result = Receipt {
            v: 1,
            command: cmd.id.to_hex(),
            request: req.clone(),
            run: req.run().into(),
            observed_at: 101,
            outcome: Outcome::Spawned,
        };
        let event = receipt(&host, &reg, &result, 101).unwrap();
        assert!(receipt(&host, &reg, &result, 100).is_err());
        let replay = receipt(&host, &reg, &result, 300).unwrap();
        assert_eq!(
            decrypt_receipt(&owner, &reg, &replay, &cmd, &req)
                .unwrap()
                .observed_at,
            101
        );
        assert_eq!(
            decrypt_receipt(&owner, &reg, &event, &cmd, &req).unwrap(),
            result
        );
        let mut different = req.clone();
        different.operation = "ef".repeat(16);
        assert!(decrypt_receipt(&owner, &reg, &event, &cmd, &different).is_err());
        assert!(decrypt_receipt(&Keys::generate(), &reg, &event, &cmd, &req).is_err());
    }
    #[test]
    fn rejects_wrong_owner_host_tenant_registration_expiry_and_tampering() {
        let owner = Keys::generate();
        let host = Keys::generate();
        let req = request();
        let reg = host::registration(&owner, host.public_key(), 99).unwrap();
        assert!(command(&Keys::generate(), &reg, &req, 100).is_err());
        let cmd = command(&owner, &reg, &req, 100).unwrap();
        assert!(decrypt_command(&Keys::generate(), &reg, &cmd, &req.relay, 101).is_err());
        assert!(decrypt_command(&host, &reg, &cmd, "wss://two.example", 101).is_err());
        assert!(decrypt_command(&host, &reg, &cmd, &req.relay, 200).is_err());
        assert!(decrypt_command(&host, &reg, &cmd, &req.relay, 60).is_err());
        let renewed = host::registration(&owner, host.public_key(), 100).unwrap();
        assert!(decrypt_command(&host, &renewed, &cmd, &req.relay, 101).is_err());
        let mut tampered = cmd;
        tampered.content.push('x');
        assert!(decrypt_command(&host, &reg, &tampered, &req.relay, 101).is_err());
    }
    #[test]
    fn exact_stop_and_confused_outcomes_rejected() {
        let owner = Keys::generate();
        let host = Keys::generate();
        let reg = host::registration(&owner, host.public_key(), 99).unwrap();
        let mut req = request();
        req.action = Action::Stop {
            run: "12".repeat(16),
        };
        let cmd = command(&owner, &reg, &req, 100).unwrap();
        let mut result = Receipt {
            v: 1,
            command: cmd.id.to_hex(),
            request: req.clone(),
            run: req.run().into(),
            observed_at: 101,
            outcome: Outcome::Stopped,
        };
        assert!(receipt(&host, &reg, &result, 201).is_ok());
        result.run = req.operation.clone();
        assert!(receipt(&host, &reg, &result, 201).is_err());
        result.run = req.run().into();
        result.outcome = Outcome::Ready;
        assert!(receipt(&host, &reg, &result, 201).is_err());
        req.expires_at = 401;
        assert!(command(&owner, &reg, &req, 100).is_err());
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    #[test]
    fn private_transport_requires_exact_owner_registration_and_host_signature() {
        let owner = Keys::generate();
        let host = Keys::generate();
        let stranger = Keys::generate();
        let reg = host::registration(&owner, host.public_key(), 100).unwrap();
        let request = Command {
            v: 1,
            operation: "ab".repeat(16),
            relay: "wss://relay.example".into(),
            agent: Keys::generate().public_key().to_hex(),
            expires_at: 400,
            action: Action::Start {
                runtime: "goose".into(),
                revision: "cd".repeat(32),
            },
        };
        let command = command(&owner, &reg, &request, 100).unwrap();
        let result = Receipt {
            v: 1,
            command: command.id.to_hex(),
            run: request.run().into(),
            request: request.clone(),
            observed_at: 101,
            outcome: Outcome::Spawned,
        };
        let receipt = receipt(&host, &reg, &result, 101).unwrap();
        for event in [&command, &receipt] {
            assert!(validate_transport(event, &reg, owner.public_key()).is_ok());
            assert!(validate_transport(event, &reg, host.public_key()).is_err());
            assert!(validate_transport(event, &reg, stranger.public_key()).is_err());
            assert!(validate_transport(
                event,
                &host::registration(&owner, host.public_key(), 99).unwrap(),
                owner.public_key()
            )
            .is_err());
            assert!(!crate::filter::reader_authorized_for_event(
                event,
                &stranger.public_key().to_hex()
            ));
            assert!(crate::filter::reader_authorized_for_event(
                event,
                &owner.public_key().to_hex()
            ));
            let forged = EventBuilder::new(event.kind, event.content.clone())
                .tags(event.tags.clone())
                .allow_self_tagging()
                .sign_with_keys(&stranger)
                .unwrap();
            assert!(validate_transport(&forged, &reg, owner.public_key()).is_err());
        }
        assert!(decrypt_command(&host, &reg, &command, &request.relay, 400).is_err());
        assert!(decrypt_receipt(&owner, &reg, &receipt, &command, &request).is_ok());
    }
}
