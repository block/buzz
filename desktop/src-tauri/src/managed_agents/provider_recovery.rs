//! The provider failure wire type and the one recovery a provider may attach.
//!
//! Split out of `backend.rs` because this is the trust boundary between the
//! desktop and a `buzz-backend-*` subprocess it merely discovered on PATH:
//! everything here exists to decide what a provider is allowed to make the
//! desktop *do*, which is a different concern from invoking one or finding one.

/// The one control-server URL a provider is allowed to send us to, and the one
/// prefix `ProviderRecovery::from_response` will accept.
const TAILSCALE_AUTH_PREFIX: &str = "https://login.tailscale.com/a/";

/// An actionable step the user can take to clear a provider failure.
///
/// Deliberately not a free-form URL: this is the only enum, and each variant
/// names both the affordance and the destination, so adding a second one is a
/// deliberate act rather than a provider's choice at runtime.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ProviderRecovery {
    /// Open `url` in the user's browser. The URL is validated against
    /// [`TAILSCALE_AUTH_PREFIX`] before it is ever constructed.
    OpenUrl { url: String },
}

impl ProviderRecovery {
    /// Read the optional `recovery` key of a provider's failure response.
    ///
    /// **Fails closed and validates here, not at the opener.** The SSH provider
    /// builds this URL from a fixed prefix rather than parsing one out of
    /// remote stderr, but that guarantee does not cross the process boundary:
    /// the desktop treats a provider as a subprocess it merely discovered, not
    /// a trusted peer — which is why it re-redacts secrets on the way in on the
    /// very next line. Checking on entry rather than at `open_url` means an
    /// unvalidated URL never exists in desktop memory at all, so no later
    /// reader of the payload can become a second, unguarded way to open it.
    ///
    /// Anything unrecognized yields `None`: the failure still surfaces with its
    /// message, minus a button.
    pub(super) fn from_response(response: &serde_json::Value) -> Option<Self> {
        // `pub(super)`: `backend::invoke_provider` is the one caller, and
        // keeping it inside `managed_agents` is what stops a second, unguarded
        // construction path appearing elsewhere in the crate.
        let recovery = response.get("recovery")?;
        if recovery.get("action").and_then(|v| v.as_str()) != Some("open_url") {
            return None;
        }
        let url = recovery.get("url").and_then(|v| v.as_str())?;
        let token = url.strip_prefix(TAILSCALE_AUTH_PREFIX)?;
        // The prefix alone is not enough: `…/a/` followed by anything would
        // still be a match, and userinfo/query/fragment bytes are exactly how a
        // look-alike destination would be smuggled past a `starts_with` check.
        let unreserved = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~');
        if token.is_empty() || token.len() > 128 || !token.chars().all(unreserved) {
            return None;
        }
        Some(Self::OpenUrl {
            url: url.to_string(),
        })
    }
}

/// A provider op failure, plus the optional recovery the UI needs to offer a
/// way out. Mirrors the provider's own `Failure` type across the process
/// boundary.
///
/// A struct rather than an error enum for the same reason it is one there:
/// almost every failure in this path is an unremarkable `format!`, and an enum
/// would make each one name a variant. `From<String>`/`From<&str>` keep those
/// compiling unchanged. There is deliberately **no** `From<ProviderFailure> for
/// String` — that is the type-level guard against a caller flattening the
/// recovery away, which is exactly the bug this type exists to prevent.
///
/// It serializes as `{message, recovery?}`, which is the shape the frontend's
/// `toTauriError` already turns into a `TauriInvokeError` carrying `payload`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ProviderFailure {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<ProviderRecovery>,
}

impl From<String> for ProviderFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            recovery: None,
        }
    }
}

impl From<&str> for ProviderFailure {
    fn from(message: &str) -> Self {
        Self::from(message.to_string())
    }
}

impl std::fmt::Display for ProviderFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A provider failure response carrying the recovery shape.
    fn failure_with_recovery(url: &str) -> serde_json::Value {
        serde_json::json!({
            "ok": false,
            "error": "this host requires Tailscale SSH authentication in a browser",
            "recovery": { "action": "open_url", "url": url },
        })
    }

    #[test]
    fn a_tailscale_login_url_is_the_one_recovery_that_survives() {
        let response = failure_with_recovery("https://login.tailscale.com/a/1a2b3c4d");
        assert_eq!(
            ProviderRecovery::from_response(&response),
            Some(ProviderRecovery::OpenUrl {
                url: "https://login.tailscale.com/a/1a2b3c4d".to_string()
            })
        );
    }

    #[test]
    fn a_provider_cannot_send_the_desktop_anywhere_else() {
        // The provider is a discovered subprocess, not a trusted peer: a
        // compromised or merely buggy one must not be able to pick the
        // destination of a browser the desktop opens.
        for url in [
            "https://login.tailscale.com.evil.test/a/tok",
            "https://evil.test/https://login.tailscale.com/a/tok",
            "http://login.tailscale.com/a/tok",
            "https://user@login.tailscale.com/a/tok",
            "javascript:alert(1)",
            "file:///etc/passwd",
            // Prefix-correct but with a payload a bare `starts_with` would pass.
            "https://login.tailscale.com/a/tok?next=https://evil.test",
            "https://login.tailscale.com/a/tok#/../../evil",
            "https://login.tailscale.com/a/tok/../../evil",
            // Nothing after the marker is not a link, it is a 404.
            "https://login.tailscale.com/a/",
        ] {
            assert_eq!(
                ProviderRecovery::from_response(&failure_with_recovery(url)),
                None,
                "{url}"
            );
        }
    }

    #[test]
    fn an_unknown_or_absent_recovery_is_simply_dropped() {
        // Fails closed in every direction: the failure still surfaces with its
        // message, minus a button.
        assert_eq!(
            ProviderRecovery::from_response(&serde_json::json!({ "ok": false, "error": "no" })),
            None
        );
        assert_eq!(
            ProviderRecovery::from_response(&serde_json::json!({
                "ok": false, "error": "no",
                "recovery": { "action": "run_command", "command": "rm -rf /" },
            })),
            None
        );
        assert_eq!(
            ProviderRecovery::from_response(&serde_json::json!({
                "ok": false, "error": "no", "recovery": "https://login.tailscale.com/a/tok",
            })),
            None
        );
    }

    #[test]
    fn an_overlong_token_is_refused_rather_than_truncated() {
        let at_cap = format!("https://login.tailscale.com/a/{}", "a".repeat(128));
        assert!(ProviderRecovery::from_response(&failure_with_recovery(&at_cap)).is_some());
        let over_cap = format!("https://login.tailscale.com/a/{}", "a".repeat(129));
        assert_eq!(
            ProviderRecovery::from_response(&failure_with_recovery(&over_cap)),
            None
        );
    }

    #[test]
    fn a_failure_serializes_as_the_shape_the_frontend_reads() {
        // `toTauriError` turns this into a `TauriInvokeError` whose `payload`
        // is the object, which is what `providerRecoveryOf` reads.
        let failure = ProviderFailure {
            message: "nope".to_string(),
            recovery: Some(ProviderRecovery::OpenUrl {
                url: "https://login.tailscale.com/a/tok".to_string(),
            }),
        };
        assert_eq!(
            serde_json::to_value(&failure).unwrap(),
            serde_json::json!({
                "message": "nope",
                "recovery": { "action": "open_url", "url": "https://login.tailscale.com/a/tok" },
            })
        );
        // Without a recovery the key is absent, not null — an older frontend
        // reading `.recovery` sees undefined either way, and the wire stays the
        // same size for the overwhelmingly common failure.
        assert_eq!(
            serde_json::to_value(ProviderFailure::from("plain")).unwrap(),
            serde_json::json!({ "message": "plain" })
        );
    }
}
