//! Strict NIP-PL kind:30350 envelope and plaintext validation.
//!
//! This module performs only syntax/policy validation. Tenant selection remains
//! the caller's responsibility and must come from `TenantContext`, never from
//! the decrypted `origin` member.

use std::collections::HashSet;

use nostr::Event;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};
use sha2::Digest as _;

pub(crate) const PUSH_KINDS: &[u64] = &[7, 9, 1059, 40007, 46010];
pub(crate) const URGENT_KINDS: &[u64] = &[];

/// NIP-PL addressable push-lease event kind.
pub const KIND_PUSH_LEASE: u32 = 30_350;
/// Largest integer represented exactly by interoperable JSON number implementations.
pub const MAX_SAFE_JSON_INTEGER: u64 = (1_u64 << 53) - 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseEnvelope {
    pub installation_id: String,
    pub expiration: i64,
    pub executor_key_id: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LeasePlaintext {
    pub v: u64,
    pub origin: String,
    pub generation: u64,
    pub active: bool,
    pub app_profile: Option<String>,
    pub transport: Option<String>,
    pub endpoint: Option<String>,
    pub subscriptions: Option<Vec<Subscription>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Subscription {
    pub filter: Map<String, Value>,
    pub class: String,
    #[serde(default)]
    pub ignore: Vec<Map<String, Value>>,
    pub suppress: Option<Suppress>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Suppress {
    pub p_tags_max: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct AppProfile<'a> {
    pub id: &'a str,
    pub transport: &'a str,
}

pub struct LeaseLimits<'a> {
    pub expected_origin: &'a str,
    pub author_hex: &'a str,
    pub app_profiles: &'a [AppProfile<'a>],
    pub supported_classes: &'a [&'a str],
    pub push_kinds: &'a [u64],
    pub urgent_kinds: &'a [u64],
    pub max_subscriptions: usize,
    pub max_kinds: usize,
    pub max_authors: usize,
    pub max_h: usize,
    pub max_tag_values: usize,
    pub max_ignore: usize,
    pub max_endpoint_len: usize,
    pub max_string_len: usize,
}

/// Validate the signed event's public tags and lease lifetime.
pub fn validate_envelope(
    event: &Event,
    now: i64,
    allowed_skew_secs: i64,
    max_lease_ttl_secs: i64,
    max_content_len: usize,
) -> Result<LeaseEnvelope, String> {
    if event.kind.as_u16() as u32 != KIND_PUSH_LEASE {
        return Err("wrong event kind".into());
    }
    if event.content.len() > max_content_len {
        return Err("content too long".into());
    }

    let mut d = None;
    let mut expiration = None;
    let mut exec = None;
    let mut alt_seen = false;
    for tag in event.tags.iter() {
        let parts = tag.as_slice();
        let Some(name) = parts.first().map(String::as_str) else {
            return Err("empty public tag".into());
        };
        if parts.len() != 2 {
            return Err(format!("{name} tag must have exactly one value"));
        }
        let value = &parts[1];
        match name {
            "d" if d.replace(value.clone()).is_none() => {}
            "expiration" if expiration.replace(value.clone()).is_none() => {}
            "exec" if exec.replace(value.clone()).is_none() => {}
            "alt" if !alt_seen => alt_seen = true,
            "d" | "expiration" | "exec" | "alt" => {
                return Err(format!("duplicate {name} tag"));
            }
            _ => return Err(format!("unexpected public tag: {name}")),
        }
    }

    let installation_id = d.ok_or_else(|| "missing d tag".to_string())?;
    if installation_id.is_empty() || installation_id.len() > 64 {
        return Err("invalid d tag length".into());
    }
    let expiration = expiration
        .ok_or_else(|| "missing expiration tag".to_string())?
        .parse::<i64>()
        .map_err(|_| "expiration must be integer Unix seconds".to_string())?;
    if expiration <= now - allowed_skew_secs {
        return Err("lease already expired".into());
    }
    if expiration > now + max_lease_ttl_secs {
        return Err("lease ttl too long".into());
    }
    let executor_key_id = exec.ok_or_else(|| "missing exec tag".to_string())?;
    if executor_key_id.is_empty() {
        return Err("empty exec tag".into());
    }

    Ok(LeaseEnvelope {
        installation_id,
        expiration,
        executor_key_id,
    })
}

/// Parse one bounded plaintext object, rejecting duplicate keys at every depth and non-exact schemas.
pub fn parse_plaintext(input: &str, max_plaintext_len: usize) -> Result<LeasePlaintext, String> {
    if input.len() > max_plaintext_len {
        return Err("plaintext too long".into());
    }
    let mut deserializer = serde_json::Deserializer::from_str(input);
    let value = NoDuplicates
        .deserialize(&mut deserializer)
        .map_err(|e| e.to_string())?;
    deserializer.end().map_err(|e| e.to_string())?;
    let object = value
        .as_object()
        .ok_or_else(|| "lease plaintext must be an object".to_string())?;
    let active = object
        .get("active")
        .ok_or_else(|| "missing active".to_string())?
        .as_bool()
        .ok_or_else(|| "active must be a boolean".to_string())?;
    let required: &[&str] = if active {
        &[
            "v",
            "origin",
            "app_profile",
            "transport",
            "endpoint",
            "generation",
            "active",
            "subscriptions",
        ]
    } else {
        &["v", "origin", "generation", "active"]
    };
    let optional: &[&str] = &[];
    if let Some(key) = required.iter().find(|key| !object.contains_key(**key)) {
        return Err(format!("missing {key}"));
    }
    if let Some(key) = object
        .keys()
        .find(|key| !required.contains(&key.as_str()) && !optional.contains(&key.as_str()))
    {
        return Err(format!("unknown field: {key}"));
    }
    serde_json::from_value(value).map_err(|e| format!("invalid lease schema: {e}"))
}

/// Validate a parsed v1 plaintext against the server-resolved descriptor policy.
pub fn validate_plaintext(body: &LeasePlaintext, limits: &LeaseLimits<'_>) -> Result<(), String> {
    if body.v != 1 {
        return Err("unsupported version".into());
    }
    if body.generation == 0 || body.generation > MAX_SAFE_JSON_INTEGER {
        return Err("generation must be a positive safe integer".into());
    }
    if body.origin != limits.expected_origin {
        return Err("origin mismatch".into());
    }
    check_string(&body.origin, limits.max_string_len)?;

    if !body.active {
        if body.app_profile.is_some()
            || body.transport.is_some()
            || body.endpoint.is_some()
            || body.subscriptions.is_some()
        {
            return Err("inactive lease must use minimal schema".into());
        }
        return Ok(());
    }

    let app_profile = body.app_profile.as_deref().ok_or("missing app_profile")?;
    let transport = body.transport.as_deref().ok_or("missing transport")?;
    let endpoint = body.endpoint.as_deref().ok_or("missing endpoint")?;
    let subscriptions = body.subscriptions.as_ref().ok_or("missing subscriptions")?;
    let advertised = limits
        .app_profiles
        .iter()
        .find(|profile| profile.id == app_profile)
        .ok_or("app profile not supported")?;
    if transport != advertised.transport {
        return Err("transport mismatch".into());
    }
    if endpoint.is_empty() || endpoint.len() > limits.max_endpoint_len {
        return Err("invalid endpoint length".into());
    }
    check_string(app_profile, limits.max_string_len)?;
    check_string(transport, limits.max_string_len)?;
    check_string(endpoint, limits.max_endpoint_len)?;
    if subscriptions.is_empty() || subscriptions.len() > limits.max_subscriptions {
        return Err("subscription quota exceeded".into());
    }
    for subscription in subscriptions {
        validate_subscription(subscription, limits)?;
    }
    Ok(())
}

fn validate_subscription(sub: &Subscription, limits: &LeaseLimits<'_>) -> Result<(), String> {
    if !limits.supported_classes.contains(&sub.class.as_str()) {
        return Err("class not supported".into());
    }
    validate_filter(&sub.filter, limits, true, &sub.class)?;
    if sub.ignore.len() > limits.max_ignore {
        return Err("ignore quota exceeded".into());
    }
    for filter in &sub.ignore {
        // Ignore filters can only subtract from an already-positive match, so
        // urgent-kind confinement belongs solely to the positive filter.
        validate_filter(filter, limits, false, "")?;
    }
    if sub.suppress.as_ref().is_some_and(|s| s.p_tags_max == 0) {
        return Err("p_tags_max must be positive".into());
    }
    Ok(())
}

fn validate_filter(
    filter: &Map<String, Value>,
    limits: &LeaseLimits<'_>,
    require_narrowing: bool,
    class: &str,
) -> Result<(), String> {
    const ALLOWED: &[&str] = &["kinds", "authors", "#p", "#h", "#e"];
    if let Some(key) = filter.keys().find(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(format!("filter member not permitted: {key}"));
    }
    let kinds = string_or_integer_array(filter, "kinds", limits.max_kinds, true)?;
    let kinds: Vec<u64> = kinds
        .iter()
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| "kind must be an integer".to_string())
        })
        .collect::<Result<_, _>>()?;
    if kinds.iter().any(|kind| !limits.push_kinds.contains(kind)) {
        return Err("kind not push-eligible".into());
    }
    if class == "urgent" && kinds.iter().any(|kind| !limits.urgent_kinds.contains(kind)) {
        return Err("class not permitted for kind".into());
    }

    let authors = optional_string_array(filter, "authors", limits.max_authors)?;
    let p = optional_string_array(filter, "#p", limits.max_tag_values)?;
    let h = optional_string_array(filter, "#h", limits.max_h)?;
    let e = optional_string_array(filter, "#e", limits.max_tag_values)?;
    if require_narrowing && authors.is_none() && p.is_none() && h.is_none() {
        return Err("lease filter not narrowed".into());
    }
    for value in authors.into_iter().flatten() {
        check_exact_hex(value, "author")?;
    }
    for value in p.into_iter().flatten() {
        check_exact_hex(value, "p tag")?;
        if value != limits.author_hex {
            return Err("p-tag must be self".into());
        }
    }
    for value in h.into_iter().flatten() {
        check_string(value, limits.max_string_len)?;
        let uuid = uuid::Uuid::parse_str(value).map_err(|_| "invalid h tag".to_string())?;
        if uuid.get_version_num() != 4 || uuid.to_string() != *value {
            return Err("invalid h tag".into());
        }
    }
    for value in e.into_iter().flatten() {
        check_exact_hex(value, "e tag")?;
    }
    Ok(())
}

fn string_or_integer_array<'a>(
    filter: &'a Map<String, Value>,
    key: &str,
    max: usize,
    required: bool,
) -> Result<&'a Vec<Value>, String> {
    let Some(value) = filter.get(key) else {
        return if required {
            Err(format!("missing {key}"))
        } else {
            Err("internal optional-array misuse".into())
        };
    };
    let values = value
        .as_array()
        .ok_or_else(|| format!("{key} must be an array"))?;
    if values.is_empty() || values.len() > max {
        return Err(format!("invalid {key} count"));
    }
    Ok(values)
}

fn optional_string_array<'a>(
    filter: &'a Map<String, Value>,
    key: &str,
    max: usize,
) -> Result<Option<Vec<&'a str>>, String> {
    let Some(value) = filter.get(key) else {
        return Ok(None);
    };
    let values = value
        .as_array()
        .ok_or_else(|| format!("{key} must be an array"))?;
    if values.is_empty() || values.len() > max {
        return Err(format!("invalid {key} count"));
    }
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| format!("{key} values must be strings"))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn check_exact_hex(value: &str, label: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(format!("non-exact match value for {label}"));
    }
    Ok(())
}

fn check_string(value: &str, max: usize) -> Result<(), String> {
    if value.is_empty() || value.len() > max {
        return Err("invalid string length".into());
    }
    Ok(())
}

struct NoDuplicates;

impl<'de> DeserializeSeed<'de> for NoDuplicates {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(NoDuplicatesVisitor)
    }
}

struct NoDuplicatesVisitor;

impl<'de> Visitor<'de> for NoDuplicatesVisitor {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }
    fn visit_i64<E>(self, value: i64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_u64<E>(self, value: u64) -> Result<Value, E> {
        Ok(Value::Number(value.into()))
    }
    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("non-finite number"))
    }
    fn visit_str<E>(self, value: &str) -> Result<Value, E> {
        Ok(Value::String(value.into()))
    }
    fn visit_string<E>(self, value: String) -> Result<Value, E> {
        Ok(Value::String(value))
    }
    fn visit_none<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        NoDuplicates.deserialize(d)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(NoDuplicates)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        let mut seen = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            if !seen.insert(key.clone()) {
                return Err(de::Error::custom(format!("duplicate object key: {key}")));
            }
            values.insert(key, map.next_value_seed(NoDuplicates)?);
        }
        Ok(Value::Object(values))
    }
}

/// A lease rejection is either caller-correctable protocol validation or a
/// transient/internal dependency failure that must remain retryable.
#[derive(Debug)]
pub enum AcceptError {
    Validation(String),
    Internal(String),
}

impl From<String> for AcceptError {
    fn from(reason: String) -> Self {
        Self::Validation(reason)
    }
}

/// Fully validate, provision, and atomically persist one kind:30350 lease.
pub async fn accept(
    tenant: &buzz_core::TenantContext,
    state: &std::sync::Arc<crate::state::AppState>,
    event: &Event,
    now: i64,
) -> Result<buzz_db::push::AcceptLeaseOutcome, AcceptError> {
    const MAX_LEASE_TTL: i64 = 30 * 24 * 60 * 60;
    const ALLOWED_SKEW: i64 = 120;
    const MAX_CONTENT: usize = 65_536;
    const MAX_PLAINTEXT: usize = 32_768;
    const MAX_ACTIVE_LEASES: i64 = 16;
    if state.config.push_gateway_delivery_url.is_none() {
        return Err(AcceptError::Validation("push not supported".to_string()));
    }
    let envelope = validate_envelope(event, now, ALLOWED_SKEW, MAX_LEASE_TTL, MAX_CONTENT)?;
    if envelope.executor_key_id != state.config.push_executor_key_id {
        return Err(AcceptError::Validation("unknown executor key".to_string()));
    }
    let plaintext = nostr::nips::nip44::decrypt(
        state.relay_keypair.secret_key(),
        &event.pubkey,
        &event.content,
    )
    .map_err(|_| "invalid encrypted content".to_string())?;
    let body = parse_plaintext(&plaintext, MAX_PLAINTEXT)?;
    let origin = canonical_origin(&state.config.relay_url, tenant.host())?;
    let author_hex = event.pubkey.to_hex();
    let limits = LeaseLimits {
        expected_origin: &origin,
        author_hex: &author_hex,
        app_profiles: &[
            AppProfile {
                id: "buzz-ios-production",
                transport: "apns",
            },
            AppProfile {
                id: "buzz-ios-sandbox",
                transport: "apns",
            },
        ],
        supported_classes: &["silent", "default", "time_sensitive"],
        push_kinds: PUSH_KINDS,
        urgent_kinds: URGENT_KINDS,
        max_subscriptions: 16,
        max_kinds: 16,
        max_authors: 20,
        max_h: 50,
        max_tag_values: 20,
        max_ignore: 8,
        max_endpoint_len: 4096,
        max_string_len: 512,
    };
    validate_plaintext(&body, &limits)?;
    let generation =
        i64::try_from(body.generation).map_err(|_| "invalid generation".to_string())?;
    let version = buzz_db::push::LeaseVersion {
        source_event_id: event.id.as_bytes(),
        source_created_at: event.created_at.as_secs() as i64,
        generation,
        expires_at: envelope.expiration,
    };
    let endpoint_hash;
    let subscriptions;
    let capability;
    let active = if body.active {
        let endpoint = body.endpoint.as_deref().expect("validated active endpoint");
        endpoint_hash = sha2::Sha256::digest(endpoint.as_bytes()).to_vec();
        let max_class = body
            .subscriptions
            .as_ref()
            .expect("validated subscriptions")
            .iter()
            .map(|sub| sub.class.as_str())
            .max_by_key(|class| class_rank(class))
            .expect("non-empty subscriptions");
        capability = endpoint.to_owned();
        subscriptions = serde_json::to_value(
            body.subscriptions
                .as_ref()
                .expect("validated subscriptions"),
        )
        .map_err(|_| "invalid subscriptions".to_string())?;
        Some(buzz_db::push::ActiveLease {
            app_profile: body.app_profile.as_deref().expect("validated profile"),
            endpoint_hash: &endpoint_hash,
            endpoint_grant: &capability,
            max_class,
            subscriptions: &subscriptions,
        })
    } else {
        None
    };
    state
        .db
        .accept_push_lease_event(
            tenant.community(),
            event,
            &envelope.installation_id,
            version,
            active,
            MAX_ACTIVE_LEASES,
        )
        .await
        .map_err(|_| AcceptError::Internal("lease persistence failed".to_string()))
}

fn class_rank(class: &str) -> u8 {
    match class {
        "silent" => 0,
        "default" => 1,
        "time_sensitive" => 2,
        "urgent" => 3,
        _ => 0,
    }
}

fn canonical_origin(relay_url: &str, host: &str) -> Result<String, String> {
    let scheme = if relay_url.starts_with("wss://") {
        "wss"
    } else if relay_url.starts_with("ws://") {
        "ws"
    } else {
        return Err("invalid relay URL".to_string());
    };
    if host.is_empty() {
        return Err("invalid tenant host".to_string());
    }
    Ok(format!("{scheme}://{host}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};

    fn event(tags: Vec<Tag>) -> Event {
        EventBuilder::new(Kind::Custom(KIND_PUSH_LEASE as u16), "ciphertext")
            .tags(tags)
            .custom_created_at(Timestamp::from(1_000_u64))
            .sign_with_keys(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn envelope_rejects_extra_and_duplicate_tags() {
        let base = vec![
            Tag::parse(["d", "installation"]).unwrap(),
            Tag::parse(["expiration", "1100"]).unwrap(),
            Tag::parse(["exec", "key"]).unwrap(),
        ];
        assert!(validate_envelope(&event(base.clone()), 1_000, 10, 200, 100).is_ok());
        let mut duplicate = base.clone();
        duplicate.push(Tag::parse(["d", "other"]).unwrap());
        assert_eq!(
            validate_envelope(&event(duplicate), 1_000, 10, 200, 100).unwrap_err(),
            "duplicate d tag"
        );
        let mut extra = base;
        extra.push(Tag::parse(["p", "secret"]).unwrap());
        assert_eq!(
            validate_envelope(&event(extra), 1_000, 10, 200, 100).unwrap_err(),
            "unexpected public tag: p"
        );
    }

    #[test]
    fn envelope_rejects_wrong_kind() {
        let event = EventBuilder::new(Kind::TextNote, "ciphertext")
            .tags([
                Tag::parse(["d", "installation"]).unwrap(),
                Tag::parse(["expiration", "1100"]).unwrap(),
                Tag::parse(["exec", "key"]).unwrap(),
            ])
            .custom_created_at(Timestamp::from(1_000_u64))
            .sign_with_keys(&Keys::generate())
            .unwrap();
        assert_eq!(
            validate_envelope(&event, 1_000, 10, 200, 100).unwrap_err(),
            "wrong event kind"
        );
    }

    #[test]
    fn parser_rejects_duplicate_keys_at_any_depth() {
        let top = r##"{"v":1,"v":1,"origin":"o","generation":1,"active":false}"##;
        assert!(parse_plaintext(top, 1024)
            .unwrap_err()
            .contains("duplicate object key: v"));
        let nested = r##"{"v":1,"origin":"o","generation":1,"active":true,"app_profile":"p","transport":"apns","endpoint":"e","subscriptions":[{"filter":{"kinds":[9],"kinds":[7],"#p":["aa"]},"class":"default"}]}"##;
        assert!(parse_plaintext(nested, 4096)
            .unwrap_err()
            .contains("duplicate object key: kinds"));
    }

    #[test]
    fn inactive_schema_is_minimal() {
        assert_eq!(
            parse_plaintext(
                r##"{"v":1,"origin":"o","generation":1,"active":false,"endpoint":"x"}"##,
                1024,
            )
            .unwrap_err(),
            "unknown field: endpoint"
        );
    }

    fn limits<'a>() -> LeaseLimits<'a> {
        LeaseLimits {
            expected_origin: "o",
            author_hex: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            app_profiles: &[AppProfile {
                id: "p",
                transport: "apns",
            }],
            supported_classes: &["default", "urgent"],
            push_kinds: &[9, 46010],
            urgent_kinds: &[46010],
            max_subscriptions: 4,
            max_kinds: 4,
            max_authors: 4,
            max_h: 4,
            max_tag_values: 4,
            max_ignore: 2,
            max_endpoint_len: 128,
            max_string_len: 128,
        }
    }

    #[test]
    fn migration_trigger_allowlist_matches_advertised_push_kinds() {
        let kinds = PUSH_KINDS
            .iter()
            .map(u64::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let predicate = format!("NEW.kind IN ({kinds})");
        let migration = include_str!("../../../../migrations/0018_push_match_queue.sql");
        assert!(
            migration.contains(&predicate),
            "migration trigger must use PUSH_KINDS exactly: {predicate}"
        );
    }

    #[test]
    fn active_filter_requires_narrowing_and_self_p_tag() {
        let body = parse_plaintext(r##"{"v":1,"origin":"o","generation":1,"active":true,"app_profile":"p","transport":"apns","endpoint":"token","subscriptions":[{"filter":{"kinds":[9]},"class":"default"}]}"##, 4096).unwrap();
        assert_eq!(
            validate_plaintext(&body, &limits()).unwrap_err(),
            "lease filter not narrowed"
        );
    }

    #[test]
    fn profile_transport_and_positive_generation_are_enforced() {
        let body = parse_plaintext(r##"{"v":1,"origin":"o","generation":1,"active":true,"app_profile":"p","transport":"fcm","endpoint":"token","subscriptions":[{"filter":{"kinds":[9],"#p":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]},"class":"default"}]}"##, 4096).unwrap();
        assert_eq!(
            validate_plaintext(&body, &limits()).unwrap_err(),
            "transport mismatch"
        );

        let body = parse_plaintext(
            r##"{"v":1,"origin":"o","generation":0,"active":false}"##,
            4096,
        )
        .unwrap();
        assert_eq!(
            validate_plaintext(&body, &limits()).unwrap_err(),
            "generation must be a positive safe integer"
        );
    }

    #[test]
    fn h_uses_its_advertised_limit_not_the_generic_tag_limit() {
        let body = parse_plaintext(r##"{"v":1,"origin":"o","generation":1,"active":true,"app_profile":"p","transport":"apns","endpoint":"token","subscriptions":[{"filter":{"kinds":[9],"#h":["123e4567-e89b-42d3-a456-426614174000","123e4567-e89b-42d3-a456-426614174001"]},"class":"default"}]}"##, 4096).unwrap();
        let mut limits = limits();
        limits.max_h = 2;
        limits.max_tag_values = 1;
        assert!(validate_plaintext(&body, &limits).is_ok());
    }

    #[test]
    fn canonical_origin_preserves_server_resolved_authority() {
        assert_eq!(
            canonical_origin("wss://relay.example", "tenant.example:8443").unwrap(),
            "wss://tenant.example:8443"
        );
        assert_eq!(
            canonical_origin("ws://relay.example", "[::1]:3000").unwrap(),
            "ws://[::1]:3000"
        );
        assert!(canonical_origin("https://relay.example", "tenant.example").is_err());
        assert!(canonical_origin("wss://relay.example", "").is_err());
    }

    #[test]
    fn urgent_is_limited_by_event_kind() {
        let body = parse_plaintext(r##"{"v":1,"origin":"o","generation":1,"active":true,"app_profile":"p","transport":"apns","endpoint":"token","subscriptions":[{"filter":{"kinds":[9],"#p":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]},"class":"urgent"}]}"##, 4096).unwrap();
        assert_eq!(
            validate_plaintext(&body, &limits()).unwrap_err(),
            "class not permitted for kind"
        );
    }
}
