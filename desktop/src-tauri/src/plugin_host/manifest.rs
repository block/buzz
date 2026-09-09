//! Strict `0.1.0-alpha` browser plugin manifest parsing and validation.
//!
//! Every rule here is structural (bytes and strings only); rules that need
//! the extracted filesystem — file count, total size, symlinks, digest
//! verification against real bytes — live in `package.rs`.

use std::path::{Component, Path};
use std::sync::LazyLock;

use regex::Regex;

use crate::plugin_host::types::{InstallReject, Manifest, RuntimeTarget};

const PACKAGE_FORMAT_VERSION: &str = "0.1.0-alpha";
const CONTRACT_VERSION: &str = "0.1.0-alpha";
const RUNTIME_TYPE: &str = "stdio";
const BROWSER_GRANT: &str = "browser.browse";
const BROWSER_CONTRIBUTION_KIND: &str = "browser";
const MAX_NAME_LENGTH: usize = 64;
const MAX_PUBLISHER_LENGTH: usize = 64;
const MAX_CONTRIBUTION_TITLE_LENGTH: usize = 48;

/// Reverse-domain plugin id, per `docs/plugin-browser-prototype.md` §2.
static PLUGIN_ID_PATTERN: LazyLock<Result<Regex, regex::Error>> = LazyLock::new(|| {
    Regex::new(r"^[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)+$")
});

/// Package-local contribution id, per `docs/plugin-browser-prototype.md` §2.
static CONTRIBUTION_ID_PATTERN: LazyLock<Result<Regex, regex::Error>> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9-]{1,32}$"));

/// Full SemVer 2.0.0 grammar (semver.org Appendix A), so a leading-zero
/// component is rejected while a valid prerelease/build suffix is accepted.
/// Uses `[0-9]` rather than `\d` throughout: the `regex` crate's `\d` matches
/// any Unicode decimal digit (e.g. U+0661 ARABIC-INDIC DIGIT ONE), not just
/// ASCII, so `\d` would accept a version component SemVer itself does not.
static SEMVER_PATTERN: LazyLock<Result<Regex, regex::Error>> = LazyLock::new(|| {
    Regex::new(
        r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-((?:0|[1-9][0-9]*|[0-9]*[a-zA-Z-][0-9a-zA-Z-]*)(?:\.(?:0|[1-9][0-9]*|[0-9]*[a-zA-Z-][0-9a-zA-Z-]*))*))?(?:\+([0-9a-zA-Z-]+(?:\.[0-9a-zA-Z-]+)*))?$",
    )
});

/// Rejects a compound SPDX license *expression* (`AND`/`OR`/`WITH`,
/// parentheses) before the real identifier check below runs. The manifest
/// field is a single SPDX identifier per `docs/plugin-browser-prototype.md`
/// §2, not an expression, so this only needs to catch the presence of
/// expression syntax and whitespace; it does not need to validate identifier
/// characters itself — `spdx::license_id` does that against the real SPDX
/// license list.
static LICENSE_EXPRESSION_CHARACTERS: LazyLock<Result<Regex, regex::Error>> =
    LazyLock::new(|| Regex::new(r"[\s()]"));

/// Every pattern above is a hardcoded literal covered by this module's own
/// tests, so `Regex::new` failing for one is not expected to happen in
/// practice — but a static initializer that panicked on that failure would
/// still crash every caller of [`validate`] rather than surface a typed
/// error, so compilation failure is instead threaded through as an
/// [`InstallReject::InvalidManifest`].
fn compiled<'a>(
    pattern: &'a LazyLock<Result<Regex, regex::Error>>,
    label: &str,
) -> Result<&'a Regex, InstallReject> {
    pattern.as_ref().map_err(|error| {
        InstallReject::InvalidManifest(format!("internal error: {label} pattern: {error}"))
    })
}

/// Parses manifest JSON bytes and validates every structural rule.
///
/// `serde`'s `deny_unknown_fields` on every manifest struct rejects an
/// unrecognized key at any level as a parse error, folded into
/// [`InstallReject::InvalidManifest`] below.
pub fn parse_and_validate(bytes: &[u8]) -> Result<Manifest, InstallReject> {
    let manifest: Manifest = serde_json::from_slice(bytes)
        .map_err(|error| InstallReject::InvalidManifest(error.to_string()))?;
    validate(&manifest)?;
    Ok(manifest)
}

fn validate(manifest: &Manifest) -> Result<(), InstallReject> {
    if manifest.package_format_version != PACKAGE_FORMAT_VERSION {
        return Err(InstallReject::InvalidManifest(format!(
            "unsupported packageFormatVersion {:?}",
            manifest.package_format_version
        )));
    }
    if manifest.contract_version != CONTRACT_VERSION {
        return Err(InstallReject::InvalidManifest(format!(
            "unsupported contractVersion {:?}",
            manifest.contract_version
        )));
    }
    if !compiled(&PLUGIN_ID_PATTERN, "plugin id")?.is_match(&manifest.id) {
        return Err(InstallReject::InvalidManifest(format!(
            "id {:?} is not a reverse-domain identifier",
            manifest.id
        )));
    }
    if manifest.name.is_empty() || manifest.name.chars().count() > MAX_NAME_LENGTH {
        return Err(InstallReject::InvalidManifest(format!(
            "name must be 1-{MAX_NAME_LENGTH} characters"
        )));
    }
    if !compiled(&SEMVER_PATTERN, "semver")?.is_match(&manifest.version) {
        return Err(InstallReject::InvalidManifest(format!(
            "version {:?} is not a valid semantic version",
            manifest.version
        )));
    }
    if manifest.publisher.is_empty() || manifest.publisher.chars().count() > MAX_PUBLISHER_LENGTH {
        return Err(InstallReject::InvalidManifest(format!(
            "publisher must be 1-{MAX_PUBLISHER_LENGTH} characters"
        )));
    }
    if compiled(&LICENSE_EXPRESSION_CHARACTERS, "license expression")?.is_match(&manifest.license)
        || spdx::license_id(&manifest.license).is_none()
    {
        return Err(InstallReject::InvalidManifest(format!(
            "license {:?} is not a valid SPDX license identifier",
            manifest.license
        )));
    }
    if let Some(homepage) = &manifest.homepage {
        if !is_https_url(homepage) {
            return Err(InstallReject::InvalidManifest(
                "homepage must be a valid https URL".into(),
            ));
        }
    }
    if manifest.runtime.runtime_type != RUNTIME_TYPE {
        return Err(InstallReject::InvalidManifest(format!(
            "unsupported runtime type {:?}",
            manifest.runtime.runtime_type
        )));
    }
    if manifest.runtime.targets.is_empty() {
        return Err(InstallReject::InvalidManifest(
            "runtime.targets is empty".into(),
        ));
    }
    for (triple, target) in &manifest.runtime.targets {
        validate_target(triple, target)?;
    }
    if manifest.grants.len() != 1 || manifest.grants[0] != BROWSER_GRANT {
        return Err(InstallReject::InvalidManifest(format!(
            "grants must be exactly [{BROWSER_GRANT:?}]"
        )));
    }
    if manifest.contributions.len() != 1 {
        return Err(InstallReject::InvalidManifest(
            "manifest must declare exactly one contribution".into(),
        ));
    }
    let contribution = &manifest.contributions[0];
    if contribution.kind != BROWSER_CONTRIBUTION_KIND {
        return Err(InstallReject::InvalidManifest(format!(
            "unsupported contribution kind {:?}",
            contribution.kind
        )));
    }
    if !compiled(&CONTRIBUTION_ID_PATTERN, "contribution id")?.is_match(&contribution.id) {
        return Err(InstallReject::InvalidManifest(format!(
            "contribution id {:?} must match ^[a-z0-9-]{{1,32}}$",
            contribution.id
        )));
    }
    if contribution.title.is_empty()
        || contribution.title.chars().count() > MAX_CONTRIBUTION_TITLE_LENGTH
    {
        return Err(InstallReject::InvalidManifest(format!(
            "contribution title must be 1-{MAX_CONTRIBUTION_TITLE_LENGTH} characters"
        )));
    }
    Ok(())
}

fn validate_target(triple: &str, target: &RuntimeTarget) -> Result<(), InstallReject> {
    if triple.trim().is_empty() {
        return Err(InstallReject::InvalidManifest("empty target triple".into()));
    }
    if target.path.trim().is_empty() {
        return Err(InstallReject::InvalidManifest(format!(
            "empty path for target {triple:?}"
        )));
    }
    if !is_safe_relative_path(&target.path) {
        return Err(InstallReject::InvalidManifest(format!(
            "target {triple:?} path must be a safe package-relative path"
        )));
    }
    if !is_lowercase_hex_sha256(&target.sha256) {
        return Err(InstallReject::InvalidManifest(format!(
            "sha256 for target {triple:?} is not 64 lowercase hex characters"
        )));
    }
    Ok(())
}

/// Rejects an absolute path and any `..` segment; only a package-relative,
/// downward path is safe to join under the staged package root.
fn is_safe_relative_path(path: &str) -> bool {
    let candidate = Path::new(path);
    if candidate.is_absolute() {
        return false;
    }
    candidate
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
        && candidate.components().next().is_some()
}

fn is_lowercase_hex_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Parses `value` as an absolute URL (RFC 3986, via the `url` crate) and
/// requires an `https` scheme with a host, rather than approximating with a
/// string-prefix check that would accept a scheme-confusable string like
/// `https://.` or reject a syntactically valid URL with unusual but legal
/// components.
fn is_https_url(value: &str) -> bool {
    url::Url::parse(value).is_ok_and(|url| url.scheme() == "https" && url.host().is_some())
}

#[cfg(test)]
#[path = "manifest_tests.rs"]
mod tests;
