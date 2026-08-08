//! Advisory parsing of the `Buzz-Client` identity header.
//!
//! Clients announce themselves with an [RFC 8941](https://www.rfc-editor.org/rfc/rfc8941)
//! structured dictionary so the relay can report which application, platform,
//! and version its live connections come from:
//!
//! ```text
//! Buzz-Client: v=1, app=buzz-desktop, platform=macos, app-version="0.5.2"
//! ```
//!
//! # This input is never trusted
//!
//! The header is unauthenticated: it arrives before NIP-42 AUTH and anyone can
//! forge it. It is therefore **advisory only** — it never participates in
//! authentication, authorization, tenant selection, rate limiting, or any
//! other decision. It feeds metrics and logs, nothing else. A missing header
//! is a normal, supported state; a malformed one is counted and discarded. No
//! input on this path can ever cause a connection to be rejected.
//!
//! # Cardinality
//!
//! `app` and `platform` are resolved to `&'static str` from closed allowlists,
//! so an unrecognized token yields no label rather than a passthrough of
//! attacker-controlled bytes. `app_version` is narrowed to `MAJOR.MINOR` with
//! each component capped at [`MAX_VERSION_COMPONENT_LEN`] digits, which still
//! leaves a large *reachable* label alphabet from a forgeable header.
//!
//! The bound that matters in practice is therefore not the alphabet but the
//! metric kind: this module emits **only a gauge**. The Prometheus recorder is
//! configured with an idle timeout for `MetricKindMask::GAUGE`
//! (`metrics::install`), so a label set that stops being emitted is dropped
//! from the registry. A burst of forged versions costs memory for one idle
//! timeout window and then self-cleans. A counter would have to be retained
//! for the process lifetime, so connection *rate* is deliberately left to the
//! existing `buzz_ws_connections_total` rather than duplicated here with a
//! forgeable label set.
//!
//! # Why a hand-written parser
//!
//! Only the tiny dictionary subset above is accepted, so a dedicated strict
//! reader is easier to audit than a general RFC 8941 implementation and costs
//! the relay no new dependency. Unknown keys are ignored rather than rejected,
//! which is what lets clients add fields (mobile already sends `app-build`,
//! `os-version`, and `os-api`) without a coordinated relay deploy.

use axum::http::HeaderMap;
use buzz_core::client_identity::{
    ClientApp, ClientPlatform, CLIENT_HEADER, CLIENT_HEADER_FORMAT_VERSION, MAX_APP_VERSION_LEN,
    MAX_HEADER_LEN,
};

/// Label value used when a dimension is unknown.
///
/// Emitting an explicit bucket rather than omitting the series puts
/// "unidentified" on a dashboard as a visible line instead of a silent gap,
/// and lets `sum(buzz_client_connections_active)` reconcile with
/// `buzz_ws_connections_active`. Used both for a wholly unidentified
/// connection and for a client that identified itself but has no real release
/// version to report (`buzz-cli` and `buzz-acp` inherit a workspace version
/// that is never bumped, so they deliberately send no `app-version`).
pub const UNKNOWN_LABEL: &str = "unknown";

/// Longest accepted `MAJOR` or `MINOR` component of `app-version`.
///
/// Five digits admits every plausible version while capping the label
/// alphabet at 10^5 values per component.
const MAX_VERSION_COMPONENT_LEN: usize = 5;

/// Why a present `Buzz-Client` header could not be used.
///
/// Rendered as a bounded `reason` label on
/// `buzz_client_header_parse_failures_total`; every variant is a fixed string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParseFailure {
    /// Not valid ASCII, or longer than [`MAX_HEADER_LEN`].
    Unreadable,
    /// Not a well-formed dictionary of the accepted subset.
    Malformed,
    /// `v` was absent or not [`CLIENT_HEADER_FORMAT_VERSION`].
    UnsupportedVersion,
    /// `app` was absent or outside the allowlist.
    UnknownApp,
    /// `platform` was absent or outside the allowlist.
    UnknownPlatform,
    /// `app-version` was present but not a bounded `MAJOR.MINOR[...]`.
    BadAppVersion,
}

impl ParseFailure {
    /// The fixed `reason` label for this failure.
    const fn as_str(self) -> &'static str {
        match self {
            Self::Unreadable => "unreadable",
            Self::Malformed => "malformed",
            Self::UnsupportedVersion => "unsupported_version",
            Self::UnknownApp => "unknown_app",
            Self::UnknownPlatform => "unknown_platform",
            Self::BadAppVersion => "bad_app_version",
        }
    }
}

/// A validated, allowlisted client identity.
///
/// Construction guarantees every field is safe to use as a metric label or log
/// field. There is no way to build one holding unvalidated input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientInfo {
    /// Allowlisted application token.
    app: &'static str,
    /// Allowlisted platform token.
    platform: &'static str,
    /// `MAJOR.MINOR`, safe as a metric label, or `None` when the client sent
    /// no version.
    app_version: Option<String>,
}

impl ClientInfo {
    /// Parse the `Buzz-Client` header, recording a failure metric when a
    /// header is present but unusable.
    ///
    /// Returns `None` both when no header was sent (silently — that is the
    /// common case for older clients and the web app) and when parsing failed
    /// (counted). Callers treat `None` as [`UNKNOWN_LABEL`].
    #[must_use]
    pub fn from_headers(headers: &HeaderMap) -> Option<Self> {
        // No header at all is normal and deliberately not counted as a
        // failure: it would otherwise fire continuously for every client that
        // predates this feature.
        let raw = headers.get(CLIENT_HEADER)?;

        match Self::parse_bytes(raw.as_bytes()) {
            Ok(info) => Some(info),
            Err(failure) => {
                metrics::counter!(
                    "buzz_client_header_parse_failures_total",
                    "reason" => failure.as_str()
                )
                .increment(1);
                None
            }
        }
    }

    /// Parse a raw header value.
    fn parse_bytes(raw: &[u8]) -> Result<Self, ParseFailure> {
        if raw.len() > MAX_HEADER_LEN {
            return Err(ParseFailure::Unreadable);
        }
        let raw = std::str::from_utf8(raw).map_err(|_| ParseFailure::Unreadable)?;
        if !raw.is_ascii() {
            return Err(ParseFailure::Unreadable);
        }
        Self::parse(raw)
    }

    /// Parse an ASCII header value of the accepted dictionary subset.
    fn parse(raw: &str) -> Result<Self, ParseFailure> {
        let mut format_version: Option<i64> = None;
        let mut app: Option<&str> = None;
        let mut platform: Option<&str> = None;
        let mut app_version: Option<&str> = None;

        for member in split_members(raw)? {
            let (key, value) = split_member(member)?;
            // Unknown keys are ignored on purpose: it lets clients add fields
            // (mobile's `app-build` / `os-version` / `os-api`) without this
            // relay having to know about them first.
            match key {
                "v" => format_version = Some(value.as_integer().ok_or(ParseFailure::Malformed)?),
                "app" => app = Some(value.as_token().ok_or(ParseFailure::Malformed)?),
                "platform" => platform = Some(value.as_token().ok_or(ParseFailure::Malformed)?),
                "app-version" => {
                    app_version = Some(value.as_string().ok_or(ParseFailure::Malformed)?);
                }
                _ => {}
            }
        }

        if format_version != Some(CLIENT_HEADER_FORMAT_VERSION) {
            return Err(ParseFailure::UnsupportedVersion);
        }

        // Resolve to the allowlist's own `&'static str`, so the label value can
        // never be attacker-controlled bytes even if they compared equal.
        let app = app
            .and_then(|candidate| ClientApp::all().iter().copied().find(|a| *a == candidate))
            .ok_or(ParseFailure::UnknownApp)?;
        let platform = platform
            .and_then(|candidate| {
                ClientPlatform::all()
                    .iter()
                    .copied()
                    .find(|p| *p == candidate)
            })
            .ok_or(ParseFailure::UnknownPlatform)?;

        // An absent `app-version` is valid: clients without an independently
        // bumped release version omit it rather than send a constant. A
        // *present* one that is unusable is still a failure, so a genuinely
        // broken version is visible rather than silently downgraded.
        let app_version = match app_version {
            Some(raw) => Some(major_minor(raw).ok_or(ParseFailure::BadAppVersion)?),
            None => None,
        };

        Ok(Self {
            app,
            platform,
            app_version,
        })
    }

    /// Allowlisted application token.
    #[must_use]
    pub const fn app(&self) -> &'static str {
        self.app
    }

    /// Allowlisted platform token.
    #[must_use]
    pub const fn platform(&self) -> &'static str {
        self.platform
    }

    /// `MAJOR.MINOR` version, or [`UNKNOWN_LABEL`] when the client sent none.
    #[must_use]
    pub fn app_version(&self) -> &str {
        self.app_version.as_deref().unwrap_or(UNKNOWN_LABEL)
    }
}

/// The per-client live-connection gauge, labeled for `info`.
///
/// This is a **separate** series from `buzz_ws_connections_active`, which is
/// consumed by the HPA as an unlabeled `AverageValue` target. Labeling that
/// gauge would shard the series the autoscaler reads; this parallel gauge
/// gives the same breakdown without touching scaling behaviour.
///
/// A gauge is the only metric kind emitted here, deliberately: the recorder
/// idle-evicts gauge series, so a forged label set cannot accumulate for the
/// process lifetime the way a counter would.
fn active_gauge(info: Option<&ClientInfo>) -> metrics::Gauge {
    let (app, platform, app_version) = match info {
        Some(info) => (info.app, info.platform, info.app_version()),
        None => (UNKNOWN_LABEL, UNKNOWN_LABEL, UNKNOWN_LABEL),
    };
    metrics::gauge!(
        "buzz_client_connections_active",
        "app" => app.to_owned(),
        "platform" => platform.to_owned(),
        "app_version" => app_version.to_owned()
    )
}

/// Add a live connection to the per-client active gauge.
pub fn increment_active(info: Option<&ClientInfo>) {
    active_gauge(info).increment(1.0);
}

/// Remove a closed connection from the per-client active gauge.
///
/// Must be paired with exactly one [`increment_active`] call, or the gauge
/// drifts.
pub fn decrement_active(info: Option<&ClientInfo>) {
    active_gauge(info).decrement(1.0);
}

/// A dictionary member's value, still in its wire form.
enum Value<'a> {
    /// An unquoted token, e.g. `buzz-desktop`.
    Token(&'a str),
    /// The contents of a quoted string, e.g. `0.5.2` from `"0.5.2"`.
    Quoted(&'a str),
}

impl<'a> Value<'a> {
    /// The value as an RFC 8941 token, or `None` if it was quoted or not a
    /// valid token.
    fn as_token(&self) -> Option<&'a str> {
        match self {
            Self::Token(token) if is_token(token) => Some(token),
            _ => None,
        }
    }

    /// The value as a quoted string's contents, or `None` if it was a token.
    fn as_string(&self) -> Option<&'a str> {
        match self {
            Self::Quoted(text) => Some(text),
            Self::Token(_) => None,
        }
    }

    /// The value as an integer, or `None` if it was quoted or not an integer.
    fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Token(token) => token.parse().ok(),
            Self::Quoted(_) => None,
        }
    }
}

/// Split a dictionary into members on commas outside quoted strings.
///
/// Splitting naively on `,` would let `app-version="1,2"` smuggle a member
/// boundary, so quotes are tracked.
fn split_members(raw: &str) -> Result<Vec<&str>, ParseFailure> {
    let mut members = Vec::new();
    let mut in_quotes = false;
    let mut escaped = false;
    let mut start = 0;

    for (idx, ch) in raw.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            // Control characters, including the CR/LF of a header-injection
            // attempt, are never valid. Tab is exempt: it is legal whitespace
            // between members and is stripped by `split_member`.
            c if c.is_ascii_control() && c != '\t' => return Err(ParseFailure::Malformed),
            ',' if !in_quotes => {
                members.push(&raw[start..idx]);
                start = idx + 1;
            }
            _ => {}
        }
    }

    if in_quotes || escaped {
        return Err(ParseFailure::Malformed);
    }
    members.push(&raw[start..]);
    Ok(members)
}

/// Split one member into its key and value.
///
/// RFC 8941 parameters (`;k=v`) are discarded: nothing in this vocabulary uses
/// them, and ignoring them keeps an unknown-parameter-bearing member usable
/// instead of failing the whole header.
fn split_member(member: &str) -> Result<(&str, Value<'_>), ParseFailure> {
    let member = member.trim_matches([' ', '\t']);
    let (key, raw_value) = member.split_once('=').ok_or(ParseFailure::Malformed)?;
    let key = key.trim_matches([' ', '\t']);
    if !is_token(key) {
        return Err(ParseFailure::Malformed);
    }

    let raw_value = raw_value.trim_matches([' ', '\t']);
    if let Some(rest) = raw_value.strip_prefix('"') {
        // Parameters may follow the closing quote; the string ends at it.
        let end = rest.find('"').ok_or(ParseFailure::Malformed)?;
        let text = &rest[..end];
        if text.contains('\\') {
            // Buzz never emits escapes, so refuse rather than implement
            // unescaping that would only ever process hostile input.
            return Err(ParseFailure::Malformed);
        }
        Ok((key, Value::Quoted(text)))
    } else {
        let value = raw_value
            .split(';')
            .next()
            .unwrap_or("")
            .trim_matches([' ', '\t']);
        if value.is_empty() {
            return Err(ParseFailure::Malformed);
        }
        Ok((key, Value::Token(value)))
    }
}

/// Whether `candidate` is a plausible RFC 8941 key or token.
///
/// Intentionally narrower than the RFC: this vocabulary only uses lowercase
/// alphanumerics with `-`, `_`, `.`, and `*`.
fn is_token(candidate: &str) -> bool {
    !candidate.is_empty()
        && candidate
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'*'))
}

/// Narrow a version to `MAJOR.MINOR`, or `None` if it is not one.
///
/// Trailing components and pre-release/build metadata are discarded so patch
/// releases do not each create a new time series. The whole string is
/// validated first so a version that is malformed only in its discarded tail
/// is still reported as a failure rather than silently accepted.
fn major_minor(version: &str) -> Option<String> {
    if !is_plausible_version(version) {
        return None;
    }
    let mut parts = version.split('.');
    let major = numeric_component(parts.next()?)?;
    let minor = numeric_component(parts.next()?)?;
    Some(format!("{major}.{minor}"))
}

/// Whether the full version string looks like a version Buzz produced.
///
/// Matches the sender-side rule in `buzz_core::client_identity`: dot-separated
/// alphanumerics with optional `-`/`+` pre-release and build metadata, within
/// the shared length bound.
fn is_plausible_version(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= MAX_APP_VERSION_LEN
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
}

/// A single bounded, all-digit version component.
///
/// Digits only: anything else (`x`, `1-rc`, whitespace) means this is not a
/// version Buzz produced, and guessing would widen the label alphabet.
fn numeric_component(component: &str) -> Option<&str> {
    if component.is_empty()
        || component.len() > MAX_VERSION_COMPONENT_LEN
        || !component.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some(component)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::client_identity::client_header_value;

    fn parse(raw: &str) -> Result<ClientInfo, ParseFailure> {
        ClientInfo::parse(raw)
    }

    #[test]
    fn parses_a_desktop_header() {
        let info = parse(r#"v=1, app=buzz-desktop, platform=macos, app-version="0.5.2""#).unwrap();
        assert_eq!(info.app(), "buzz-desktop");
        assert_eq!(info.platform(), "macos");
        assert_eq!(info.app_version(), "0.5");
    }

    #[test]
    fn accepts_a_header_with_no_app_version() {
        // buzz-cli and buzz-acp have no independently bumped release version,
        // so they omit `app-version` rather than send a misleading constant.
        // That must parse, not be counted as a failure.
        let info = parse("v=1, app=buzz-cli, platform=linux").unwrap();
        assert_eq!(info.app(), "buzz-cli");
        assert_eq!(info.platform(), "linux");
        assert_eq!(info.app_version(), UNKNOWN_LABEL);
    }

    #[test]
    fn round_trips_every_header_the_shared_builder_emits() {
        // The builder in buzz-core and this parser are a wire contract across
        // crates; assert they agree rather than restating the format.
        for app in [
            ClientApp::Desktop,
            ClientApp::Mobile,
            ClientApp::Cli,
            ClientApp::Acp,
        ] {
            for platform in [
                ClientPlatform::MacOs,
                ClientPlatform::Windows,
                ClientPlatform::Linux,
                ClientPlatform::Ios,
                ClientPlatform::Android,
            ] {
                for (version, expected) in [(Some("1.2.3"), "1.2"), (None, UNKNOWN_LABEL)] {
                    let raw = client_header_value(app, platform, version);
                    let info = parse(&raw).unwrap_or_else(|e| panic!("{raw:?} rejected as {e:?}"));
                    assert_eq!(info.app(), app.as_str());
                    assert_eq!(info.platform(), platform.as_str());
                    assert_eq!(info.app_version(), expected);
                }
            }
        }
    }

    #[test]
    fn accepts_the_mobile_header_with_its_extra_fields() {
        // Mobile (block/buzz#2596) sends app-build, os-version, and os-api.
        // Unknown keys must be ignored, not rejected, or that client would be
        // counted as unidentified.
        let info = parse(
            r#"v=1, app=buzz-mobile, platform=android, app-version="0.4.5", app-build="6", os-version="15", os-api=35"#,
        )
        .unwrap();
        assert_eq!(info.app(), "buzz-mobile");
        assert_eq!(info.platform(), "android");
        assert_eq!(info.app_version(), "0.4");
    }

    #[test]
    fn ignores_keys_it_has_never_heard_of() {
        let info = parse(r#"v=1, app=buzz-cli, platform=linux, app-version="2.0.0", future-key=7"#)
            .unwrap();
        assert_eq!(info.app_version(), "2.0");
    }

    #[test]
    fn is_order_independent() {
        let info =
            parse(r#"app-version="3.4.5", platform=windows, app=buzz-desktop, v=1"#).unwrap();
        assert_eq!(info.app(), "buzz-desktop");
        assert_eq!(info.platform(), "windows");
        assert_eq!(info.app_version(), "3.4");
    }

    #[test]
    fn narrows_versions_to_major_minor() {
        for (raw, expected) in [
            ("1.2", "1.2"),
            ("1.2.3", "1.2"),
            ("1.2.3.4", "1.2"),
            ("0.0.1", "0.0"),
            ("10.20.30", "10.20"),
        ] {
            let header = format!(r#"v=1, app=buzz-cli, platform=linux, app-version="{raw}""#);
            assert_eq!(parse(&header).unwrap().app_version(), expected, "{raw}");
        }
    }

    #[test]
    fn rejects_versions_that_would_widen_the_label_alphabet() {
        for bad in [
            "1",         // no minor
            "1.x",       // non-numeric minor
            "1.2-rc.1",  // pre-release in the minor slot
            "v1.2",      // non-numeric major
            "1.123456",  // minor over the component bound
            "123456.1",  // major over the component bound
            "",          // empty
            ".1",        // empty major
            "1.",        // empty minor
            "1. 2",      // whitespace
            "\u{7f}1.2", // control character
        ] {
            let header = format!(r#"v=1, app=buzz-cli, platform=linux, app-version="{bad}""#);
            assert!(parse(&header).is_err(), "expected {bad:?} to be refused");
        }
    }

    #[test]
    fn rejects_apps_and_platforms_outside_the_allowlist() {
        assert_eq!(
            parse(r#"v=1, app=evil-client, platform=macos, app-version="1.0.0""#),
            Err(ParseFailure::UnknownApp)
        );
        assert_eq!(
            parse(r#"v=1, app=buzz-desktop, platform=freebsd, app-version="1.0.0""#),
            Err(ParseFailure::UnknownPlatform)
        );
        // A high-cardinality forgery attempt must be refused, not passed
        // through as a label value.
        assert_eq!(
            parse(r#"v=1, app=buzz-desktop-9e1f, platform=macos, app-version="1.0.0""#),
            Err(ParseFailure::UnknownApp)
        );
    }

    #[test]
    fn rejects_an_unsupported_format_version() {
        for bad in ["v=2", "v=0", "v=99"] {
            let header = format!(r#"{bad}, app=buzz-desktop, platform=macos, app-version="1.0.0""#);
            assert_eq!(
                parse(&header),
                Err(ParseFailure::UnsupportedVersion),
                "{bad}"
            );
        }
    }

    #[test]
    fn requires_every_mandatory_field() {
        assert_eq!(
            parse(r#"app=buzz-desktop, platform=macos, app-version="1.0.0""#),
            Err(ParseFailure::UnsupportedVersion)
        );
        assert_eq!(
            parse(r#"v=1, platform=macos, app-version="1.0.0""#),
            Err(ParseFailure::UnknownApp)
        );
        assert_eq!(
            parse(r#"v=1, app=buzz-desktop, app-version="1.0.0""#),
            Err(ParseFailure::UnknownPlatform)
        );
        // `app-version` is the one optional member — absent is valid, but a
        // present-and-broken one is still a failure.
        assert!(parse("v=1, app=buzz-desktop, platform=macos").is_ok());
        assert_eq!(
            parse(r#"v=1, app=buzz-desktop, platform=macos, app-version="nope""#),
            Err(ParseFailure::BadAppVersion)
        );
    }

    #[test]
    fn rejects_structurally_broken_input() {
        for bad in [
            "",                                                              // empty
            "garbage",                                                       // no `=`
            r#"v=1, app=buzz-desktop, platform=macos, app-version="#,        // empty value
            r#"v=1, app=buzz-desktop, platform=macos, app-version="1.0"#,    // unterminated quote
            r#"v=1, app=buzz-desktop, platform=macos, ="1.0""#,              // empty key
            r#"v="1", app=buzz-desktop, platform=macos, app-version="1.0""#, // quoted v
            r#"v=1, app="buzz-desktop", platform=macos, app-version="1.0""#, // quoted app
            r#"v=1, app=buzz-desktop, platform=macos, app-version=1.0"#,     // unquoted version
        ] {
            assert!(parse(bad).is_err(), "expected {bad:?} to be refused");
        }
    }

    #[test]
    fn a_comma_inside_a_quoted_string_cannot_forge_a_member() {
        // If members were split naively on `,`, this would parse as a valid
        // header plus a smuggled `app=buzz-cli` member.
        let raw = r#"v=1, app=buzz-desktop, platform=macos, app-version="1.0.0, app=buzz-cli""#;
        assert_eq!(parse(raw), Err(ParseFailure::BadAppVersion));
    }

    #[test]
    fn rejects_control_characters_and_header_injection() {
        for bad in [
            "v=1, app=buzz-desktop, platform=macos, app-version=\"1.0.0\"\r\nX-Evil: 1",
            "v=1, app=buzz-desktop,\nplatform=macos, app-version=\"1.0.0\"",
            "v=1, app=buzz-desktop, platform=macos, app-version=\"1.0\u{0}0\"",
        ] {
            assert_eq!(parse(bad), Err(ParseFailure::Malformed), "{bad:?}");
        }
    }

    #[test]
    fn rejects_backslash_escapes_rather_than_unescaping_them() {
        let raw = r#"v=1, app=buzz-desktop, platform=macos, app-version="1.\"0""#;
        assert_eq!(parse(raw), Err(ParseFailure::Malformed));
    }

    #[test]
    fn rejects_non_ascii_and_oversized_headers() {
        assert_eq!(
            ClientInfo::parse_bytes(
                "v=1, app=buzz-desktop, platform=macos, app-version=\"1.0é\"".as_bytes()
            ),
            Err(ParseFailure::Unreadable)
        );
        let padding = "x".repeat(MAX_HEADER_LEN);
        let oversized =
            format!(r#"v=1, app=buzz-desktop, platform=macos, app-version="1.0.0", pad={padding}"#);
        assert!(oversized.len() > MAX_HEADER_LEN);
        assert_eq!(
            ClientInfo::parse_bytes(oversized.as_bytes()),
            Err(ParseFailure::Unreadable)
        );
    }

    #[test]
    fn tolerates_whitespace_variations_around_delimiters() {
        for raw in [
            r#"v=1,app=buzz-desktop,platform=macos,app-version="1.0.0""#,
            "v=1,   app=buzz-desktop,\tplatform=macos,  app-version=\"1.0.0\"",
            r#" v=1, app = buzz-desktop, platform =macos, app-version= "1.0.0" "#,
        ] {
            let info = parse(raw).unwrap_or_else(|e| panic!("{raw:?} rejected as {e:?}"));
            assert_eq!(info.app(), "buzz-desktop");
            assert_eq!(info.app_version(), "1.0");
        }
    }

    #[test]
    fn a_missing_header_is_not_a_parse_failure() {
        assert!(ClientInfo::from_headers(&HeaderMap::new()).is_none());
    }

    #[test]
    fn reads_the_header_from_a_header_map() {
        let mut headers = HeaderMap::new();
        headers.insert(
            CLIENT_HEADER,
            r#"v=1, app=buzz-cli, platform=linux, app-version="9.9.9""#
                .parse()
                .expect("valid header value"),
        );
        let info = ClientInfo::from_headers(&headers).expect("parses");
        assert_eq!(info.app(), "buzz-cli");
        assert_eq!(info.app_version(), "9.9");
    }

    #[test]
    fn unidentified_connections_get_the_unknown_bucket() {
        // An unidentified connection must still produce a series, so
        // "unidentified" is a visible dashboard line rather than a silent gap.
        let info = parse(r#"v=1, app=buzz-cli, platform=linux, app-version="1.0.0""#).unwrap();
        assert_eq!(
            (info.app(), info.platform(), info.app_version()),
            ("buzz-cli", "linux", "1.0")
        );
        // Gauge helpers accept `None` and label every dimension unknown; they
        // are exercised here to prove the label path cannot panic without a
        // recorder installed.
        increment_active(None);
        decrement_active(None);
        increment_active(Some(&info));
        decrement_active(Some(&info));
    }

    #[test]
    fn failure_reasons_are_a_small_fixed_set() {
        // These become a Prometheus label; they must stay bounded and stable.
        for (failure, expected) in [
            (ParseFailure::Unreadable, "unreadable"),
            (ParseFailure::Malformed, "malformed"),
            (ParseFailure::UnsupportedVersion, "unsupported_version"),
            (ParseFailure::UnknownApp, "unknown_app"),
            (ParseFailure::UnknownPlatform, "unknown_platform"),
            (ParseFailure::BadAppVersion, "bad_app_version"),
        ] {
            assert_eq!(failure.as_str(), expected);
        }
    }

    #[test]
    fn the_sender_cannot_emit_a_header_this_parser_would_reject_as_oversized() {
        // Both halves share `MAX_HEADER_LEN`, so anything the builder produces
        // must fit the parser's bound. A divergence here would silently count
        // real clients as parse failures.
        let longest = client_header_value(
            ClientApp::Desktop,
            ClientPlatform::Android,
            Some("10000.10000.99999-rc.1+build"),
        );
        assert!(longest.len() <= MAX_HEADER_LEN, "{longest}");
        assert!(ClientInfo::parse_bytes(longest.as_bytes()).is_ok());
    }
}
