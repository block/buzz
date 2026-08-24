//! Webhook secret management helpers.
//!
//! Secrets are stored inside the workflow definition JSON under the key
//! `"_webhook_secret"`.  This keeps the secret co-located with the definition
//! so that the `definition_hash` covers it — the hash **must** be computed
//! *after* calling `inject_secret`, otherwise the stored hash will never
//! match the stored definition.
//!
//! # Hash-ordering contract
//!
//! ```text
//! 1. Build / update the definition JSON.
//! 2. Call inject_secret(&mut def, &secret)   ← secret is now part of def
//! 3. Compute definition_hash over def        ← hash covers the secret
//! 4. Persist def + hash to the database
//! ```
//!
//! Reversing steps 2 and 3 (the previous bug) means the hash is computed over
//! a definition that does *not* yet contain `_webhook_secret`, so every
//! subsequent comparison fails.

/// Generate a new random webhook secret.
///
/// The secret is a UUID v4 rendered as a hyphenated string, which gives 122
/// bits of randomness — sufficient for an HMAC-style bearer token.
pub fn generate_webhook_secret() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Inject `secret` into `def` under the key `"_webhook_secret"`.
///
/// If `def` is not a JSON object the call is a no-op (the definition is
/// malformed and will fail validation elsewhere).
pub fn inject_secret(def: &mut serde_json::Value, secret: &str) {
    if let Some(map) = def.as_object_mut() {
        map.insert(
            "_webhook_secret".to_string(),
            serde_json::Value::String(secret.to_string()),
        );
    }
}

/// Extract the webhook secret from `def`, if present.
///
/// Returns `None` when the key is absent or its value is not a string.
pub fn extract_secret(def: &serde_json::Value) -> Option<String> {
    def.get("_webhook_secret")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Return a copy of `def` with `"_webhook_secret"` removed.
///
/// Use this before returning a definition to API callers — the secret must
/// never be embedded in a response body (it is returned once, at creation
/// time, via a dedicated `webhook_secret` field).
pub fn strip_secret(def: &serde_json::Value) -> serde_json::Value {
    match def.as_object() {
        Some(map) => {
            let filtered: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .filter(|(k, _)| k.as_str() != "_webhook_secret")
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            serde_json::Value::Object(filtered)
        }
        None => def.clone(),
    }
}

/// Apply webhook-secret projection semantics before hashing and persistence.
///
/// Existing webhook workflows retain their secret. Callers that can securely
/// return a newly minted secret may set `allow_generate`; background owner
/// command completion must not mint an undiscoverable credential.
pub fn prepare_definition(
    definition: &mut serde_json::Value,
    existing: Option<&serde_json::Value>,
    is_webhook: bool,
    allow_generate: bool,
) -> Result<Option<String>, &'static str> {
    if !is_webhook {
        return Ok(None);
    }
    if let Some(secret) = existing.and_then(extract_secret) {
        inject_secret(definition, &secret);
        return Ok(None);
    }
    if !allow_generate {
        return Err("webhook_secret_unavailable");
    }
    let secret = generate_webhook_secret();
    inject_secret(definition, &secret);
    Ok(Some(secret))
}

/// Compare `provided` against `stored` in constant time.
///
/// Returns `true` only when the two strings are identical.  The XOR-fold
/// ensures that the comparison does not short-circuit on the first differing
/// byte, preventing timing-oracle attacks.
///
/// Note: a length mismatch is revealed immediately (not constant-time), but
/// an attacker who can observe response latency already knows the expected
/// length from the generation algorithm (UUID v4 → always 36 bytes), so
/// leaking the length check provides no additional information.
pub fn verify_secret(provided: &str, stored: &str) -> bool {
    if provided.len() != stored.len() {
        return false;
    }
    let mut result = 0u8;
    for (a, b) in provided.bytes().zip(stored.bytes()) {
        result |= a ^ b;
    }
    result == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_is_nonempty() {
        let s = generate_webhook_secret();
        assert!(!s.is_empty());
    }

    #[test]
    fn generate_is_unique() {
        let a = generate_webhook_secret();
        let b = generate_webhook_secret();
        assert_ne!(a, b);
    }

    #[test]
    fn inject_and_extract_roundtrip() {
        let mut def = serde_json::json!({"name": "my-workflow"});
        let secret = "test-secret-abc";
        inject_secret(&mut def, secret);
        assert_eq!(extract_secret(&def), Some(secret.to_string()));
    }

    #[test]
    fn inject_noop_on_non_object() {
        let mut def = serde_json::json!("not-an-object");
        inject_secret(&mut def, "secret");
        assert_eq!(extract_secret(&def), None);
    }

    #[test]
    fn strip_removes_secret() {
        let mut def = serde_json::json!({"name": "wf", "steps": []});
        inject_secret(&mut def, "supersecret");
        let stripped = strip_secret(&def);
        assert!(stripped.get("_webhook_secret").is_none());
        assert_eq!(stripped.get("name").and_then(|v| v.as_str()), Some("wf"));
    }

    #[test]
    fn strip_preserves_other_fields() {
        let def = serde_json::json!({"a": 1, "_webhook_secret": "s", "b": 2});
        let stripped = strip_secret(&def);
        assert!(stripped.get("_webhook_secret").is_none());
        assert_eq!(stripped.get("a").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(stripped.get("b").and_then(|v| v.as_i64()), Some(2));
    }

    #[test]
    fn prepare_definition_preserves_existing_secret() {
        let existing = serde_json::json!({"_webhook_secret": "keep-me"});
        let mut replacement = serde_json::json!({"name": "replacement"});
        assert_eq!(
            prepare_definition(&mut replacement, Some(&existing), true, false),
            Ok(None)
        );
        assert_eq!(extract_secret(&replacement).as_deref(), Some("keep-me"));
    }

    #[test]
    fn prepare_definition_rejects_undeliverable_new_secret() {
        let mut replacement = serde_json::json!({"name": "replacement"});
        assert_eq!(
            prepare_definition(&mut replacement, None, true, false),
            Err("webhook_secret_unavailable")
        );
        assert!(extract_secret(&replacement).is_none());
    }

    #[test]
    fn prepare_definition_generates_when_caller_can_return_secret() {
        let mut replacement = serde_json::json!({"name": "replacement"});
        let generated = prepare_definition(&mut replacement, None, true, true)
            .unwrap()
            .expect("new secret");
        assert_eq!(
            extract_secret(&replacement).as_deref(),
            Some(generated.as_str())
        );
    }

    #[test]
    fn verify_secret_matches() {
        assert!(verify_secret("hello-world", "hello-world"));
    }

    #[test]
    fn verify_secret_rejects_wrong() {
        assert!(!verify_secret("hello-world", "hello-WORLD"));
    }

    #[test]
    fn verify_secret_rejects_different_length() {
        assert!(!verify_secret("short", "longer-string"));
    }

    #[test]
    fn verify_secret_empty_strings() {
        assert!(verify_secret("", ""));
    }
}
