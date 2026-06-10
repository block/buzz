//! NIP-AE Agent Engrams — pure crypto + parsing primitives.
//!
//! See `docs/nips/NIP-AE.md` for the spec. This module is I/O-free: it does
//! not talk to relays or filesystems. Callers wire it to a transport and a
//! key source.
//!
//! Shared by `sprout-cli` (`sprout mem …`) and `sprout-acp` (core injection
//! at session creation).

use hmac::digest::KeyInit;
use hmac::{Hmac, Mac};
use nostr::nips::nip44::{self, v2::ConversationKey, Version};
use nostr::{Event, EventBuilder, Keys, Kind, PublicKey, SecretKey, Tag};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::kind::KIND_AGENT_ENGRAM;

/// The reserved slug for the agent's core (identity) engram.
pub const CORE_SLUG: &str = "core";

/// Domain prefix for the `d`-tag HMAC. Followed by a `0x00` byte and the slug.
/// Versioned independently of the NIP number; future revisions MUST change it.
pub const D_TAG_DOMAIN: &[u8] = b"agent-memory/v1/d-tag";

/// NIP-44 plaintext limit (bytes). Bodies whose serialized JSON exceeds this
/// MUST NOT be encrypted (spec: *Encryption*).
pub const NIP44_PLAINTEXT_MAX: usize = 65_535;

/// Maximum slug length in bytes (spec: *Slugs*).
pub const SLUG_MAX_LEN: usize = 255;

/// Memory slug grammar prefix.
const MEM_PREFIX: &str = "mem/";

/// Errors from engram operations.
#[derive(Debug, thiserror::Error)]
pub enum EngramError {
    /// Slug failed the *Slugs* grammar.
    #[error("invalid slug: {0}")]
    InvalidSlug(String),
    /// Body parsing or shape check failed.
    #[error("invalid body: {0}")]
    InvalidBody(String),
    /// Event failed *Head selection* rule (1) — tag shape or addressing.
    #[error("invalid envelope: {0}")]
    InvalidEnvelope(String),
    /// NIP-44 decryption failed.
    #[error("decrypt failed")]
    Decrypt,
    /// Encryption failed.
    #[error("encrypt failed: {0}")]
    Encrypt(String),
    /// Body exceeds the NIP-44 plaintext cap.
    #[error("body exceeds {NIP44_PLAINTEXT_MAX}-byte plaintext limit ({0} bytes)")]
    BodyTooLarge(usize),
    /// Signing error.
    #[error("sign failed: {0}")]
    Sign(String),
}

/// Validate a slug against the *Slugs* grammar.
///
/// Returns `Ok(())` if the slug is the reserved `core` or matches:
/// `^mem/[a-z0-9][a-z0-9_-]{0,63}(/[a-z0-9][a-z0-9_-]{0,63})*$` with total
/// length ≤ 255 bytes.
pub fn validate_slug(slug: &str) -> Result<(), EngramError> {
    if slug == CORE_SLUG {
        return Ok(());
    }
    if slug.len() > SLUG_MAX_LEN {
        return Err(EngramError::InvalidSlug(format!(
            "length {} exceeds {}",
            slug.len(),
            SLUG_MAX_LEN
        )));
    }
    let Some(rest) = slug.strip_prefix(MEM_PREFIX) else {
        return Err(EngramError::InvalidSlug(format!(
            "expected `core` or `mem/…`, got {slug:?}"
        )));
    };
    if rest.is_empty() {
        return Err(EngramError::InvalidSlug("empty after `mem/`".into()));
    }
    for (i, segment) in rest.split('/').enumerate() {
        validate_segment(segment).map_err(|why| {
            EngramError::InvalidSlug(format!("segment {} ({:?}): {why}", i + 1, segment))
        })?;
    }
    Ok(())
}

fn validate_segment(s: &str) -> Result<(), &'static str> {
    if s.is_empty() {
        return Err("empty");
    }
    if s.len() > 64 {
        return Err("longer than 64 bytes");
    }
    let bytes = s.as_bytes();
    let first = bytes[0];
    if !is_lower_alnum(first) {
        return Err("first byte must be [a-z0-9]");
    }
    for &b in &bytes[1..] {
        if !(is_lower_alnum(b) || b == b'_' || b == b'-') {
            return Err("only [a-z0-9_-] allowed after the first byte");
        }
    }
    Ok(())
}

const fn is_lower_alnum(b: u8) -> bool {
    matches!(b, b'a'..=b'z' | b'0'..=b'9')
}

/// Normalize a user-supplied shorthand to a full slug.
///
/// `core` is returned verbatim. Otherwise, if the input does not already
/// start with `mem/`, the prefix is added. The result is validated; the
/// caller still gets an `InvalidSlug` if the shorthand is not a valid slug.
pub fn normalize_slug(raw: &str) -> Result<String, EngramError> {
    let slug = if raw == CORE_SLUG || raw.starts_with(MEM_PREFIX) {
        raw.to_string()
    } else {
        format!("{MEM_PREFIX}{raw}")
    };
    validate_slug(&slug)?;
    Ok(slug)
}

/// Derive the conversation key `K_c` for the agent ↔ owner pair (NIP-44 v2).
///
/// `K_c` is symmetric: `derive(seckey_a, pubkey_o) == derive(seckey_o, pubkey_a)`.
pub fn conversation_key(my_seckey: &SecretKey, their_pubkey: &PublicKey) -> ConversationKey {
    ConversationKey::derive(my_seckey, their_pubkey).expect("valid keys produce conversation key")
}

/// Compute the `d` tag for a slug under a conversation key.
///
/// `d = lower_hex(HMAC-SHA256(K_c, "agent-memory/v1/d-tag" || 0x00 || slug))`,
/// 64 hex characters.
pub fn d_tag(k_c: &ConversationKey, slug: &str) -> String {
    // HMAC-SHA256 accepts a key of any byte length; `new_from_slice` only
    // returns `Err` for fixed-length MAC variants. This is infallible for
    // SHA-256 and propagating it would just add noise at every call site.
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(k_c.as_bytes())
        .expect("HMAC-SHA256 is keyed-prefix MAC; new_from_slice cannot fail");
    mac.update(D_TAG_DOMAIN);
    mac.update(&[0u8]);
    mac.update(slug.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

// ── Bodies ──────────────────────────────────────────────────────────────────

/// A decoded engram body. The slug discriminates the variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// `slug == "core"` — agent identity surface.
    Core {
        /// Free-form UTF-8 maintained by the agent.
        profile: String,
    },
    /// `slug == "mem/…"` — one logical entry. `value = None` is a tombstone.
    Memory {
        /// The slug this body addresses.
        slug: String,
        /// The entry's value, or `None` for a tombstone.
        value: Option<String>,
    },
}

impl Body {
    /// Return the slug this body addresses.
    pub fn slug(&self) -> &str {
        match self {
            Body::Core { .. } => CORE_SLUG,
            Body::Memory { slug, .. } => slug,
        }
    }

    /// `true` if this is a tombstone (memory with `value = null`).
    pub fn is_tombstone(&self) -> bool {
        matches!(self, Body::Memory { value: None, .. })
    }

    /// Serialize to the exact JSON encoding the spec specifies for the body
    /// passed to NIP-44. Whitespace-free, slug first.
    pub fn to_json_bytes(&self) -> Vec<u8> {
        // We hand-roll the JSON so the *Reference test vectors* match
        // byte-for-byte (`{"slug":"…","value":"…"}` / `{"slug":"core","profile":"…"}`).
        let mut out = String::with_capacity(64);
        out.push('{');
        out.push_str("\"slug\":");
        write_json_string(&mut out, self.slug());
        match self {
            Body::Memory { value: Some(v), .. } => {
                out.push_str(",\"value\":");
                write_json_string(&mut out, v);
            }
            Body::Memory { value: None, .. } => {
                out.push_str(",\"value\":null");
            }
            Body::Core { profile } => {
                out.push_str(",\"profile\":");
                write_json_string(&mut out, profile);
            }
        }
        out.push('}');
        out.into_bytes()
    }

    /// Parse a body from its decrypted JSON bytes. Rejects duplicate object
    /// member names (spec: *Head selection* rule (3)). Unknown fields are
    /// ignored. The body's `slug` is validated.
    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, EngramError> {
        // serde_json by default *accepts* duplicate keys (last wins). The spec
        // requires rejection, so we deserialize through a wrapper that walks
        // the tree once and fails on the first repeated member name at any
        // nesting depth.
        let raw = parse_strict_json(bytes)?;

        let obj = match raw {
            serde_json::Value::Object(m) => m,
            _ => return Err(EngramError::InvalidBody("top-level not an object".into())),
        };
        let slug = match obj.get("slug") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(_) => return Err(EngramError::InvalidBody("`slug` is not a string".into())),
            None => return Err(EngramError::InvalidBody("missing `slug`".into())),
        };
        validate_slug(&slug)?;
        if slug == CORE_SLUG {
            let profile = match obj.get("profile") {
                Some(serde_json::Value::String(s)) => s.clone(),
                Some(_) => {
                    return Err(EngramError::InvalidBody("`profile` is not a string".into()))
                }
                None => return Err(EngramError::InvalidBody("core missing `profile`".into())),
            };
            Ok(Body::Core { profile })
        } else {
            let value = match obj.get("value") {
                Some(serde_json::Value::String(s)) => Some(s.clone()),
                Some(serde_json::Value::Null) => None,
                Some(_) => return Err(EngramError::InvalidBody("`value` is not a string".into())),
                None => return Err(EngramError::InvalidBody("memory missing `value`".into())),
            };
            Ok(Body::Memory { slug, value })
        }
    }
}

/// Minimal JSON-string encoder per RFC 8259 §7. Escapes `"` `\` and the
/// required control chars (`\b \f \n \r \t` + `\u00XX` for the rest).
fn write_json_string(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Parse a JSON document, rejecting duplicate member names at any nesting
/// depth. Returns the parsed `serde_json::Value` on success.
///
/// `serde_json::from_slice` happily last-wins on duplicates, but *Head
/// selection* rule (3) requires strict rejection. We get correct behaviour
/// by feeding the deserializer a custom visitor for maps that tracks seen
/// keys; arrays / scalars fall through to the default `Value` visitor.
fn parse_strict_json(bytes: &[u8]) -> Result<serde_json::Value, EngramError> {
    use serde::de::{DeserializeSeed, Deserializer, MapAccess, SeqAccess, Visitor};
    use serde_json::Value;
    use std::collections::HashSet;
    use std::fmt;

    struct StrictValue;

    impl<'de> DeserializeSeed<'de> for StrictValue {
        type Value = Value;
        fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
            d.deserialize_any(StrictValue)
        }
    }

    impl<'de> Visitor<'de> for StrictValue {
        type Value = Value;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("any valid JSON value (objects must have unique keys)")
        }

        fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
            Ok(Value::Bool(v))
        }
        fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
            Ok(Value::Number(v.into()))
        }
        fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
            Ok(Value::Number(v.into()))
        }
        fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
            serde_json::Number::from_f64(v)
                .map(Value::Number)
                .ok_or_else(|| E::custom("non-finite float"))
        }
        fn visit_str<E>(self, v: &str) -> Result<Value, E> {
            Ok(Value::String(v.to_owned()))
        }
        fn visit_string<E>(self, v: String) -> Result<Value, E> {
            Ok(Value::String(v))
        }
        fn visit_unit<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_none<E>(self) -> Result<Value, E> {
            Ok(Value::Null)
        }
        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
            d.deserialize_any(StrictValue)
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
            let mut out = Vec::with_capacity(seq.size_hint().unwrap_or(0));
            while let Some(v) = seq.next_element_seed(StrictValue)? {
                out.push(v);
            }
            Ok(Value::Array(out))
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
            use serde::de::Error;
            let mut seen: HashSet<String> = HashSet::new();
            let mut out = serde_json::Map::new();
            while let Some(k) = map.next_key::<String>()? {
                if !seen.insert(k.clone()) {
                    return Err(A::Error::custom(format!(
                        "duplicate object member name: {k}"
                    )));
                }
                let v = map.next_value_seed(StrictValue)?;
                out.insert(k, v);
            }
            Ok(Value::Object(out))
        }
    }

    let mut de = serde_json::Deserializer::from_slice(bytes);
    let v = StrictValue
        .deserialize(&mut de)
        .map_err(|e| EngramError::InvalidBody(format!("invalid JSON body: {e}")))?;
    de.end()
        .map_err(|e| EngramError::InvalidBody(format!("trailing data after JSON body: {e}")))?;
    Ok(v)
}

// ── Reference extraction (`[[slug]]`) ───────────────────────────────────────

/// Extract `[[slug]]` references from a body's free-form text field
/// (`profile` for [`Body::Core`], `value` for [`Body::Memory`]).
///
/// Per *NIP-AE: References*, references are literal substrings of the form
/// `[[<slug>]]` where `<slug>` matches the *Slugs* grammar. Bare slug-shaped
/// strings without brackets are NOT references. The spec defines no escaping
/// mechanism and no markup-aware exclusion, so this scan is purely textual.
///
/// Returns the slugs in **first-occurrence order**, deduplicated. Candidates
/// that fail [`validate_slug`] are silently dropped: callers building a
/// reachability graph want only well-formed targets, and an empty `[[]]` or
/// a `[[bogus slug!]]` is treated the same as ordinary text.
///
/// This function performs no I/O and no allocation beyond the returned
/// `Vec<String>`.
pub fn extract_refs(body: &str) -> Vec<String> {
    let bytes = body.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i + 3 < bytes.len() {
        // Need at least `[[x]]` — 5 bytes — to contain a non-empty payload.
        if bytes[i] == b'[' && bytes[i + 1] == b'[' {
            let start = i + 2;
            // Find the next `]]` after `start`. We do not allow nesting:
            // the first `]]` closes the reference. If we hit another `[[`
            // before `]]`, restart the scan from that inner `[[` so e.g.
            // `[[outer [[mem/x]]` still surfaces `mem/x`.
            let mut j = start;
            let mut closed = false;
            while j + 1 < bytes.len() {
                if bytes[j] == b'[' && bytes[j + 1] == b'[' {
                    // Inner `[[` — abandon the outer match, restart there.
                    break;
                }
                if bytes[j] == b']' && bytes[j + 1] == b']' {
                    closed = true;
                    break;
                }
                j += 1;
            }
            if closed {
                // SAFETY: `start` and `j` both sit on ASCII bracket
                // boundaries; the slice between them is UTF-8 because the
                // input is.
                let candidate = &body[start..j];
                if validate_slug(candidate).is_ok() && !out.iter().any(|s| s == candidate) {
                    out.push(candidate.to_string());
                }
                i = j + 2;
                continue;
            }
            // Either we hit an inner `[[` or ran off the end without a
            // closing `]]`. Advance past the opening `[[` and keep looking.
            i = start;
            continue;
        }
        i += 1;
    }
    out
}

// ── Envelope build / parse ──────────────────────────────────────────────────

/// Build a signed `kind:30174` event for a given body.
///
/// * `created_at` is the timestamp to sign — callers MUST supply a value
///   respecting the *Writing* monotonic rule (`max(now, T_head + 1)`).
/// * Returns `BodyTooLarge` if the serialized body exceeds 65,535 bytes.
pub fn build_event(
    agent_keys: &Keys,
    owner_pubkey: &PublicKey,
    body: &Body,
    created_at: u64,
) -> Result<Event, EngramError> {
    let plaintext = body.to_json_bytes();
    if plaintext.len() > NIP44_PLAINTEXT_MAX {
        return Err(EngramError::BodyTooLarge(plaintext.len()));
    }
    // `to_json_bytes` only emits ASCII control chars or `&str` bytes, so
    // this is always Ok. We still verify rather than `.expect()` so a future
    // change to the serializer can't silently introduce a panic on the hot
    // path.
    let plaintext_str = std::str::from_utf8(&plaintext)
        .map_err(|e| EngramError::Encrypt(format!("body JSON not UTF-8: {e}")))?;

    let k_c = conversation_key(agent_keys.secret_key(), owner_pubkey);
    let ciphertext = nip44::encrypt(
        agent_keys.secret_key(),
        owner_pubkey,
        plaintext_str,
        Version::V2,
    )
    .map_err(|e| EngramError::Encrypt(e.to_string()))?;

    let d = d_tag(&k_c, body.slug());
    let tags = vec![
        Tag::parse(["d", &d]).map_err(|e| EngramError::Encrypt(e.to_string()))?,
        Tag::parse(["p", &owner_pubkey.to_hex()])
            .map_err(|e| EngramError::Encrypt(e.to_string()))?,
    ];

    EventBuilder::new(Kind::Custom(KIND_AGENT_ENGRAM as u16), ciphertext)
        .tags(tags)
        .custom_created_at(nostr::Timestamp::from(created_at))
        .sign_with_keys(agent_keys)
        .map_err(|e| EngramError::Sign(e.to_string()))
}

/// Validate an event against *Head selection* rules (1) and (5) and return
/// the decoded body. Caller must verify the signature beforehand (NIP-44
/// requires outer-signature-before-decrypt; `nostr::Event::verify_signature`
/// or NIP-01 ingest path handles this).
///
/// * `event` — the candidate event.
/// * `expected_agent` — the agent pubkey the event's `pubkey` field must equal.
/// * `expected_owner` — the owner pubkey the event's `p` tag must contain.
/// * `my_seckey` / `their_pubkey` — the *NIP-44 ECDH pair* the caller holds.
///   For an agent reading its own engrams: `my_seckey = seckey_a`,
///   `their_pubkey = pubkey_o`. For an owner reading the agent's engrams:
///   `my_seckey = seckey_o`, `their_pubkey = pubkey_a`. Either yields the
///   same `K_c`.
pub fn validate_and_decrypt(
    event: &Event,
    expected_agent: &PublicKey,
    expected_owner: &PublicKey,
    my_seckey: &SecretKey,
    their_pubkey: &PublicKey,
) -> Result<Body, EngramError> {
    if event.kind.as_u16() as u32 != KIND_AGENT_ENGRAM {
        return Err(EngramError::InvalidEnvelope(format!(
            "wrong kind: {}",
            event.kind.as_u16()
        )));
    }
    if &event.pubkey != expected_agent {
        return Err(EngramError::InvalidEnvelope(
            "pubkey != expected_agent".into(),
        ));
    }
    let mut d_tags = event.tags.iter().filter(|t| t.kind().to_string() == "d");
    let d_value = d_tags
        .next()
        .and_then(|t| t.content().map(|s| s.to_string()))
        .ok_or_else(|| EngramError::InvalidEnvelope("missing d tag".into()))?;
    if d_tags.next().is_some() {
        return Err(EngramError::InvalidEnvelope("multiple d tags".into()));
    }
    // Spec: d = lower_hex(HMAC...). Anything else is non-canonical and we
    // refuse to interoperate with it.
    if d_value.len() != 64
        || !d_value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(EngramError::InvalidEnvelope(
            "d tag must be 64 lowercase hex chars".into(),
        ));
    }
    let mut p_tags = event.tags.iter().filter(|t| t.kind().to_string() == "p");
    let p_value = p_tags
        .next()
        .and_then(|t| t.content().map(|s| s.to_string()))
        .ok_or_else(|| EngramError::InvalidEnvelope("missing p tag".into()))?;
    if p_tags.next().is_some() {
        return Err(EngramError::InvalidEnvelope("multiple p tags".into()));
    }
    if p_value.eq_ignore_ascii_case(&expected_owner.to_hex()) {
        // ok
    } else {
        return Err(EngramError::InvalidEnvelope(
            "p tag != expected_owner".into(),
        ));
    }

    // Decrypt. `K_c` is symmetric per NIP-44, so the caller's `(my_seckey,
    // their_pubkey)` pair yields the same conversation key regardless of
    // whether the caller is the agent or the owner.
    let plaintext = nip44::decrypt(my_seckey, their_pubkey, &event.content)
        .map_err(|_| EngramError::Decrypt)?;
    let body = Body::from_json_bytes(plaintext.as_bytes())?;

    // Rule (4): body slug re-derives to the event's d tag.
    let k_c = conversation_key(my_seckey, their_pubkey);
    let derived = d_tag(&k_c, body.slug());
    if derived != d_value {
        return Err(EngramError::InvalidEnvelope(
            "body slug does not re-derive to d tag".into(),
        ));
    }
    Ok(body)
}

/// Pick the head from a set of events targeting the same slug — greatest
/// `created_at`, ties broken by lowest event id per NIP-01.
///
/// Caller is responsible for validating each event (kind/tags/sig/decrypt)
/// before passing it in; this function only does the LWW tiebreak.
pub fn select_head<I>(events: I) -> Option<Event>
where
    I: IntoIterator<Item = Event>,
{
    events.into_iter().reduce(|a, b| {
        let a_ts = a.created_at.as_secs();
        let b_ts = b.created_at.as_secs();
        if b_ts > a_ts {
            return b;
        }
        if a_ts > b_ts {
            return a;
        }
        // Tie — lower id wins (lexicographic on hex == byte-wise on bytes).
        if b.id.to_hex() < a.id.to_hex() {
            b
        } else {
            a
        }
    })
}

/// Compute `created_at` for a new write per the *Writing* monotonic rule:
/// `max(now, prior_head_created_at + 1)`.
pub fn monotonic_created_at(now: u64, prior_head: Option<u64>) -> u64 {
    match prior_head {
        Some(t) => now.max(t.saturating_add(1)),
        None => now,
    }
}

/// Wire representation for `sprout mem ls`: one entry per non-tombstone
/// memory slug (`core` is excluded by the listing procedure).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Listing {
    /// The slug (e.g. `mem/notes/today`).
    pub slug: String,
    /// Event id of the current head.
    pub event_id: String,
    /// `created_at` of the current head.
    pub created_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys_from_hex(s: &str) -> Keys {
        Keys::parse(s).unwrap()
    }

    // ── Reference test vectors from docs/nips/NIP-AE.md §"Reference test
    //    vectors". Pinning these as CI invariants is the single best
    //    interop guarantee for this implementation. ──

    const SECKEY_A: &str = "0000000000000000000000000000000000000000000000000000000000000001";
    const SECKEY_O: &str = "0000000000000000000000000000000000000000000000000000000000000002";

    const PUBKEY_A: &str = "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798";
    const PUBKEY_O: &str = "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5";

    const K_C_HEX: &str = "c41c775356fd92eadc63ff5a0dc1da211b268cbea22316767095b2871ea1412d";

    const D_CORE: &str = "bdc233238ffe52e272b44cc233c8f33a2bc510b08be04495b225964283be4a90";
    const D_EXAMPLE: &str = "72d4f9629106451505d7d341ea85bb3ebad4f654fcfd2aad100d5a35f8a85cba";
    const D_NOTES: &str = "31651571a312780cfdc1f0b706b682ac9f3f51a053e8dca76fe57710bae5a4d4";

    #[test]
    fn pubkeys_match_spec() {
        assert_eq!(keys_from_hex(SECKEY_A).public_key().to_hex(), PUBKEY_A);
        assert_eq!(keys_from_hex(SECKEY_O).public_key().to_hex(), PUBKEY_O);
    }

    #[test]
    fn conversation_key_matches_spec() {
        let a = keys_from_hex(SECKEY_A);
        let o = keys_from_hex(SECKEY_O);
        let k_c_ao = conversation_key(a.secret_key(), &o.public_key());
        let k_c_oa = conversation_key(o.secret_key(), &a.public_key());
        assert_eq!(hex::encode(k_c_ao.as_bytes()), K_C_HEX, "agent-side K_c");
        assert_eq!(hex::encode(k_c_oa.as_bytes()), K_C_HEX, "owner-side K_c");
    }

    #[test]
    fn d_tags_match_spec() {
        let a = keys_from_hex(SECKEY_A);
        let o = keys_from_hex(SECKEY_O);
        let k_c = conversation_key(a.secret_key(), &o.public_key());
        assert_eq!(d_tag(&k_c, "core"), D_CORE);
        assert_eq!(d_tag(&k_c, "mem/example"), D_EXAMPLE);
        assert_eq!(d_tag(&k_c, "mem/notes/2026-05-12"), D_NOTES);
    }

    #[test]
    fn body_round_trips_byte_exact() {
        // Memory with value.
        let b = Body::Memory {
            slug: "mem/example".into(),
            value: Some("hello, agent memory".into()),
        };
        assert_eq!(
            b.to_json_bytes(),
            br#"{"slug":"mem/example","value":"hello, agent memory"}"#.to_vec()
        );
        // Memory with reference.
        let b = Body::Memory {
            slug: "mem/notes/2026-05-12".into(),
            value: Some("meeting note: [[mem/example]]".into()),
        };
        assert_eq!(
            b.to_json_bytes(),
            br#"{"slug":"mem/notes/2026-05-12","value":"meeting note: [[mem/example]]"}"#.to_vec()
        );
        // Tombstone.
        let b = Body::Memory {
            slug: "mem/example".into(),
            value: None,
        };
        assert_eq!(
            b.to_json_bytes(),
            br#"{"slug":"mem/example","value":null}"#.to_vec()
        );
        // Core.
        let b = Body::Core {
            profile: "test agent. see [[mem/example]] and [[mem/notes/2026-05-12]].".into(),
        };
        assert_eq!(
            b.to_json_bytes(),
            br#"{"slug":"core","profile":"test agent. see [[mem/example]] and [[mem/notes/2026-05-12]]."}"#.to_vec()
        );
    }

    #[test]
    fn body_parse_rejects_duplicate_keys() {
        let dup = br#"{"slug":"mem/x","slug":"mem/y","value":"v"}"#;
        let err = Body::from_json_bytes(dup).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("duplicate"), "got: {msg}");
    }

    #[test]
    fn body_parse_ignores_unknown_fields() {
        let body = br#"{"slug":"mem/x","value":"v","unknown":42,"future":{"nested":"ok"}}"#;
        let b = Body::from_json_bytes(body).unwrap();
        assert!(matches!(b, Body::Memory { value: Some(_), .. }));
    }

    #[test]
    fn body_parse_accepts_arrays_of_repeated_strings() {
        // Repeats inside an *array* aren't object-member duplicates and must
        // be accepted. The previous hand-rolled scanner incorrectly flagged
        // this as a dup.
        let body = br#"{"slug":"mem/x","value":"v","future":["slug","slug","value"]}"#;
        let b = Body::from_json_bytes(body).unwrap();
        assert!(matches!(b, Body::Memory { value: Some(_), .. }));
    }

    #[test]
    fn body_parse_accepts_surrogate_pair_escapes() {
        // `\uD83D\uDE00` (😀) is a valid UTF-16 surrogate pair that serde_json
        // decodes correctly. The previous hand-rolled scanner rejected each
        // half as an invalid codepoint.
        let body = br#"{"slug":"mem/x","value":"hi \uD83D\uDE00"}"#;
        let b = Body::from_json_bytes(body).unwrap();
        match b {
            Body::Memory {
                value: Some(v),
                slug,
            } => {
                assert_eq!(slug, "mem/x");
                assert_eq!(v, "hi \u{1F600}");
            }
            _ => panic!("expected Memory"),
        }
    }

    #[test]
    fn body_parse_rejects_duplicates_at_any_depth() {
        // Object inside an unknown field with duplicate keys: still rejected.
        let dup = br#"{"slug":"mem/x","value":"v","nested":{"k":1,"k":2}}"#;
        let err = Body::from_json_bytes(dup).unwrap_err();
        assert!(format!("{err}").contains("duplicate"), "got: {err}");
    }

    #[test]
    fn validate_slug_accepts_grammar() {
        for ok in [
            "core",
            "mem/x",
            "mem/x-y_z",
            "mem/0",
            "mem/notes/2026-05-12",
            "mem/a/b/c",
        ] {
            assert!(validate_slug(ok).is_ok(), "{ok} should be valid");
        }
    }

    #[test]
    fn validate_slug_rejects_garbage() {
        for bad in [
            "", "MEM/x", "mem/", "mem//x", "mem/-x", "mem/_x", "mem/x/-y", "mem/x/", "memx",
            "mem/x/Y",
            "mem/x.y",
            // 64-byte boundary: first byte then 64 more = 65 -> too long
        ] {
            assert!(validate_slug(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn normalize_slug_adds_mem_prefix() {
        assert_eq!(normalize_slug("foo").unwrap(), "mem/foo");
        assert_eq!(normalize_slug("core").unwrap(), "core");
        assert_eq!(normalize_slug("mem/bar").unwrap(), "mem/bar");
        assert!(normalize_slug("Foo").is_err());
    }

    #[test]
    fn monotonic_clock_rule() {
        assert_eq!(monotonic_created_at(100, None), 100);
        assert_eq!(monotonic_created_at(100, Some(50)), 100);
        assert_eq!(monotonic_created_at(100, Some(100)), 101);
        assert_eq!(monotonic_created_at(100, Some(200)), 201);
    }

    #[test]
    fn select_head_lww_with_id_tiebreak() {
        // We build three events with the same kind/pubkey/d tag and check
        // (1) greater created_at wins, (2) ties broken by lower id.
        let agent = keys_from_hex(SECKEY_A);
        let owner = keys_from_hex(SECKEY_O);
        let body = Body::Memory {
            slug: "mem/example".into(),
            value: Some("v".into()),
        };
        let e1 = build_event(&agent, &owner.public_key(), &body, 1_700_000_000).unwrap();
        let e2 = build_event(&agent, &owner.public_key(), &body, 1_700_000_001).unwrap();
        let pick = select_head([e1.clone(), e2.clone()]).unwrap();
        assert_eq!(pick.id, e2.id);
    }

    #[test]
    fn round_trip_event_validates_and_decrypts() {
        let agent = keys_from_hex(SECKEY_A);
        let owner = keys_from_hex(SECKEY_O);
        let original = Body::Memory {
            slug: "mem/example".into(),
            value: Some("hello".into()),
        };
        let event = build_event(&agent, &owner.public_key(), &original, 1_700_000_000).unwrap();
        let decoded = validate_and_decrypt(
            &event,
            &agent.public_key(),
            &owner.public_key(),
            owner.secret_key(),
            &agent.public_key(),
        )
        .unwrap();
        // And the agent-side decrypt path must also work.
        let decoded_agent = validate_and_decrypt(
            &event,
            &agent.public_key(),
            &owner.public_key(),
            agent.secret_key(),
            &owner.public_key(),
        )
        .unwrap();
        assert_eq!(decoded_agent, decoded);
        assert_eq!(decoded, original);
    }

    #[test]
    fn validate_and_decrypt_rejects_non_lowercase_d_tag() {
        // Build a perfectly valid event but tamper the d tag to uppercase
        // before signing. `validate_and_decrypt` must refuse, because the
        // spec requires lowercase hex and otherwise two stringly-different
        // d-tags would map to the same head.
        let agent = keys_from_hex(SECKEY_A);
        let owner = keys_from_hex(SECKEY_O);
        let body = Body::Memory {
            slug: "mem/example".into(),
            value: Some("hi".into()),
        };
        let ev = build_event(&agent, &owner.public_key(), &body, 1).unwrap();
        // Re-sign with the d tag uppercased.
        let d = ev
            .tags
            .iter()
            .find(|t| t.kind().to_string() == "d")
            .and_then(|t| t.content().map(|s| s.to_string()))
            .unwrap();
        let upper_d = d.to_uppercase();
        let tags = vec![
            Tag::parse(["d", &upper_d]).unwrap(),
            Tag::parse(["p", &owner.public_key().to_hex()]).unwrap(),
        ];
        let tampered =
            EventBuilder::new(Kind::Custom(KIND_AGENT_ENGRAM as u16), ev.content.clone())
                .tags(tags)
                .custom_created_at(ev.created_at)
                .sign_with_keys(&agent)
                .unwrap();
        let err = validate_and_decrypt(
            &tampered,
            &agent.public_key(),
            &owner.public_key(),
            agent.secret_key(),
            &owner.public_key(),
        )
        .unwrap_err();
        assert!(matches!(err, EngramError::InvalidEnvelope(_)));
    }

    #[test]
    fn body_too_large_rejected_at_build_time() {
        let agent = keys_from_hex(SECKEY_A);
        let owner = keys_from_hex(SECKEY_O);
        // Build a value whose JSON representation just barely exceeds the limit.
        let huge = "a".repeat(NIP44_PLAINTEXT_MAX);
        let body = Body::Memory {
            slug: "mem/example".into(),
            value: Some(huge),
        };
        let err = build_event(&agent, &owner.public_key(), &body, 1).unwrap_err();
        assert!(matches!(err, EngramError::BodyTooLarge(_)));
    }

    // ── extract_refs ────────────────────────────────────────────────────────

    #[test]
    fn extract_refs_empty_body() {
        assert!(extract_refs("").is_empty());
    }

    #[test]
    fn extract_refs_no_refs() {
        assert!(extract_refs("plain prose with no brackets").is_empty());
        assert!(extract_refs("single [bracket] only").is_empty());
        assert!(extract_refs("mem/example without brackets").is_empty());
    }

    #[test]
    fn extract_refs_basic_memory() {
        assert_eq!(
            extract_refs("see [[mem/example]] for context"),
            vec!["mem/example".to_string()]
        );
    }

    #[test]
    fn extract_refs_basic_core() {
        assert_eq!(
            extract_refs("rooted at [[core]] yo"),
            vec!["core".to_string()]
        );
    }

    #[test]
    fn extract_refs_multiple_in_order() {
        assert_eq!(
            extract_refs("[[mem/a]] then [[mem/b]] then [[mem/c]]"),
            vec![
                "mem/a".to_string(),
                "mem/b".to_string(),
                "mem/c".to_string(),
            ]
        );
    }

    #[test]
    fn extract_refs_dedupes_preserving_first_occurrence() {
        assert_eq!(
            extract_refs("[[mem/a]] [[mem/b]] [[mem/a]] [[mem/c]] [[mem/b]]"),
            vec![
                "mem/a".to_string(),
                "mem/b".to_string(),
                "mem/c".to_string(),
            ]
        );
    }

    #[test]
    fn extract_refs_nested_segments() {
        assert_eq!(
            extract_refs("see [[mem/notes/2026-05-12]]"),
            vec!["mem/notes/2026-05-12".to_string()]
        );
    }

    #[test]
    fn extract_refs_spec_fixture_core_profile() {
        // Body 4 from the NIP-AE reference vectors.
        assert_eq!(
            extract_refs("test agent. see [[mem/example]] and [[mem/notes/2026-05-12]]."),
            vec![
                "mem/example".to_string(),
                "mem/notes/2026-05-12".to_string(),
            ]
        );
    }

    #[test]
    fn extract_refs_drops_empty_brackets() {
        assert!(extract_refs("[[]]").is_empty());
        assert!(extract_refs("ends with [[]] yo").is_empty());
    }

    #[test]
    fn extract_refs_drops_invalid_slugs() {
        // Uppercase, spaces, leading dash, missing `mem/` — all rejected by validate_slug.
        assert!(extract_refs("[[Mem/Example]]").is_empty());
        assert!(extract_refs("[[mem/with spaces]]").is_empty());
        assert!(extract_refs("[[mem/-leading-dash]]").is_empty());
        assert!(extract_refs("[[example]]").is_empty());
        assert!(extract_refs("[[mem/]]").is_empty());
    }

    #[test]
    fn extract_refs_unclosed_brackets() {
        assert!(extract_refs("[[mem/x with no closing").is_empty());
        assert_eq!(
            extract_refs("[[mem/x with no close, but [[mem/y]] is fine"),
            vec!["mem/y".to_string()]
        );
    }

    #[test]
    fn extract_refs_single_brackets_dont_match() {
        assert!(extract_refs("[mem/x]").is_empty());
        assert!(extract_refs("[mem/x] and [mem/y]").is_empty());
    }

    #[test]
    fn extract_refs_triple_brackets_match_inner() {
        // `[[[mem/x]]]` — the outer `[[` at position 0 opens; the inner
        // scan walks through `[mem/x` and finds `]]` at positions 8-9, so
        // the candidate is `[mem/x` (with the leading `[`), which fails
        // `validate_slug` and is dropped. Surplus opening brackets without
        // matching `]]` boundaries are noise.
        assert!(extract_refs("[[[mem/x]]]").is_empty());
        // `[[mem/x]]]` — `mem/x` matches; trailing `]` is just text.
        assert_eq!(extract_refs("[[mem/x]]]"), vec!["mem/x".to_string()]);
    }

    #[test]
    fn extract_refs_handles_utf8_around_brackets() {
        assert_eq!(
            extract_refs("héllo [[mem/example]] wörld 🎉"),
            vec!["mem/example".to_string()]
        );
    }

    #[test]
    fn extract_refs_long_slug_at_limit() {
        // Build a slug that's exactly SLUG_MAX_LEN bytes — must extract.
        let segment = "a".repeat(64);
        // mem/ (4) + segment (64) = 68 — well under the limit, but exercises
        // a long single-segment slug.
        let slug = format!("mem/{segment}");
        let body = format!("[[{slug}]]");
        assert_eq!(extract_refs(&body), vec![slug]);
    }

    #[test]
    fn extract_refs_oversized_slug_dropped() {
        // mem/ + 256 bytes = 260 bytes total > SLUG_MAX_LEN (255).
        let slug = format!("mem/{}", "a".repeat(64));
        // Repeat enough segments to bust the cap; each segment is 64+1 = 65 bytes.
        let oversized = format!(
            "{slug}/{}/{}/{}",
            "b".repeat(64),
            "c".repeat(64),
            "d".repeat(64)
        );
        assert!(oversized.len() > SLUG_MAX_LEN);
        let body = format!("[[{oversized}]]");
        assert!(extract_refs(&body).is_empty());
    }

    #[test]
    fn extract_refs_self_reference_is_allowed() {
        // The function does not know "self"; consumers handle that. We just
        // surface every well-formed `[[slug]]`.
        assert_eq!(
            extract_refs("I refer to [[mem/me]] from inside mem/me's value"),
            vec!["mem/me".to_string()]
        );
    }
}
