//! Per-invocation Git assertion configuration; never persist JWTs to git config.
use crate::{federated_identity::IdentityError, identity_adapter};

/// Add exactly one origin-scoped identity header after refreshing authority.
/// Redirects and curl tracing are disabled so credentials cannot cross origins
/// or enter Git diagnostics. The existing Nostr credential helper remains in use.
pub async fn configure(
    command: &mut std::process::Command,
    keys: &nostr::Keys,
    relay: &str,
    base: usize,
) -> Result<(), IdentityError> {
    let Some(header) = identity_adapter::environment_header(relay, keys).await? else {
        return Ok(());
    };
    let mut origin = url::Url::parse(relay).map_err(|_| IdentityError::Invalid)?;
    if origin.scheme() == "ws" {
        origin
            .set_scheme("http")
            .map_err(|_| IdentityError::Invalid)?;
    }
    if origin.scheme() == "wss" {
        origin
            .set_scheme("https")
            .map_err(|_| IdentityError::Invalid)?;
    }
    let prefix = format!("http.{}/git", origin.origin().ascii_serialization());
    let entries = [
        (format!("{prefix}.extraHeader"), String::new()),
        (
            format!("{prefix}.extraHeader"),
            format!(
                "Nostr-Federated-Identity: {}",
                header.to_str().map_err(|_| IdentityError::Invalid)?
            ),
        ),
        ("http.followRedirects".into(), "false".into()),
    ];
    command.env("GIT_CONFIG_COUNT", (base + entries.len()).to_string());
    for (offset, (key, value)) in entries.into_iter().enumerate() {
        command.env(format!("GIT_CONFIG_KEY_{}", base + offset), key);
        command.env(format!("GIT_CONFIG_VALUE_{}", base + offset), value);
    }
    for key in [
        "GIT_TRACE",
        "GIT_TRACE_CURL",
        "GIT_CURL_VERBOSE",
        "GIT_TRACE2",
        "GIT_TRACE2_EVENT",
        "GIT_TRACE2_PERF",
    ] {
        command.env_remove(key);
    }
    Ok(())
}
