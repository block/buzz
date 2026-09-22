//! Strict, canonical NIP-TW requests. Unknown constraints fail rather than
//! silently describing different rows from the ones the caller requested.

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Maximum reply scan candidates per window.
pub const MAX_LIMIT: u32 = 200;
/// Conversation row kinds. Edits, reactions, deletions and metadata are aux,
/// never reply-budget candidates, even if they have thread metadata.
pub const ROW_KINDS: [u32; 4] = [9, 40002, 45001, 45003];

/// A position in `(created_at DESC, id ASC)` order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Cursor {
    /// Nonnegative Unix seconds, representable by PostgreSQL/chrono.
    pub created_at: i64,
    /// Full lowercase hexadecimal event id.
    pub id: String,
}

impl Cursor {
    /// Checked timestamp conversion (also used at the database boundary).
    pub fn timestamp(&self) -> Result<DateTime<Utc>, String> {
        DateTime::from_timestamp(self.created_at, 0)
            .filter(|_| self.created_at >= 0)
            .ok_or_else(|| "thread_window: cursor timestamp out of range".into())
    }
}

/// Validated pagination and response-affecting arguments. Construct with
/// [`Request::parse`]; the database also validates public inputs defensively.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// Exactly one canonical channel UUID.
    pub channel: Uuid,
    /// Exactly one lowercase full root event id.
    pub root: String,
    /// Raw reply candidate budget, 1..=200.
    pub limit: u32,
    /// Maximum absolute depth below the root, 1..=100.
    pub depth: u32,
    /// Sorted, deduplicated conversation row kinds.
    pub kinds: Vec<u32>,
    /// Request upper bound; absent means newest page.
    pub cursor: Option<Cursor>,
    /// Include root/retained-row edits, reactions and deletion closure.
    pub include_aux: bool,
}

fn event_id(v: &Value) -> Result<String, String> {
    v.as_str()
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| "thread_window: expected a full 64-hex event id".into())
}

fn singleton<'a>(raw: &'a Value, key: &str) -> Result<&'a Value, String> {
    raw.get(key)
        .and_then(Value::as_array)
        .filter(|a| a.len() == 1)
        .and_then(|a| a.first())
        .ok_or_else(|| format!("thread_window requires exactly one {key}"))
}

fn bounded(raw: &Value, key: &str, default: u32, max: u32) -> Result<u32, String> {
    match raw.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=u64::from(max)).contains(n))
            .map(|n| n as u32)
            .ok_or_else(|| format!("thread_window: {key} must be an integer in 1..={max}")),
    }
}

impl Request {
    /// Parse only an explicitly opted-in filter. Unsupported filter fields,
    /// including legacy cursors, offsets and other bridge modes, are errors.
    pub fn parse(raw: &Value) -> Result<Self, String> {
        let object = raw.as_object().ok_or("thread_window: expected object")?;
        const FIELDS: &[&str] = &[
            "thread_window",
            "#h",
            "#e",
            "kinds",
            "limit",
            "depth_limit",
            "until",
            "before_id",
            "include_aux",
        ];
        if object.keys().any(|key| !FIELDS.contains(&key.as_str())) {
            return Err("thread_window: unsupported or conflicting filter field".into());
        }
        if raw.get("thread_window") != Some(&Value::Bool(true)) {
            return Err("thread_window must be true".into());
        }
        let channel = singleton(raw, "#h")?
            .as_str()
            .and_then(|s| Uuid::parse_str(s).ok())
            .ok_or("thread_window: #h must be a channel UUID")?;
        let root = event_id(singleton(raw, "#e")?)?;
        let limit = bounded(raw, "limit", 50, MAX_LIMIT)?;
        let depth = bounded(raw, "depth_limit", 100, 100)?;
        let mut kinds = raw
            .get("kinds")
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty() && a.len() <= ROW_KINDS.len())
            .ok_or("thread_window requires nonempty conversation kinds")?
            .iter()
            .map(|v| {
                v.as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .filter(|n| ROW_KINDS.contains(n))
                    .ok_or_else(|| {
                        "thread_window supports row kinds 9, 40002, 45001, 45003 only".to_string()
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        kinds.sort_unstable();
        kinds.dedup();
        let cursor = match (raw.get("until"), raw.get("before_id")) {
            (None, None) => None,
            (Some(ts), Some(id)) => {
                let cursor = Cursor {
                    created_at: ts
                        .as_i64()
                        .filter(|n| *n >= 0)
                        .ok_or("thread_window: until must be nonnegative integer seconds")?,
                    id: event_id(id)?,
                };
                cursor.timestamp()?;
                Some(cursor)
            }
            _ => return Err("thread_window requires both until and before_id, or neither".into()),
        };
        let include_aux = match raw.get("include_aux") {
            None => false,
            Some(v) => v
                .as_bool()
                .ok_or("thread_window: include_aux must be boolean")?,
        };
        Ok(Self {
            channel,
            root,
            limit,
            depth,
            kinds,
            cursor,
            include_aux,
        })
    }

    /// Canonical NIP-TW v1 request identity. SHA-256 over a compact JSON array
    /// avoids ambiguous separators and binds all normalized response options.
    /// The host is server-resolved; the reader is the authenticated lowercase
    /// public key. Neither may be taken from filter-supplied fields.
    pub fn binding(&self, resolved_host: &str, reader_hex: &str) -> String {
        let cursor = self.cursor.as_ref().map(|c| json!([c.created_at, c.id]));
        let canonical = json!([
            "tw",
            1,
            "older",
            resolved_host,
            reader_hex,
            self.channel.to_string(),
            self.root,
            self.limit,
            self.depth,
            self.kinds,
            cursor,
            self.include_aux
        ]);
        format!(
            "tw:1:{}",
            hex::encode(Sha256::digest(canonical.to_string().as_bytes()))
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn filter() -> Value {
        json!({"thread_window":true,"#h":[Uuid::nil()],"#e":["ab".repeat(32)],"kinds":[9]})
    }
    #[test]
    fn rejects_malformed_and_conflicting_constraints() {
        for (key, value) in [
            ("until", json!(1)),
            ("before_id", json!("ab".repeat(32))),
            ("limit", json!(0)),
            ("limit", json!(201)),
            ("depth_limit", json!(101)),
            ("depth_limit", json!(-1)),
            ("include_aux", json!("true")),
            ("thread_cursor", json!(0)),
            ("threadCursorId", json!(null)),
            ("top_level", json!(false)),
            ("page", json!(1)),
            ("offset", json!(0)),
            ("authors", json!([])),
            ("since", json!(1)),
            ("#p", json!([])),
            ("kinds", json!([7])),
            ("kinds", json!([])),
            ("search", json!("hello")),
            ("#h", json!([Uuid::nil(), Uuid::nil()])),
            ("#e", json!(["bad"])),
        ] {
            let mut f = filter();
            f[key] = value;
            assert!(Request::parse(&f).is_err(), "{f}");
        }
        for ts in [
            json!(-1),
            json!(1.5),
            json!(null),
            json!(u64::MAX),
            json!(i64::MAX),
        ] {
            let mut f = filter();
            f["until"] = ts;
            f["before_id"] = json!("a".repeat(64));
            assert!(Request::parse(&f).is_err(), "{f}");
        }
    }
    #[test]
    fn binding_normalizes_defaults_and_binds_every_argument() {
        let f = filter();
        let request = Request::parse(&f).unwrap();
        assert_eq!(
            request.binding("relay.example", &"ab".repeat(32)),
            "tw:1:5252322dfd797ddb1d5f1150acd4cf914b9fc09e39bcf3048d25aed514b3546d"
        );
        let mut explicit = f.clone();
        explicit["limit"] = json!(50);
        explicit["depth_limit"] = json!(100);
        explicit["include_aux"] = json!(false);
        explicit["#e"] = json!(["AB".repeat(32)]);
        assert_eq!(
            request.binding("relay.example", &"ab".repeat(32)),
            Request::parse(&explicit)
                .unwrap()
                .binding("relay.example", &"ab".repeat(32))
        );
        for (key, val) in [
            ("limit", json!(49)),
            ("depth_limit", json!(1)),
            ("include_aux", json!(true)),
            ("kinds", json!([40002])),
            ("#e", json!(["cd".repeat(32)])),
            ("#h", json!([Uuid::new_v4()])),
        ] {
            let mut changed = f.clone();
            changed[key] = val;
            assert_ne!(
                request.binding("relay.example", &"ab".repeat(32)),
                Request::parse(&changed)
                    .unwrap()
                    .binding("relay.example", &"ab".repeat(32))
            );
        }
        assert_ne!(
            request.binding("relay.example", &"ab".repeat(32)),
            request.binding("other.example", &"ab".repeat(32))
        );
        assert_ne!(
            request.binding("relay.example", &"ab".repeat(32)),
            request.binding("relay.example", &"cd".repeat(32))
        );
        let mut changed = f;
        changed["until"] = json!(0);
        changed["before_id"] = json!("ab".repeat(32));
        assert_ne!(
            request.binding("relay.example", &"ab".repeat(32)),
            Request::parse(&changed)
                .unwrap()
                .binding("relay.example", &"ab".repeat(32))
        );
        assert!(crate::kind::is_relay_only_kind(
            crate::kind::KIND_THREAD_WINDOW_BOUNDS
        ));
    }
}
