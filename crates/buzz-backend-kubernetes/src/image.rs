//! Image reference validation (spec §Image).
//!
//! The object holding this reference runs with an nsec, so the reference must
//! be **immutable**. Registry tags are mutable pointers — Kubernetes itself
//! distinguishes them from digests for exactly this reason — so a tag-only
//! reference is rejected, not just `:latest`.
//!
//! There is no parse-time fallback: `image` is required, and its absence
//! fails closed with a named field. The published `ghcr.io/block/buzz-sprig`
//! digest is offered only as a schema `default` (a UI prefill the desktop
//! submits explicitly — see `config::DEFAULT_IMAGE`), so the create-intent
//! fingerprint never depends on compiled-in provider state.

/// A validated, digest-qualified image reference.
///
/// The inner string is always in canonical tagless form `name@sha256:<hex>`:
/// `name:tag@sha256:…` normalizes by dropping the tag, because the tag is
/// decorative once a digest pins the content, and leaving it in would make
/// two references to identical bytes produce different create-intent
/// fingerprints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef(String);

impl ImageRef {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ImageRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Parse and normalize a user-supplied image reference.
pub fn parse(raw: &str) -> Result<ImageRef, String> {
    let reference = raw.trim();
    if reference.is_empty() {
        return Err("provider_config.image is required: no image is assumed \
                    at parse time, so the digest-pinned image to run must be \
                    given explicitly"
            .to_string());
    }

    let mut parts = reference.split('@');
    let name_and_tag = parts.next().unwrap_or_default();
    let digest = match (parts.next(), parts.next()) {
        (Some(d), None) => d,
        (None, _) => {
            return Err(format!(
                "provider_config.image {reference:?} is not digest-pinned: a \
                 tag is a mutable pointer, and this object runs with the \
                 agent's private key. Use name@sha256:<64 hex chars>"
            ))
        }
        (Some(_), Some(_)) => {
            return Err(format!(
                "provider_config.image {reference:?} contains more than one '@'"
            ))
        }
    };

    let hex = digest.strip_prefix("sha256:").ok_or_else(|| {
        format!("provider_config.image digest {digest:?} must start with 'sha256:'")
    })?;
    // Lowercase only: OCI canonicalizes digest hex, and accepting uppercase
    // would let two spellings of one digest produce two fingerprints.
    if hex.len() != 64
        || !hex
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
    {
        return Err(format!(
            "provider_config.image digest {digest:?} must be exactly 64 \
             lowercase hex characters"
        ));
    }

    // Drop any tag: `name:tag@sha256:…` and `name@sha256:…` name the same
    // bytes and must fingerprint identically. Only a *final* colon segment
    // that isn't a port counts as a tag — `host:5000/name` has no tag.
    let name = match name_and_tag.rfind(':') {
        Some(colon) if !name_and_tag[colon + 1..].contains('/') => &name_and_tag[..colon],
        _ => name_and_tag,
    };
    if name.is_empty() {
        return Err(format!(
            "provider_config.image {reference:?} has no repository name"
        ));
    }

    Ok(ImageRef(format!("{name}@sha256:{hex}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_digest_pinned_reference() {
        let d = "a".repeat(64);
        let r = parse(&format!("ghcr.io/block/buzz-sprig@sha256:{d}")).unwrap();
        assert_eq!(r.as_str(), format!("ghcr.io/block/buzz-sprig@sha256:{d}"));
    }

    /// The normalization that keeps the fingerprint stable: two spellings of
    /// the same bytes must produce one reference.
    #[test]
    fn strips_tag_from_tag_plus_digest_form() {
        let d = "b".repeat(64);
        let tagged = parse(&format!("ghcr.io/block/buzz-sprig:v1.2@sha256:{d}")).unwrap();
        let plain = parse(&format!("ghcr.io/block/buzz-sprig@sha256:{d}")).unwrap();
        assert_eq!(tagged, plain);
    }

    /// A registry port is not a tag. `host:5000/name` must keep its port.
    #[test]
    fn registry_port_is_not_mistaken_for_a_tag() {
        let d = "c".repeat(64);
        let r = parse(&format!("localhost:5000/buzz-sprig@sha256:{d}")).unwrap();
        assert_eq!(r.as_str(), format!("localhost:5000/buzz-sprig@sha256:{d}"));
    }

    #[test]
    fn port_and_tag_together_drops_only_the_tag() {
        let d = "d".repeat(64);
        let r = parse(&format!("localhost:5000/buzz-sprig:dev@sha256:{d}")).unwrap();
        assert_eq!(r.as_str(), format!("localhost:5000/buzz-sprig@sha256:{d}"));
    }

    /// Wren's amendment: *every* tag-only reference is rejected, not just
    /// `:latest`. A `sha-<gitsha>` tag is traceable but still movable.
    #[test]
    fn rejects_every_tag_only_reference() {
        for bad in [
            "ghcr.io/block/buzz-sprig:latest",
            "ghcr.io/block/buzz-sprig:v1.2.3",
            "ghcr.io/block/buzz-sprig:sha-abc1234",
            "ghcr.io/block/buzz-sprig",
            "localhost:5000/buzz-sprig",
        ] {
            let err = parse(bad).unwrap_err();
            assert!(err.contains("digest-pinned"), "for {bad:?} got: {err}");
        }
    }

    /// Uppercase hex is a second spelling of one digest; accepting it would
    /// let the same image fingerprint two ways.
    #[test]
    fn rejects_uppercase_digest_hex() {
        let d = "A".repeat(64);
        assert!(parse(&format!("img@sha256:{d}")).is_err());
    }

    #[test]
    fn rejects_malformed_digests() {
        let short = "a".repeat(63);
        let long = "a".repeat(65);
        let ok = "a".repeat(64);
        for bad in [
            format!("img@sha256:{short}"),
            format!("img@sha256:{long}"),
            format!("img@sha512:{ok}"),
            format!("img@{ok}"),
            format!("img@sha256:{}", "g".repeat(64)),
            format!("img@sha256:{ok}@sha256:{ok}"),
            format!("@sha256:{ok}"),
        ] {
            assert!(parse(&bad).is_err(), "accepted {bad:?}");
        }
    }

    /// Parsing has no fallback (the schema default is a UI prefill, not a
    /// parse-time substitute), so an absent image is an error that names the
    /// field rather than a silent fallback.
    #[test]
    fn empty_reference_names_the_field() {
        for empty in ["", "   "] {
            let err = parse(empty).unwrap_err();
            assert!(err.contains("provider_config.image"), "got: {err}");
        }
    }
}
