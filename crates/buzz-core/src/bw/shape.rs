use super::*;
use nostr::{JsonUtil, PublicKey};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

fn hex(v: &str, len: usize) -> bool {
    v.len() == len
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn key(v: &str) -> bool {
    hex(v, 64) && PublicKey::from_hex(v).and_then(|p| p.xonly()).is_ok()
}
fn token(v: &str, extra: &str) -> bool {
    !v.is_empty()
        && v.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || extra.as_bytes().contains(&b))
}
pub(super) fn url(v: &str) -> bool {
    v.len() <= 2048
        && url::Url::parse(v).is_ok_and(|u| {
            u.scheme() == "https"
                && u.host_str().is_some()
                && u.username().is_empty()
                && u.password().is_none()
                && u.fragment().is_none()
        })
}
pub(super) fn repo(v: &str) -> bool {
    let parts: Vec<_> = v.split(':').collect();
    parts.len() == 3
        && parts[0] == "30617"
        && key(parts[1])
        && parts[2].len() <= 64
        && token(parts[2], "_-")
        && parts[2].as_bytes()[0].is_ascii_alphanumeric()
}
fn primitive(v: &Value, t: &str) -> bool {
    let text = s(v);
    match t {
        "id" => hex(text, 64),
        "pubkey" => key(text),
        "git" => hex(text, 40),
        "bool" => v.is_boolean(),
        "time" => v.as_u64().is_some_and(|x| x <= u32::MAX as u64),
        "positive" => v
            .as_u64()
            .is_some_and(|x| (1..=i32::MAX as u64).contains(&x)),
        "text" | "longtext" | "title" | "note" => {
            v.is_string()
                && (if t == "note" { 0 } else { 1 }..=match t {
                    "longtext" => 8192,
                    "title" => 256,
                    _ => 2048,
                })
                    .contains(&text.len())
        }
        "url" => v.is_string() && url(text),
        "repo" => repo(text),
        "stream" => {
            text.len() <= 128
                && token(text, "._/-")
                && text.as_bytes()[0].is_ascii_alphanumeric()
                && !text.contains("..")
                && !text.contains("@{")
                && !text.ends_with(['.', '/'])
                && text
                    .split('/')
                    .all(|p| !p.is_empty() && !p.starts_with('.') && !p.ends_with(".lock"))
        }
        "release" => {
            text.len() <= 64 && token(text, "._+-") && text.as_bytes()[0].is_ascii_alphanumeric()
        }
        "path" => {
            !text.is_empty()
                && text.len() <= 256
                && !text.contains('\\')
                && !text.starts_with('/')
                && text.split('/').all(|x| !matches!(x, "" | "." | ".."))
        }
        "runid" => {
            !text.is_empty()
                && text.len() <= 128
                && text
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b))
        }
        _ => false,
    }
}
fn typed(v: &Value, t: &str, field: &str, schema: &Value) -> bool {
    if let Some(t) = t.strip_prefix('?') {
        return v.is_null() || typed(v, t, field, schema);
    }
    if let Some(t) = t.strip_prefix('[').and_then(|x| x.strip_suffix(']')) {
        let bounds = if schema["arrays"][field].is_null() {
            &schema["arrays"]["default"]
        } else {
            &schema["arrays"][field]
        };
        return v.is_array()
            && (n(&bounds[0])..=n(&bounds[1])).contains(&(a(v).len() as u64))
            && a(v)
                .iter()
                .enumerate()
                .all(|(i, x)| !a(v)[..i].contains(x) && typed(x, t, field, schema));
    }
    if t.contains('|') {
        return t.split('|').any(|x| v == x);
    }
    if !schema["types"][t].is_null() {
        return object(v, &schema["types"][t], schema)
            && (t != "patch" || v.as_object().is_some_and(|x| !x.is_empty()));
    }
    primitive(v, t)
}
fn object(v: &Value, definition: &Value, schema: &Value) -> bool {
    let Some(obj) = v.as_object() else {
        return false;
    };
    let Some(required) = definition["required"].as_object() else {
        return false;
    };
    if !required.keys().all(|k| obj.contains_key(k)) {
        return false;
    }
    obj.iter().all(|(k, v)| {
        let t = required.get(k).or_else(|| definition["optional"].get(k));
        t.is_some_and(|t| typed(v, s(t), k, schema))
    })
}
/// Validate the BW-specific shape of an unsigned draft. This does not grant roles
/// or authorize publication. Content bytes and tag order are left untouched.
pub fn validate_shape(kind: u64, tags: &Value, content: &str) -> Result<(), String> {
    if !tags.is_array()
        || !a(tags)
            .iter()
            .all(|t| t.is_array() && !a(t).is_empty() && a(t).iter().all(Value::is_string))
    {
        return Err("tag-array".into());
    }
    if kind == 1
        && !a(tags)
            .iter()
            .any(|t| t[0] == "t" && matches!(s(&t[1]), "assignment" | "unassignment"))
    {
        return Err("assignment-profile".into());
    }
    // Reserve the full signed envelope size at the largest valid timestamp.
    let wire = json!({"id":"0".repeat(64),"pubkey":"0".repeat(64),"sig":"0".repeat(128),"created_at":u32::MAX,"kind":kind,"tags":tags,"content":content});
    check(&wire).map(|_| ()).map_err(|f| f.code.into())
}
fn check(w: &Value) -> Check<Value> {
    let mut errors = BTreeSet::new();
    let mut add = |ok: bool, code| {
        if !ok {
            errors.insert(code);
        }
    };
    let tags = a(&w["tags"]);
    let kind = n(&w["kind"]);
    let content = s(&w["content"]);
    add(
        content.len() <= 32768
            && tags.len() <= 16
            && serde_json::to_vec(w)
                .map(|b| b.len() <= 65536)
                .unwrap_or(false)
            && tags
                .iter()
                .all(|t| a(t).iter().all(|s| super::s(s).len() <= 2048)),
        "oversize",
    );
    let mut names = BTreeSet::new();
    for tag in tags {
        add(names.insert(s(&tag[0])), "duplicate-tag");
    }
    let tag = |name: &str| {
        tags.iter()
            .find(|t| s(&t[0]) == name)
            .map(|t| s(&t[1]))
            .unwrap_or("")
    };
    let body = if kind == 46100 {
        match parse_json(content.as_bytes()) {
            Ok(v) => v,
            Err(_) => {
                add(false, "json");
                Value::Null
            }
        }
    } else {
        Value::Null
    };
    let schema: Value =
        serde_json::from_str(include_str!("schema.json")).map_err(|_| fail("shape", "schema"))?;
    let mut required: Vec<&str>;
    let mut allowed: Vec<&str>;
    match kind {
        46100 => {
            let def = &schema["schemas"][tag("record")];
            add(!def.is_null(), "unknown-record");
            required = vec!["record", "a"];
            required.extend(a(&def["tags"]).iter().map(s));
            if tag("record") != "role-policy" || names.contains("previous") {
                required.push("policy");
            }
            allowed = required.clone();
            allowed.extend(a(&def["optional_tags"]).iter().map(s));
            if !def.is_null() {
                add(object(&body, def, &schema), "schema");
            }
            // Closed conditional shapes, in addition to the shared field type table.
            let conditional: Option<Vec<&str>> = match tag("record") {
                "triage-action" => Some(match s(&body["action"]) {
                    "accept" => vec!["action"],
                    "need-info" => vec!["action", "question", "recipient"],
                    "snooze" => vec!["action", "until"],
                    "duplicate" => vec!["action", "target"],
                    _ => vec!["action", "reason"],
                }),
                "issue-state" => Some(match s(&body["state"]) {
                    "triage" => vec!["state"],
                    "backlog" => vec!["state", "triage"],
                    "ready" => {
                        let mut v = vec!["state", "stream", "assignment", "update", "rework"];
                        if body.get("terminal_set").is_some() {
                            v.push("terminal_set")
                        }
                        v
                    }
                    "in-development" => vec!["state", "stream", "assignment"],
                    _ => vec![
                        "state",
                        "stream",
                        "assignment",
                        "commit",
                        "tests",
                        "remote_readback",
                    ],
                }),
                "release-set" => Some(if body["action"] == "freeze" {
                    vec![
                        "action",
                        "release",
                        "pipeline",
                        "platform",
                        "stream",
                        "relay_sha",
                        "members",
                        "readback",
                    ]
                } else {
                    vec!["action", "set", "outcome", "reason"]
                }),
                _ => None,
            };
            if let Some(keys) = conditional.filter(|_| body.is_object()) {
                add(
                    body.as_object().is_some_and(|o| {
                        o.len() == keys.len() && keys.iter().all(|k| o.contains_key(*k))
                    }),
                    "conditional-fields",
                );
            }
            if tag("record") == "release-set" && body["action"] == "freeze" {
                for key in ["issue", "implemented"] {
                    let mut seen = BTreeSet::new();
                    add(
                        a(&body["members"]).iter().all(|m| seen.insert(s(&m[key]))),
                        "duplicate-member",
                    );
                }
            }
        }
        1063 => {
            required = vec![
                "a", "set", "run", "platform", "release", "url", "m", "x", "size",
            ];
            allowed = required.clone();
            add(primitive(&w["content"], "text"), "schema");
        }
        1621 => {
            required = vec!["a", "subject"];
            allowed = required.clone();
            add(primitive(&json!(tag("subject")), "title"), "schema");
        }
        30617 => {
            required = vec!["d"];
            allowed = vec![
                "d",
                "name",
                "description",
                "clone",
                "relays",
                "maintainers",
                "web",
                "r",
                "t",
                "buzz-channel",
            ]
        }
        1 if matches!(tag("t"), "assignment" | "unassignment") => {
            required = vec!["e", "a", "p", "t"];
            allowed = vec!["e", "a", "p", "t", "prior"];
            add(
                tags.iter()
                    .find(|t| s(&t[0]) == "e")
                    .is_some_and(|t| t[2] == "" && t[3] == "root"),
                "arity",
            );
        }
        _ => {
            required = vec![];
            allowed = names.iter().copied().collect();
        }
    }
    allowed.push("auth");
    add(required.iter().all(|k| names.contains(k)), "missing-tag");
    add(names.iter().all(|k| allowed.contains(k)), "unknown-tag");
    for t in tags {
        let name = s(&t[0]);
        let expected = if name == "auth" || (kind == 1 && name == "e") {
            4
        } else {
            2
        };
        // Repository metadata retains NIP-34's multi-value forms. The existing
        // Buzz channel binding is a singleton name/value tag, not BW authority.
        if kind != 30617 || name == "buzz-channel" {
            add(a(t).len() == expected, "arity");
        }
        let value = &t[1];
        let typ = match name {
            "a" => Some("repo"),
            "issue" | "policy" | "previous" | "prior" | "delegation" | "set" | "run" | "x"
            | "e" => Some("id"),
            "p" => Some("pubkey"),
            "url" => Some("url"),
            "release" => Some("release"),
            _ => None,
        };
        if let Some(typ) = typ {
            add(primitive(value, typ), "tag-value");
        }
        if name == "auth" {
            add(
                key(s(value)) && s(&t[2]).len() <= 256 && hex(s(&t[3]), 128),
                "auth-shape",
            );
        }
        if name == "size" {
            let x = s(value);
            add(
                !x.starts_with('0')
                    && x.bytes().all(|b| b.is_ascii_digit())
                    && x.parse::<u32>()
                        .is_ok_and(|x| x > 0 && x <= i32::MAX as u32),
                "tag-value",
            );
        }
        if name == "platform" {
            add(
                matches!(s(value), "windows" | "android" | "ios" | "macos"),
                "tag-value",
            );
        }
        if name == "m" {
            let p: Vec<_> = s(value).split('/').collect();
            add(
                p.len() == 2 && p.iter().all(|s| token(s, ".+-")),
                "tag-value",
            );
        }
    }
    if let Some(code) = errors.first() {
        Err(fail("shape", code))
    } else {
        Ok(body)
    }
}
pub(super) fn decode(w: Value) -> Check<Record> {
    let obj = w.as_object().ok_or_else(|| fail("envelope", "fields"))?;
    if obj.len() != 7
        || ![
            "id",
            "pubkey",
            "created_at",
            "kind",
            "tags",
            "content",
            "sig",
        ]
        .iter()
        .all(|k| obj.contains_key(*k))
        || !hex(s(&w["id"]), 64)
        || !key(s(&w["pubkey"]))
        || !hex(s(&w["sig"]), 128)
        || !primitive(&w["created_at"], "time")
        || w["kind"].as_u64().is_none_or(|k| k > 65535)
        || !w["content"].is_string()
        || !w["tags"].is_array()
        || !a(&w["tags"])
            .iter()
            .all(|t| t.is_array() && !a(t).is_empty() && a(t).iter().all(Value::is_string))
    {
        return Err(fail("envelope", "fields"));
    }
    let preimage = serde_json::to_vec(&json!([
        0,
        w["pubkey"],
        w["created_at"],
        w["kind"],
        w["tags"],
        w["content"]
    ]))
    .map_err(|_| fail("envelope", "json"))?;
    if hex::encode(Sha256::digest(preimage)) != s(&w["id"]) {
        return Err(fail("id", "event-id"));
    }
    let e = nostr::Event::from_json(w.to_string()).map_err(|_| fail("signature", "signature"))?;
    if !e.verify_signature() {
        return Err(fail("signature", "signature"));
    }
    let body = check(&w)?;
    if let Some(tag) = a(&w["tags"]).iter().find(|t| s(&t[0]) == "auth") {
        let owner =
            PublicKey::from_hex(s(&tag[1])).map_err(|_| fail("attestation", "auth-signature"))?;
        let sig = s(&tag[3])
            .parse::<nostr::secp256k1::schnorr::Signature>()
            .map_err(|_| fail("attestation", "auth-signature"))?;
        let preimage = format!("nostr:agent-auth:{}:{}", s(&w["pubkey"]), s(&tag[2]));
        let message =
            nostr::secp256k1::Message::from_digest(Sha256::digest(preimage.as_bytes()).into());
        if s(&tag[1]) == s(&w["pubkey"])
            || nostr::SECP256K1
                .verify_schnorr(
                    &sig,
                    &message,
                    &owner
                        .xonly()
                        .map_err(|_| fail("attestation", "auth-signature"))?,
                )
                .is_err()
        {
            return Err(fail("attestation", "auth-signature"));
        }
        if !s(&tag[2]).is_empty() {
            for clause in s(&tag[2]).split('&') {
                let (digits, actual, op) = if let Some(v) = clause.strip_prefix("kind=") {
                    (v, n(&w["kind"]), '=')
                } else if let Some(v) = clause.strip_prefix("created_at<") {
                    (v, n(&w["created_at"]), '<')
                } else if let Some(v) = clause.strip_prefix("created_at>") {
                    (v, n(&w["created_at"]), '>')
                } else {
                    return Err(fail("attestation", "auth-conditions"));
                };
                let value = digits
                    .parse::<u64>()
                    .map_err(|_| fail("attestation", "auth-conditions"))?;
                if digits.is_empty()
                    || (digits.len() > 1 && digits.starts_with('0'))
                    || !digits.bytes().all(|b| b.is_ascii_digit())
                    || value > if op == '=' { 65535 } else { u32::MAX as u64 }
                    || !match op {
                        '=' => actual == value,
                        '<' => actual < value,
                        _ => actual > value,
                    }
                {
                    return Err(fail("attestation", "auth-conditions"));
                }
            }
        }
    }
    Ok(Record { wire: w, body })
}
