use chrono::Utc;

pub fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

/// Deserialize an `Option<Option<T>>` field that distinguishes an absent key
/// from an explicit JSON `null`.
///
/// Plain serde collapses a present `null` into the outer `None`, making
/// "clear this field" indistinguishable from "leave it unchanged". Paired with
/// `#[serde(default)]`, this yields the tri-state needed for nullable patches:
/// absent → `None`, `null` → `Some(None)`, value → `Some(Some(value))`.
pub fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: serde::Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
}

/// Turn a human-readable name into a filesystem-safe slug.
///
/// Non-alphanumeric characters become hyphens, leading/trailing hyphens are
/// stripped, and the result is capped at `max_len` characters (on a hyphen
/// boundary when possible). Returns `fallback` when the input produces an
/// empty slug.
pub fn slugify(name: &str, fallback: &str, max_len: usize) -> String {
    let raw: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let raw = if raw.is_empty() { fallback } else { &raw };
    let raw = if raw.len() > max_len {
        &raw[..max_len]
    } else {
        raw
    };
    raw.trim_end_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn double_option_tristate() {
        #[derive(serde::Deserialize)]
        struct P {
            #[serde(default, deserialize_with = "super::double_option")]
            ttl: Option<Option<i32>>,
        }
        let absent: P = serde_json::from_str("{}").unwrap();
        let null: P = serde_json::from_str(r#"{"ttl": null}"#).unwrap();
        let set: P = serde_json::from_str(r#"{"ttl": 3600}"#).unwrap();
        assert_eq!(absent.ttl, None);
        assert_eq!(null.ttl, Some(None));
        assert_eq!(set.ttl, Some(Some(3600)));
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("My Cool Team", "team", 50), "my-cool-team");
    }

    #[test]
    fn slugify_special_chars() {
        assert_eq!(slugify("héllo wörld!", "fallback", 50), "h-llo-w-rld");
    }

    #[test]
    fn slugify_empty_uses_fallback() {
        assert_eq!(slugify("   ", "persona", 50), "persona");
        assert_eq!(slugify("", "team", 50), "team");
    }

    #[test]
    fn slugify_truncates_at_max_len() {
        let long_name = "a]".repeat(60);
        let result = slugify(&long_name, "fallback", 10);
        assert!(result.len() <= 10);
        assert!(!result.ends_with('-'));
    }

    #[test]
    fn slugify_trims_trailing_hyphens_after_truncation() {
        // "abcde-----fghij" truncated at 10 → "abcde-----" → trimmed → "abcde"
        assert_eq!(slugify("abcde     fghij", "x", 10), "abcde");
    }
}
