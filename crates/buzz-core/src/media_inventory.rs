//! Conservative inventory extraction, separate from read authorization.
//! Free-text sightings do not grant media access or prove upload ownership.
use serde_json::Value;
use std::collections::BTreeSet;

/// Extract local media hashes with JSON-pointer-like field paths. Nested JSON
/// strings and animated-avatar descriptors are decoded. Resource/format errors
/// must not be interpreted as an empty reference set by the caller.
pub fn references(value: &Value, bases: &[String]) -> Result<BTreeSet<(String, String)>, String> {
    let mut out = BTreeSet::new();
    visit(value, Some(bases), "", 0, &mut 0, &mut out)?;
    Ok(out)
}

/// Extract file hashes for write exclusion, including absolute URL aliases.
/// Shares the inventory parser, but accepts any HTTP origin and drops evidence
/// paths. Public keys and event IDs in ordinary tags are not file references.
pub fn write_hashes(value: &Value) -> Result<BTreeSet<String>, String> {
    let mut out = BTreeSet::new();
    visit(value, None, "", 0, &mut 0, &mut out)?;
    Ok(out.into_iter().map(|(hash, _)| hash).collect())
}

fn visit(
    value: &Value,
    bases: Option<&[String]>,
    path: &str,
    depth: usize,
    work: &mut usize,
    out: &mut BTreeSet<(String, String)>,
) -> Result<(), String> {
    if depth > 32 || out.len() > 10_000 {
        return Err("media reference parsing limit exceeded".into());
    }
    match value {
        Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                visit(v, bases, &format!("{path}/{i}"), depth + 1, work, out)?;
            }
        }
        Value::Object(items) => {
            for (k, v) in items {
                visit(
                    v,
                    bases,
                    &format!("{path}/{}", k.replace('~', "~0").replace('/', "~1")),
                    depth + 1,
                    work,
                    out,
                )?;
            }
        }
        Value::String(s) => {
            *work = work.saturating_add(s.len());
            if *work > 128 * 1024 * 1024 {
                return Err("media reference parsing byte budget exceeded".into());
            }
            // JSON embedded in event content is commonplace. Decode before URL tokenization.
            if (s.trim_start().starts_with('{') || s.trim_start().starts_with('['))
                && (bases.is_none() || s.len() <= 1_048_576)
            {
                if let Ok(decoded) = serde_json::from_str::<Value>(s) {
                    visit(
                        &decoded,
                        bases,
                        &format!("{path}/json"),
                        depth + 1,
                        work,
                        out,
                    )?;
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
                if let Some((poster, animation)) = crate::media::animated_avatar_urls(token) {
                    for (suffix, url) in [("poster", poster), ("animation", animation)] {
                        add_url(&url, bases, &format!("{path}/{suffix}"), out);
                    }
                } else {
                    // Match avatar consumers: an invalid animation fragment
                    // leaves only its poster URL. Prose mentioning the fragment
                    // syntax does not turn an otherwise readable source into an error.
                    add_url(token, bases, path, out);
                }
            }
            if let Some(hash) = s.strip_prefix("x ").filter(|h| valid_hash(h)) {
                out.insert((
                    hash.into(),
                    if bases.is_some() {
                        path.into()
                    } else {
                        String::new()
                    },
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn add_url(url: &str, bases: Option<&[String]>, path: &str, out: &mut BTreeSet<(String, String)>) {
    // Every accepted path contains a slash and a full 64-byte hash. Most
    // message tokens cannot be media URLs; avoid allocating three prefixes per
    // community for each of those ordinary words during a whole-database scan.
    if url.len() < 65 || !url.contains('/') {
        return;
    }
    let url = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim_end_matches('.');
    if let Some(bases) = bases {
        // Discovery only attributes URLs in this storage namespace.
        for base in bases {
            if let Some(hash) = crate::media::local_hash(url, base) {
                out.insert((hash, path.into()));
                break;
            }
        }
    } else if url.starts_with("/media/")
        || url.starts_with("https://")
        || url.starts_with("http://")
    {
        // A writer must also cover configured aliases without an unbounded
        // community-host lookup on every event. Foreign URL matches can only
        // conservatively reject use of a currently reserved content hash.
        if let Some((base, _)) = url.rsplit_once('/') {
            if let Some(hash) = crate::media::local_hash(url, base) {
                out.insert((hash, String::new()));
            }
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
    fn write_guard_decodes_large_embedded_json_and_ignores_public_keys() {
        let hash = "a".repeat(64);
        let escaped = "\\u0061".repeat(64);
        let content = format!(
            r#"{{"padding":"{}","url":"/media/{}.jpg"}}"#,
            " ".repeat(1_048_576),
            escaped
        );
        let value = serde_json::json!({"content": content});
        assert_eq!(
            write_hashes(&value).unwrap(),
            BTreeSet::from([hash.clone()])
        );
        let tags: Vec<_> = (0..12_000)
            .map(|_| serde_json::json!(["p", hash]))
            .collect();
        assert!(write_hashes(&serde_json::json!({"tags": tags}))
            .unwrap()
            .is_empty());
        assert!(
            write_hashes(&serde_json::json!("x".repeat(16 * 1024 * 1024)))
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn attachments_nested_json_animation_and_foreign_hosts() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let c = "c".repeat(64);
        let value = serde_json::json!({"tags":[["imeta",format!("url /media/{a}.jpg")]],"content":serde_json::json!({"picture":format!("https://example.org/media/{b}.jpg#buzz-anim=https%3A%2F%2Fexample.org%2Fmedia%2F{c}.gif")}).to_string(),"external":format!("https://elsewhere.org/media/{}.png","d".repeat(64))});
        let refs = references(&value, &["https://example.org/media".into()]).unwrap();
        let hashes: BTreeSet<_> = refs.into_iter().map(|(h, _)| h).collect();
        assert_eq!(hashes, BTreeSet::from([a, b, c]));
        let malformed = serde_json::json!(format!(
            "Syntax: #buzz-anim=<url>. Poster: https://example.org/media/{}.jpg#buzz-anim=%QQ",
            "e".repeat(64)
        ));
        let refs = references(&malformed, &["https://example.org/media".into()]).unwrap();
        assert_eq!(
            refs.into_iter()
                .map(|(hash, _)| hash)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["e".repeat(64)])
        );
        assert_eq!(
            write_hashes(&malformed).unwrap(),
            BTreeSet::from(["e".repeat(64)])
        );
    }
}
