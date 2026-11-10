//! Conservative inventory extraction, separate from read authorization.
//! Free-text sightings do not grant media access or prove upload ownership.
use serde_json::Value;
use std::collections::BTreeSet;

/// Extract local media hashes with JSON-pointer-like field paths. Nested JSON
/// strings and animated-avatar descriptors are decoded. Resource/format errors
/// must not be interpreted as an empty reference set by the caller.
pub fn references(value: &Value, bases: &[String]) -> Result<BTreeSet<(String, String)>, String> {
    let mut out = BTreeSet::new();
    visit(value, bases, "", 0, &mut out)?;
    Ok(out)
}

fn visit(
    value: &Value,
    bases: &[String],
    path: &str,
    depth: usize,
    out: &mut BTreeSet<(String, String)>,
) -> Result<(), String> {
    if depth > 32 || out.len() > 10_000 {
        return Err("media reference parsing limit exceeded".into());
    }
    match value {
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                visit(v, bases, &format!("{path}/{i}"), depth + 1, out)?;
            }
        }
        Value::Object(items) => {
            for (k, v) in items {
                visit(
                    v,
                    bases,
                    &format!("{path}/{}", k.replace('~', "~0").replace('/', "~1")),
                    depth + 1,
                    out,
                )?;
            }
        }
        Value::String(s) => {
            // JSON embedded in event content is commonplace. Decode before URL tokenization.
            if (s.trim_start().starts_with('{') || s.trim_start().starts_with('['))
                && s.len() <= 1_048_576
            {
                if let Ok(decoded) = serde_json::from_str::<Value>(s) {
                    visit(&decoded, bases, &format!("{path}/json"), depth + 1, out)?;
                }
            }
            for token in s.split(|c: char| {
                c.is_whitespace()
                    || matches!(
                        c,
                        '"' | '\'' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | '`' | '\\'
                    )
            }) {
                let token = token.trim_end_matches([',', ';']);
                if token.contains("#buzz-anim=") {
                    let (poster, animation) = crate::media::animated_avatar_urls(token)
                        .ok_or_else(|| "invalid animated-avatar descriptor".to_string())?;
                    for (suffix, url) in [("poster", poster), ("animation", animation)] {
                        add_url(&url, bases, &format!("{path}/{suffix}"), out);
                    }
                } else {
                    add_url(token, bases, path, out);
                }
            }
            if let Some(hash) = s.strip_prefix("x ").filter(|h| valid_hash(h)) {
                out.insert((hash.into(), path.into()));
            }
        }
        _ => {}
    }
    Ok(())
}

fn add_url(url: &str, bases: &[String], path: &str, out: &mut BTreeSet<(String, String)>) {
    let url = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim_end_matches('.');
    // Always validate absolute URLs against configured/local hosts. A foreign
    // /media path is not an object in this namespace.
    for base in bases {
        if let Some(hash) = crate::media::local_hash(url, base) {
            out.insert((hash, path.into()));
            break;
        }
    }
}

/// Whether a value is a canonical file hash.
pub fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn attachments_nested_json_animation_and_foreign_hosts() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let c = "c".repeat(64);
        let value = serde_json::json!({"tags":[["imeta",format!("url /media/{a}.jpg")]],"content":serde_json::json!({"picture":format!("https://example.org/media/{b}.jpg#buzz-anim=https%3A%2F%2Fexample.org%2Fmedia%2F{c}.gif")}).to_string(),"external":format!("https://elsewhere.org/media/{}.png","d".repeat(64))});
        let refs = references(&value, &["https://example.org/media".into()]).unwrap();
        let hashes: BTreeSet<_> = refs.into_iter().map(|(h, _)| h).collect();
        assert_eq!(hashes, BTreeSet::from([a, b, c]));
        assert!(references(
            &serde_json::json!("https://example.org/media/a#buzz-anim=%QQ"),
            &[]
        )
        .is_err());
    }
}
