//! Consume NIP-DA scoped-data grants at session assembly and apply them to
//! the agent's memory.
//!
//! ## The gap this closes
//!
//! Delegation on Buzz works today: a publisher gift-wraps a `kind:440` Data
//! Grant to an agent's key and the relay stores it. But nothing in the harness
//! ever *consumes* it — the subscription set pipes stream messages, approvals,
//! reminders, membership and observer frames, and never issues a `REQ` for the
//! agent's `kind:1059` gift-wraps. So a granted capability sits on the relay,
//! addressed correctly to the agent, and never reaches its context.
//!
//! `sync_grants` is the missing runtime step. Per *NIP-DA* (`REPOS/nips/DA.md`)
//! and the reference design in `OUTBOX/GRANT_INGESTION_HANDLER.md`:
//!
//! 1. **Fetch** — `REQ {kinds:[1059], "#p":[agent_pk]}` for gift-wraps.
//! 2. **Unwrap (NIP-59)** — two NIP-44 decrypts: outer wrapper (ephemeral →
//!    agent) yields a `kind:13` seal; the seal (grantor → agent) yields the
//!    inner `kind:440` rumor. The seal's signature authenticates the grantor;
//!    the rumor itself is unsigned and deniable, by design.
//! 3. **Read the grant** — `a` tag → `30440:<publisher>:<scope-id>`; content
//!    JSON carries the 32-byte scope key (base64) and generation `v`.
//! 4. **Dereference the data set** — fetch the `kind:30440` addressable event.
//!    Security §6: verify its signature *and* that its pubkey matches the `a`
//!    tag before decrypting.
//! 5. **Decrypt (NIP-44 v2)** — the 32-byte scope key is used *directly* as the
//!    conversation key (no ECDH), per DA.md "Payload encryption".
//! 6. **Apply** — write the decrypted payload into the agent's NIP-AE engram
//!    store (`Body::Memory`), the same surface `buzz mem set` writes to, so the
//!    agent can read the granted scope on demand on its next turn.
//!
//! Revocation is emergent: the publisher rotates the scope key and bumps `v`;
//! the held key then fails to decrypt the republished data set (step 5 errors),
//! so the scope simply stops refreshing.
//!
//! ## Fail-open, like [`crate::engram_fetch`]
//!
//! Every error path here is non-fatal: a transport failure, an undecryptable
//! wrap, or a missing data set logs and is skipped. Session creation is never
//! blocked, and one bad grant never poisons the others.
//!
//! ## Relay-auth requirement
//!
//! Fetching the agent's own gift-wraps requires a valid per-agent relay auth
//! (NIP-98 + `BUZZ_AUTH_TAG`). The harness [`RestClient`] already holds this —
//! it authenticates the same connection it uses to read `kind:30174` engrams —
//! and the Buzz relay explicitly exempts `kind:1059` from the
//! author-equals-authed-key check (`buzz-relay/src/handlers/event.rs`), so an
//! authed member can read gift-wraps addressed to it.
//!
//! ## Where the data set lives
//!
//! The `kind:30440` data set is NOT a registered Buzz kind, so today it lives
//! on the publisher's external relays (the grant's `a`-tag carries relay
//! hints). [`fetch_data_set`] queries the connected Buzz relay first; when it
//! doesn't hold the data set (the common external-only case),
//! [`fetch_data_set_cross_relay`] dereferences it directly from the grant's
//! `a`-tag relay hints over a plain NIP-01 WebSocket. This keeps the harness
//! decoupled from where the publisher chose to store the data set, and from
//! whether the NIP is registered on the Buzz relay. A hint relay is untrusted:
//! whatever it serves still passes the full `decrypt_data_set` verification
//! before it is applied. The unwrap + decrypt path is identical either way.

use std::time::Duration;

use base64::Engine;
use buzz_core::engram::{self, Body};
use futures_util::{SinkExt, StreamExt};
use nostr::{Alphabet, Event, Keys, PublicKey, SingleLetterTag, UnsignedEvent};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::relay::{parse_relay_message, RelayMessage, RestClient};

/// NIP-59 gift wrap.
const KIND_GIFT_WRAP: u16 = 1059;
/// NIP-59 seal.
const KIND_SEAL: u16 = 13;
/// NIP-DA Data Grant rumor.
const KIND_DATA_GRANT: u16 = 440;
/// NIP-DA Scoped Data Set (addressable).
const KIND_SCOPED_DATA_SET: u16 = 30440;

/// Max gift-wraps to pull in one sync. Bounds a hostile flood; a real grantee
/// holds a handful of scopes, not thousands.
const GIFT_WRAP_LIMIT: usize = 64;

/// Per-relay bound for a cross-relay data-set dereference. Kept tight because
/// the whole sync runs inside `pool.rs`'s session-start budget; a hint that
/// doesn't answer quickly is skipped and retried next session (fail-open).
const CROSS_RELAY_TIMEOUT: Duration = Duration::from_secs(2);

/// Max relay hints to try when dereferencing one data set. Bounds a grant that
/// carries a hostile pile of hints.
const MAX_RELAY_HINTS: usize = 4;

/// A scope reference extracted from a decrypted `kind:440` grant rumor.
#[derive(Debug, Clone)]
struct GrantRef {
    /// Publisher pubkey, from the `a` tag (`30440:<pubkey>:<scope-id>`).
    publisher: PublicKey,
    /// Opaque scope id — the data set's `d` tag.
    scope_id: String,
    /// Relay hints from the `a` tag (positions 2+); may be empty.
    relay_hints: Vec<String>,
    /// Key generation this grant corresponds to (`v` tag), if present.
    generation: Option<u64>,
    /// Optional NIP-40 expiration (unix seconds).
    expiration: Option<u64>,
    /// The 32-byte scope key, decoded from the rumor content.
    scope_key: [u8; 32],
    /// Optional human-readable scope name from the rumor content.
    scope_name: Option<String>,
}

/// A grant successfully unwrapped, dereferenced, decrypted and written to the
/// agent's engram store.
#[derive(Debug, Clone)]
pub struct AppliedGrant {
    /// The engram slug the scope payload was written to.
    pub slug: String,
    /// Human-readable scope name, if the grant carried one.
    pub scope_name: Option<String>,
    /// The publisher who issued the grant.
    pub publisher: PublicKey,
}

/// Fetch, unwrap, dereference, decrypt and apply every grant addressed to the
/// agent. Returns the scopes that were applied this run. Fail-open: any error
/// on a single grant is logged and skipped; a fetch failure returns an empty
/// vec without blocking session creation.
pub async fn sync_grants(
    rest: &RestClient,
    agent_keys: &Keys,
    owner: &PublicKey,
) -> Vec<AppliedGrant> {
    let wraps = match fetch_gift_wraps(rest, &agent_keys.public_key()).await {
        Ok(w) => w,
        Err(reason) => {
            tracing::warn!(
                target: "grant_sync",
                "gift-wrap fetch failed: {reason} — applying no grants this session"
            );
            return Vec::new();
        }
    };

    let mut applied = Vec::new();
    for wrap in &wraps {
        match apply_one(rest, agent_keys, owner, wrap).await {
            Ok(Some(grant)) => {
                tracing::info!(
                    target: "grant_sync",
                    slug = %grant.slug,
                    publisher = %grant.publisher.to_hex(),
                    "applied NIP-DA grant to engram memory"
                );
                applied.push(grant);
            }
            // Not a grant (e.g. a NIP-17 DM wrap), lapsed, or for another
            // recipient — silently skip. This is the common case.
            Ok(None) => {}
            Err(reason) => {
                tracing::warn!(
                    target: "grant_sync",
                    wrap_id = %wrap.id,
                    "grant skipped: {reason}"
                );
            }
        }
    }
    applied
}

/// Process a single gift-wrap end to end. Returns `Ok(None)` when the wrap is
/// legitimately not an applicable grant (wrong inner kind, lapsed, or a data
/// set we can't reach), and `Err` only for genuine anomalies worth logging.
async fn apply_one(
    rest: &RestClient,
    agent_keys: &Keys,
    owner: &PublicKey,
    wrap: &Event,
) -> Result<Option<AppliedGrant>, String> {
    // NIP-44 requires verifying the outer signature before decrypting. The
    // gift-wrap is signed by an ephemeral key; we only check the signature is
    // valid, not who holds it (that's the whole point of the ephemeral key).
    if wrap.verify().is_err() {
        return Err("gift-wrap signature invalid".into());
    }

    let (rumor, grantor) = unwrap_gift_wrap(wrap, agent_keys)?;

    // The wrap may carry any sealed rumor (NIP-17 DMs share this envelope).
    // Anything that isn't a Data Grant is simply not ours to apply.
    if rumor.kind.as_u16() != KIND_DATA_GRANT {
        return Ok(None);
    }

    let grant = parse_grant(&rumor)?;

    // The rumor is authored by the publisher; the seal is signed by the
    // grantor. For a first-party grant these are the same key. We don't hard-
    // fail on a mismatch (a delegated issuer is conceivable), but it's worth a
    // breadcrumb if they ever diverge.
    if grantor != grant.publisher {
        tracing::debug!(
            target: "grant_sync",
            grantor = %grantor.to_hex(),
            publisher = %grant.publisher.to_hex(),
            "grant seal signer differs from data-set publisher"
        );
    }

    // NIP-40: an expired grant is advisory-lapsed; honest clients stop applying.
    if let Some(exp) = grant.expiration {
        if now() >= exp {
            return Ok(None);
        }
    }

    // Prefer the connected Buzz relay; fall back to the grant's `a`-tag relay
    // hints when it doesn't hold the data set (the common external-only case).
    let data_set = match fetch_data_set(rest, &grant).await? {
        Some(ds) => ds,
        None => match fetch_data_set_cross_relay(&grant).await {
            Some(ds) => ds,
            None => {
                tracing::info!(
                    target: "grant_sync",
                    addr = %format!("30440:{}:{}", grant.publisher.to_hex(), grant.scope_id),
                    hints = ?grant.relay_hints,
                    "data set not found on connected relay or any hinted relay — skipping"
                );
                return Ok(None);
            }
        },
    };

    let payload = decrypt_data_set(&data_set, &grant)?;

    let slug = scope_slug(&grant);
    let body = Body::Memory {
        slug: slug.clone(),
        value: Some(payload),
    };
    let event = engram::build_event(agent_keys, owner, &body, now())
        .map_err(|e| format!("engram build failed: {e}"))?;
    rest.submit_event(&event)
        .await
        .map_err(|e| format!("engram submit failed: {e}"))?;

    Ok(Some(AppliedGrant {
        slug,
        scope_name: grant.scope_name,
        publisher: grant.publisher,
    }))
}

/// NIP-59 unwrap: gift wrap → seal → rumor. Returns the inner rumor and the
/// grantor pubkey (the seal's signer). Pure: no I/O.
///
/// * Outer: `nip44::decrypt(agent_sk, ephemeral_pk, wrap.content)` → seal JSON.
/// * The seal is a signed `kind:13`; we verify its signature (this is what
///   authenticates the grantor — the rumor is intentionally unsigned).
/// * Inner: `nip44::decrypt(agent_sk, seal_pk, seal.content)` → rumor JSON.
fn unwrap_gift_wrap(
    wrap: &Event,
    agent_keys: &Keys,
) -> Result<(UnsignedEvent, PublicKey), String> {
    if wrap.kind.as_u16() != KIND_GIFT_WRAP {
        return Err(format!("outer kind {} != 1059", wrap.kind.as_u16()));
    }

    let seal_json =
        nostr::nips::nip44::decrypt(agent_keys.secret_key(), &wrap.pubkey, &wrap.content)
            .map_err(|_| "gift-wrap outer decrypt failed".to_string())?;
    let seal: Event =
        serde_json::from_str(&seal_json).map_err(|e| format!("seal parse failed: {e}"))?;

    if seal.kind.as_u16() != KIND_SEAL {
        return Err(format!("inner kind {} != 13 (seal)", seal.kind.as_u16()));
    }
    // The seal is the only signed layer — this is what proves the grantor's
    // identity. Fail closed if it doesn't verify.
    if seal.verify().is_err() {
        return Err("seal signature invalid".into());
    }

    let rumor_json =
        nostr::nips::nip44::decrypt(agent_keys.secret_key(), &seal.pubkey, &seal.content)
            .map_err(|_| "seal decrypt failed".to_string())?;
    let rumor: UnsignedEvent =
        serde_json::from_str(&rumor_json).map_err(|e| format!("rumor parse failed: {e}"))?;

    // NIP-59: the rumor's author must equal the seal's signer, else a grantor
    // could seal a rumor claiming someone else's authorship.
    if rumor.pubkey != seal.pubkey {
        return Err("rumor pubkey does not match seal signer".into());
    }

    Ok((rumor, seal.pubkey))
}

/// Parse a `kind:440` rumor into a [`GrantRef`]. Pure: no I/O.
fn parse_grant(rumor: &UnsignedEvent) -> Result<GrantRef, String> {
    if rumor.kind.as_u16() != KIND_DATA_GRANT {
        return Err(format!("rumor kind {} != 440", rumor.kind.as_u16()));
    }

    // `a` tag: ["a", "30440:<pubkey>:<scope-id>", "<relay-hint>", ...]
    let a_tag = rumor
        .tags
        .iter()
        .find(|t| t.kind().to_string() == "a")
        .ok_or("grant missing `a` tag")?;
    let a_parts = a_tag.as_slice();
    let addr = a_parts
        .get(1)
        .ok_or("grant `a` tag has no value")?
        .as_str();
    let relay_hints: Vec<String> = a_parts.iter().skip(2).cloned().collect();

    // "30440:<pubkey>:<scope-id>" — scope-id is opaque and MAY contain ':',
    // so split into at most three parts and keep the remainder as the id.
    let mut addr_parts = addr.splitn(3, ':');
    let kind_str = addr_parts.next().unwrap_or_default();
    if kind_str.parse::<u16>().ok() != Some(KIND_SCOPED_DATA_SET) {
        return Err(format!("grant `a` tag kind {kind_str} != 30440"));
    }
    let pubkey_hex = addr_parts.next().ok_or("grant `a` tag missing pubkey")?;
    let publisher = PublicKey::from_hex(pubkey_hex)
        .map_err(|e| format!("grant `a` tag pubkey invalid: {e}"))?;
    let scope_id = addr_parts
        .next()
        .ok_or("grant `a` tag missing scope id")?
        .to_string();
    if scope_id.is_empty() {
        return Err("grant `a` tag scope id empty".into());
    }

    let generation = rumor
        .tags
        .iter()
        .find(|t| t.kind().to_string() == "v")
        .and_then(|t| t.content())
        .and_then(|s| s.parse::<u64>().ok());

    let expiration = rumor
        .tags
        .iter()
        .find(|t| t.kind().to_string() == "expiration")
        .and_then(|t| t.content())
        .and_then(|s| s.parse::<u64>().ok());

    // content: {"scope_key":"<base64-32-bytes>","scope_name":"..."}
    let content: GrantContent = serde_json::from_str(&rumor.content)
        .map_err(|e| format!("grant content JSON invalid: {e}"))?;
    let key_bytes = base64::engine::general_purpose::STANDARD
        .decode(content.scope_key.as_bytes())
        .map_err(|e| format!("scope_key not valid base64: {e}"))?;
    let scope_key: [u8; 32] = key_bytes
        .as_slice()
        .try_into()
        .map_err(|_| format!("scope_key is {} bytes, expected 32", key_bytes.len()))?;

    Ok(GrantRef {
        publisher,
        scope_id,
        relay_hints,
        generation,
        expiration,
        scope_key,
        scope_name: content.scope_name,
    })
}

/// The JSON shape of a `kind:440` rumor's `content`.
#[derive(serde::Deserialize)]
struct GrantContent {
    scope_key: String,
    #[serde(default)]
    scope_name: Option<String>,
}

/// Verify and decrypt a `kind:30440` data set against a held grant. Pure: no
/// I/O. Enforces DA.md Security §6 (signature + `a`-tag pubkey match) before
/// attempting decryption.
fn decrypt_data_set(data_set: &Event, grant: &GrantRef) -> Result<String, String> {
    if data_set.kind.as_u16() != KIND_SCOPED_DATA_SET {
        return Err(format!("data set kind {} != 30440", data_set.kind.as_u16()));
    }
    // Security §6: only the publisher can sign a replacement; verify before
    // decrypting so a forged data set can't feed the agent poisoned context.
    if data_set.verify().is_err() {
        return Err("data set signature invalid".into());
    }
    if data_set.pubkey != grant.publisher {
        return Err("data set pubkey does not match grant `a` tag publisher".into());
    }
    let d_value = data_set
        .tags
        .iter()
        .find(|t| t.kind().to_string() == "d")
        .and_then(|t| t.content())
        .ok_or("data set missing `d` tag")?;
    if d_value != grant.scope_id {
        return Err("data set `d` tag does not match grant scope id".into());
    }

    // Generation staleness is advisory: if the data set's `v` has advanced past
    // the grant's, this held key was likely rotated out. We still *attempt*
    // decryption (a stale `v` tag could be a rollback attack), and let the
    // decrypt result be the source of truth: a rotated key simply won't
    // decrypt, which is the real revocation signal.
    if let (Some(grant_v), Some(ds_v)) = (grant.generation, data_set_generation(data_set)) {
        if ds_v > grant_v {
            tracing::debug!(
                target: "grant_sync",
                grant_v,
                ds_v,
                "data set generation newer than grant — key may be revoked"
            );
        }
    }

    // NIP-DA "Payload encryption": the 32-byte scope key is used directly as
    // the NIP-44 v2 conversation key — no ECDH. The `content` is the standard
    // base64 NIP-44 payload string.
    let conversation_key = nostr::nips::nip44::v2::ConversationKey::new(grant.scope_key);
    let payload_bytes = base64::engine::general_purpose::STANDARD
        .decode(data_set.content.as_bytes())
        .map_err(|e| format!("data set content not base64: {e}"))?;
    let plaintext = nostr::nips::nip44::v2::decrypt_to_bytes(&conversation_key, &payload_bytes)
        .map_err(|_| "data set decrypt failed (scope key rotated/revoked?)".to_string())?;
    String::from_utf8(plaintext).map_err(|e| format!("decrypted payload not UTF-8: {e}"))
}

/// The `v` generation of a data set, if it carries one.
fn data_set_generation(data_set: &Event) -> Option<u64> {
    data_set
        .tags
        .iter()
        .find(|t| t.kind().to_string() == "v")
        .and_then(|t| t.content())
        .and_then(|s| s.parse::<u64>().ok())
}

/// The engram slug a scope's payload is written to. Kept to a single `mem/`
/// segment (the shape the core-memory model uses for cold slugs) and namespaced
/// by scope id so distinct grants never collide.
fn scope_slug(grant: &GrantRef) -> String {
    format!("mem/grant-{}", grant.scope_id)
}

/// Query the connected relay for gift-wraps addressed to the agent.
async fn fetch_gift_wraps(rest: &RestClient, agent_pk: &PublicKey) -> Result<Vec<Event>, String> {
    let filter = nostr::Filter::new()
        .kind(nostr::Kind::Custom(KIND_GIFT_WRAP))
        .custom_tags(
            SingleLetterTag::lowercase(Alphabet::P),
            [agent_pk.to_hex()],
        )
        .limit(GIFT_WRAP_LIMIT);

    let value = rest
        .query(&[filter])
        .await
        .map_err(|e| format!("relay query failed: {e}"))?;
    let arr = value
        .as_array()
        .ok_or_else(|| "relay query returned non-array".to_string())?;
    Ok(arr
        .iter()
        .filter_map(|v| serde_json::from_value::<Event>(v.clone()).ok())
        .collect())
}

/// Dereference the `kind:30440` data set named by a grant, from the connected
/// relay. Returns the newest matching event, or `None` if the relay holds none
/// (the external-only case — see module docs). Cross-relay fetch honouring
/// `grant.relay_hints` is the pending follow-up.
async fn fetch_data_set(rest: &RestClient, grant: &GrantRef) -> Result<Option<Event>, String> {
    let filter = nostr::Filter::new()
        .kind(nostr::Kind::Custom(KIND_SCOPED_DATA_SET))
        .author(grant.publisher)
        .custom_tags(
            SingleLetterTag::lowercase(Alphabet::D),
            [grant.scope_id.clone()],
        )
        .limit(8);

    let value = rest
        .query(&[filter])
        .await
        .map_err(|e| format!("relay query failed: {e}"))?;
    let arr = value
        .as_array()
        .ok_or_else(|| "relay query returned non-array".to_string())?;
    // Rollback defence (Security §7): prefer the newest authenticated event.
    Ok(arr
        .iter()
        .filter_map(|v| serde_json::from_value::<Event>(v.clone()).ok())
        .max_by_key(|e| e.created_at))
}

/// Dereference the data set from the grant's `a`-tag relay hints, used when the
/// connected Buzz relay does not hold it. Tries each hint in order (bounded by
/// [`MAX_RELAY_HINTS`]) and returns the first data set found. Anything returned
/// here still passes the full `decrypt_data_set` verification (signature,
/// `a`-tag pubkey match, `d` match) before it is applied, so a hostile hint
/// relay cannot inject a scope — it can at most fail to serve one.
async fn fetch_data_set_cross_relay(grant: &GrantRef) -> Option<Event> {
    for hint in grant.relay_hints.iter().take(MAX_RELAY_HINTS) {
        match fetch_data_set_from_relay(hint, grant).await {
            Ok(Some(ev)) => return Some(ev),
            Ok(None) => {}
            Err(reason) => {
                tracing::debug!(
                    target: "grant_sync",
                    relay = %hint,
                    "cross-relay data-set fetch failed: {reason}"
                );
            }
        }
    }
    None
}

/// Open a plain NIP-01 WebSocket to a single external relay, issue one `REQ`
/// for the `kind:30440` data set, and collect until EOSE or a bounded timeout.
/// Public relays serve reads without auth, so no NIP-42 handshake is needed.
async fn fetch_data_set_from_relay(url: &str, grant: &GrantRef) -> Result<Option<Event>, String> {
    if !(url.starts_with("ws://") || url.starts_with("wss://")) {
        return Err(format!("unsupported relay URL scheme: {url}"));
    }

    let filter = nostr::Filter::new()
        .kind(nostr::Kind::Custom(KIND_SCOPED_DATA_SET))
        .author(grant.publisher)
        .custom_tags(
            SingleLetterTag::lowercase(Alphabet::D),
            [grant.scope_id.clone()],
        )
        .limit(8);
    let filter_json =
        serde_json::to_value(&filter).map_err(|e| format!("filter serialize failed: {e}"))?;
    const SUB_ID: &str = "grant-dataset";
    let req = serde_json::json!(["REQ", SUB_ID, filter_json]).to_string();

    let exchange = async {
        let (mut ws, _resp) = connect_async(url)
            .await
            .map_err(|e| format!("connect failed: {e}"))?;
        ws.send(Message::Text(req.into()))
            .await
            .map_err(|e| format!("REQ send failed: {e}"))?;

        // Rollback defence (Security §7): keep the newest matching event.
        let mut newest: Option<Event> = None;
        while let Some(frame) = ws.next().await {
            let frame = frame.map_err(|e| format!("ws read failed: {e}"))?;
            match frame {
                Message::Text(text) => match parse_relay_message(&text) {
                    Ok(RelayMessage::Event {
                        subscription_id,
                        event,
                    }) if subscription_id == SUB_ID
                        && newest
                            .as_ref()
                            .is_none_or(|n| event.created_at > n.created_at) =>
                    {
                        newest = Some(*event);
                    }
                    Ok(RelayMessage::Eose { subscription_id }) if subscription_id == SUB_ID => break,
                    Ok(RelayMessage::Closed { .. }) => break,
                    _ => {}
                },
                Message::Close(_) => break,
                _ => {}
            }
        }
        let _ = ws.send(Message::Close(None)).await;
        Ok::<Option<Event>, String>(newest)
    };

    match tokio::time::timeout(CROSS_RELAY_TIMEOUT, exchange).await {
        Ok(res) => res,
        Err(_) => Err("cross-relay fetch timed out".into()),
    }
}

/// Current unix time in seconds.
fn now() -> u64 {
    nostr::Timestamp::now().as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Kind, Tag, Timestamp};

    const TS: u64 = 1_700_000_000;

    /// Build a gift-wrapped `kind:440` grant addressed to `agent`, sealed by
    /// `grantor`, exactly as a publisher would (NIP-59 + NIP-DA).
    fn wrap_grant(
        grantor: &Keys,
        agent_pk: &PublicKey,
        rumor_kind: u16,
        rumor_content: &str,
        rumor_tags: Vec<Tag>,
    ) -> Event {
        // Inner rumor — unsigned, authored by the grantor.
        let rumor: UnsignedEvent = EventBuilder::new(Kind::Custom(rumor_kind), rumor_content)
            .tags(rumor_tags)
            .custom_created_at(Timestamp::from(TS))
            .build(grantor.public_key());
        let rumor_json = serde_json::to_string(&rumor).unwrap();

        // Seal — kind:13 signed by the grantor, encrypting the rumor to agent.
        let sealed = nostr::nips::nip44::encrypt(
            grantor.secret_key(),
            agent_pk,
            &rumor_json,
            nostr::nips::nip44::Version::V2,
        )
        .unwrap();
        let seal: Event = EventBuilder::new(Kind::Custom(KIND_SEAL), sealed)
            .custom_created_at(Timestamp::from(TS))
            .sign_with_keys(grantor)
            .unwrap();
        let seal_json = serde_json::to_string(&seal).unwrap();

        // Gift wrap — kind:1059 signed by an ephemeral key, encrypting the seal
        // to agent, #p-tagged to agent.
        let ephemeral = Keys::generate();
        let wrapped = nostr::nips::nip44::encrypt(
            ephemeral.secret_key(),
            agent_pk,
            &seal_json,
            nostr::nips::nip44::Version::V2,
        )
        .unwrap();
        EventBuilder::new(Kind::Custom(KIND_GIFT_WRAP), wrapped)
            .tags(vec![Tag::parse(["p", &agent_pk.to_hex()]).unwrap()])
            .custom_created_at(Timestamp::from(TS))
            .sign_with_keys(&ephemeral)
            .unwrap()
    }

    /// A base64 32-byte scope key and its raw bytes.
    fn scope_key() -> ([u8; 32], String) {
        let raw = [7u8; 32];
        let b64 = base64::engine::general_purpose::STANDARD.encode(raw);
        (raw, b64)
    }

    fn grant_tags(publisher: &PublicKey, scope_id: &str, v: u64) -> Vec<Tag> {
        vec![
            Tag::parse(["a", &format!("30440:{}:{}", publisher.to_hex(), scope_id)]).unwrap(),
            Tag::parse(["v", &v.to_string()]).unwrap(),
        ]
    }

    /// Full NIP-59 round trip: a gift-wrapped grant unwraps back to the rumor,
    /// and the grantor is recovered from the seal.
    #[test]
    fn unwrap_recovers_rumor_and_grantor() {
        let agent = Keys::generate();
        let grantor = Keys::generate();
        let (_, key_b64) = scope_key();
        let content = format!("{{\"scope_key\":\"{key_b64}\",\"scope_name\":\"Personal\"}}");
        let tags = grant_tags(&grantor.public_key(), "abc123", 1);
        let wrap = wrap_grant(&grantor, &agent.public_key(), KIND_DATA_GRANT, &content, tags);

        let (rumor, recovered) = unwrap_gift_wrap(&wrap, &agent).unwrap();
        assert_eq!(rumor.kind.as_u16(), KIND_DATA_GRANT);
        assert_eq!(recovered, grantor.public_key());
        assert_eq!(rumor.pubkey, grantor.public_key());
    }

    /// A wrap addressed to a *different* agent must not decrypt for us.
    #[test]
    fn unwrap_for_other_recipient_fails() {
        let agent = Keys::generate();
        let eavesdropper = Keys::generate();
        let grantor = Keys::generate();
        let (_, key_b64) = scope_key();
        let content = format!("{{\"scope_key\":\"{key_b64}\"}}");
        let tags = grant_tags(&grantor.public_key(), "abc123", 1);
        let wrap = wrap_grant(&grantor, &agent.public_key(), KIND_DATA_GRANT, &content, tags);

        assert!(unwrap_gift_wrap(&wrap, &eavesdropper).is_err());
    }

    /// Parsing pulls publisher, scope id, generation and the 32-byte key.
    #[test]
    fn parse_grant_extracts_fields() {
        let publisher = Keys::generate();
        let (raw, key_b64) = scope_key();
        let content = format!("{{\"scope_key\":\"{key_b64}\",\"scope_name\":\"Personal\"}}");
        let rumor: UnsignedEvent = EventBuilder::new(Kind::Custom(KIND_DATA_GRANT), content)
            .tags(grant_tags(&publisher.public_key(), "scope-9", 3))
            .custom_created_at(Timestamp::from(TS))
            .build(publisher.public_key());

        let grant = parse_grant(&rumor).unwrap();
        assert_eq!(grant.publisher, publisher.public_key());
        assert_eq!(grant.scope_id, "scope-9");
        assert_eq!(grant.generation, Some(3));
        assert_eq!(grant.scope_key, raw);
        assert_eq!(grant.scope_name.as_deref(), Some("Personal"));
    }

    /// A grant whose scope_key isn't 32 bytes is rejected.
    #[test]
    fn parse_grant_rejects_short_key() {
        let publisher = Keys::generate();
        let short = base64::engine::general_purpose::STANDARD.encode([1u8; 16]);
        let content = format!("{{\"scope_key\":\"{short}\"}}");
        let rumor: UnsignedEvent = EventBuilder::new(Kind::Custom(KIND_DATA_GRANT), content)
            .tags(grant_tags(&publisher.public_key(), "s", 1))
            .custom_created_at(Timestamp::from(TS))
            .build(publisher.public_key());

        assert!(parse_grant(&rumor).is_err());
    }

    /// Build a `kind:30440` data set encrypted under `scope_key`, signed by
    /// `publisher`.
    fn build_data_set(publisher: &Keys, scope_id: &str, v: u64, key: [u8; 32], payload: &str) -> Event {
        let ck = nostr::nips::nip44::v2::ConversationKey::new(key);
        let ct = nostr::nips::nip44::v2::encrypt_to_bytes(&ck, payload.as_bytes()).unwrap();
        let content = base64::engine::general_purpose::STANDARD.encode(ct);
        EventBuilder::new(Kind::Custom(KIND_SCOPED_DATA_SET), content)
            .tags(vec![
                Tag::parse(["d", scope_id]).unwrap(),
                Tag::parse(["v", &v.to_string()]).unwrap(),
            ])
            .custom_created_at(Timestamp::from(TS))
            .sign_with_keys(publisher)
            .unwrap()
    }

    fn grant_ref(publisher: &PublicKey, scope_id: &str, v: u64, key: [u8; 32]) -> GrantRef {
        GrantRef {
            publisher: *publisher,
            scope_id: scope_id.to_string(),
            relay_hints: vec![],
            generation: Some(v),
            expiration: None,
            scope_key: key,
            scope_name: None,
        }
    }

    /// Happy path: a data set encrypted under the scope key decrypts back to
    /// the payload.
    #[test]
    fn decrypt_data_set_round_trip() {
        let publisher = Keys::generate();
        let (raw, _) = scope_key();
        let payload = "{\"name\":\"Personal\",\"fields\":{\"display_name\":\"James\"}}";
        let ds = build_data_set(&publisher, "scope-9", 3, raw, payload);
        let grant = grant_ref(&publisher.public_key(), "scope-9", 3, raw);

        assert_eq!(decrypt_data_set(&ds, &grant).unwrap(), payload);
    }

    /// A rotated (different) scope key no longer decrypts — the revocation
    /// signal.
    #[test]
    fn decrypt_data_set_rotated_key_fails() {
        let publisher = Keys::generate();
        let old = [7u8; 32];
        let rotated = [9u8; 32];
        let ds = build_data_set(&publisher, "scope-9", 4, rotated, "secret");
        // Grantee still holds the old key from generation 3.
        let grant = grant_ref(&publisher.public_key(), "scope-9", 3, old);

        assert!(decrypt_data_set(&ds, &grant).is_err());
    }

    /// Security §6: a data set signed by someone other than the `a`-tag
    /// publisher is refused before decryption.
    #[test]
    fn decrypt_data_set_wrong_publisher_fails() {
        let publisher = Keys::generate();
        let imposter = Keys::generate();
        let (raw, _) = scope_key();
        // Correct key + scope id, but signed by the imposter.
        let ds = build_data_set(&imposter, "scope-9", 3, raw, "poison");
        let grant = grant_ref(&publisher.public_key(), "scope-9", 3, raw);

        let err = decrypt_data_set(&ds, &grant).unwrap_err();
        assert!(err.contains("publisher"), "got: {err}");
    }

    /// A data set whose `d` tag doesn't match the grant's scope is refused.
    #[test]
    fn decrypt_data_set_scope_mismatch_fails() {
        let publisher = Keys::generate();
        let (raw, _) = scope_key();
        let ds = build_data_set(&publisher, "other-scope", 3, raw, "data");
        let grant = grant_ref(&publisher.public_key(), "scope-9", 3, raw);

        assert!(decrypt_data_set(&ds, &grant).is_err());
    }

    /// Slug is stable and single-segment under `mem/`.
    #[test]
    fn scope_slug_is_stable() {
        let publisher = Keys::generate();
        let grant = grant_ref(&publisher.public_key(), "scope-9", 1, [0u8; 32]);
        assert_eq!(scope_slug(&grant), "mem/grant-scope-9");
    }

    /// A non-WebSocket relay hint is rejected before any connection is opened.
    #[tokio::test]
    async fn cross_relay_rejects_non_ws_scheme() {
        let publisher = Keys::generate();
        let grant = grant_ref(&publisher.public_key(), "scope-9", 1, [0u8; 32]);
        let err = fetch_data_set_from_relay("https://example.com", &grant)
            .await
            .unwrap_err();
        assert!(err.contains("unsupported relay URL scheme"), "{err}");
    }

    /// With no relay hints, cross-relay dereference yields nothing (and never
    /// touches the network).
    #[tokio::test]
    async fn cross_relay_no_hints_is_none() {
        let publisher = Keys::generate();
        let grant = grant_ref(&publisher.public_key(), "scope-9", 1, [0u8; 32]);
        assert!(fetch_data_set_cross_relay(&grant).await.is_none());
    }

    /// A grant whose only hints are unusable is skipped fail-open — the errors
    /// are swallowed and the result is simply `None`.
    #[tokio::test]
    async fn cross_relay_all_bad_hints_is_none() {
        let publisher = Keys::generate();
        let mut grant = grant_ref(&publisher.public_key(), "scope-9", 1, [0u8; 32]);
        grant.relay_hints = vec!["https://not-a-relay".into(), "ftp://nope".into()];
        assert!(fetch_data_set_cross_relay(&grant).await.is_none());
    }

    /// End-to-end wire proof: stand up a real in-process WebSocket relay, let the
    /// cross-relay path connect to it via the grant's hint, issue the `REQ`,
    /// receive a genuine signed+encrypted `kind:30440` EVENT then EOSE, and
    /// decrypt the payload back out. This exercises the actual socket path the
    /// three tests above deliberately skip.
    #[tokio::test]
    async fn cross_relay_fetches_and_decrypts_over_real_socket() {
        use tokio::net::TcpListener;
        use tokio_tungstenite::accept_async;

        let publisher = Keys::generate();
        let (raw, _) = scope_key();
        let scope_id = "scope-live";
        let data_set = build_data_set(&publisher, scope_id, 5, raw, "the secret payload");
        let ds_json = serde_json::to_value(&data_set).unwrap();

        // Minimal NIP-01 relay: accept one connection, read the REQ to learn the
        // subscription id, reply with the matching EVENT then EOSE.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = accept_async(stream).await.unwrap();
            let sub_id = loop {
                match ws.next().await {
                    Some(Ok(Message::Text(t))) => {
                        let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                        break v[1].as_str().unwrap().to_string();
                    }
                    Some(Ok(_)) => continue,
                    _ => return,
                }
            };
            let event_msg = serde_json::json!(["EVENT", sub_id, ds_json]).to_string();
            let eose_msg = serde_json::json!(["EOSE", sub_id]).to_string();
            ws.send(Message::Text(event_msg.into())).await.unwrap();
            ws.send(Message::Text(eose_msg.into())).await.unwrap();
        });

        let mut grant = grant_ref(&publisher.public_key(), scope_id, 5, raw);
        grant.relay_hints = vec![format!("ws://{addr}")];

        let fetched = fetch_data_set_cross_relay(&grant)
            .await
            .expect("relay should have served the data set");
        let payload = decrypt_data_set(&fetched, &grant).unwrap();
        assert_eq!(payload, "the secret payload");
        server.await.unwrap();
    }
}
