//! Wire types shared by every op: the provider config, the `info` schema, and
//! the secret wrapper that keeps credentials out of logs.

use std::fmt;

use serde::Deserialize;
use zeroize::Zeroizing;

/// A credential that must never reach a log, an error string, or a process
/// argument list.
///
/// `Debug`/`Display` render `[REDACTED]` — the inner value is reachable only
/// through [`Secret::expose`], which every call site must name explicitly. The
/// backing buffer is zeroized on drop.
#[derive(Clone, Default)]
pub struct Secret(Zeroizing<String>);

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self(Zeroizing::new(String::deserialize(deserializer)?)))
    }
}

impl Secret {
    /// Yield the raw credential. Only legal on the path that writes it to the
    /// SSH stdin channel.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}

/// An op failure, plus the optional machine-readable recovery the desktop needs
/// to offer the user a way out.
///
/// A struct rather than an error enum: exactly one failure in this crate has a
/// recovery, and an enum would make every unremarkable `format!` in five files
/// name a variant. The `From<String>`/`From<&str>` conversions keep every
/// existing `?` compiling; there is deliberately **no** `From<Failure> for
/// String`, which would silently drop `auth_url` at the first caller that
/// propagated one.
#[derive(Debug)]
pub struct Failure {
    pub message: String,
    /// A `https://login.tailscale.com/a/…` URL built by
    /// [`crate::tailscale::auth_url_in`]. Never anything else.
    pub auth_url: Option<String>,
}

impl Failure {
    /// The tailnet's ACL asks for a browser re-auth (Tailscale SSH's `check`
    /// action), which `BatchMode` cannot answer.
    ///
    /// The message stands alone: a desktop too old to read `recovery` still
    /// tells the user what happened and where to go.
    pub fn tailscale_auth(url: String) -> Self {
        Self {
            message: format!("this host requires Tailscale SSH authentication in a browser: {url}"),
            auth_url: Some(url),
        }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self {
            message,
            auth_url: None,
        }
    }
}

impl From<&str> for Failure {
    fn from(message: &str) -> Self {
        Self::from(message.to_string())
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// Strip credential-shaped tokens out of text that came from somewhere we do
/// not control (remote stderr, mostly). Mirrors the desktop's own prefix rule
/// in `managed_agents::backend::redact_secrets_with` so a leak needs two
/// independent failures rather than one.
pub fn redact(text: &str) -> String {
    let mut out = text.to_string();
    for prefix in ["nsec1", "sprt_tok_"] {
        while let Some(start) = out.find(prefix) {
            let end = out[start..]
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .map(|offset| start + offset)
                .unwrap_or(out.len());
            out.replace_range(start..end, "[REDACTED]");
        }
    }
    out
}

/// Truncate remote output to something an error string can carry.
pub fn snippet(text: &str) -> String {
    let redacted = redact(text.trim());
    match redacted.char_indices().nth(2048) {
        Some((cut, _)) => format!("{}…", &redacted[..cut]),
        None => redacted,
    }
}

/// The provider's `provider_config` block.
///
/// Field names are constrained by the desktop's `validate_provider_config`,
/// which rejects any key whose word-split contains `secret`/`password`/
/// `token`/`key`/`credential`. That is why the identity field is
/// `ssh_identity_file` and not `ssh_key_path`: the latter would be dropped on
/// the way in, silently. `schema_keys_are_accepted_by_the_desktop_validator`
/// pins this.
#[derive(Debug, Clone, Default)]
pub struct SshConfig {
    pub host: String,
    pub user: Option<String>,
    pub port: Option<u16>,
    pub identity_file: Option<String>,
    pub buzz_acp_path: Option<String>,
}

impl SshConfig {
    pub fn from_request(request: &serde_json::Value) -> Result<Self, String> {
        let config = request
            .get("provider_config")
            .ok_or("request is missing 'provider_config'")?;

        let string = |key: &str| {
            config
                .get(key)
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };

        let host = string("ssh_host").ok_or("'ssh_host' is required")?;
        // `ssh` reads a leading dash as an option, and whitespace would split
        // the argument. Neither can appear in a real hostname.
        if host.starts_with('-') || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(format!("'ssh_host' is not a valid host: {host:?}"));
        }

        let port = match config.get("ssh_port") {
            Some(serde_json::Value::Number(n)) => n.as_u64(),
            Some(serde_json::Value::String(s)) if !s.trim().is_empty() => Some(
                s.trim()
                    .parse::<u64>()
                    .map_err(|_| format!("'ssh_port' is not a port number: {:?}", s.trim()))?,
            ),
            _ => None,
        };
        let port = match port {
            Some(p) if (1..=65535).contains(&p) => Some(p as u16),
            Some(p) => return Err(format!("'ssh_port' is out of range: {p}")),
            None => None,
        };

        Ok(Self {
            host,
            user: string("ssh_user"),
            port,
            identity_file: string("ssh_identity_file"),
            buzz_acp_path: string("buzz_acp_path"),
        })
    }

    /// The `[user@]host` argument handed to `ssh`.
    pub fn target(&self) -> String {
        match &self.user {
            Some(user) if !self.host.contains('@') => format!("{user}@{}", self.host),
            _ => self.host.clone(),
        }
    }

    /// The host without any `user@` prefix, for tailnet membership lookups.
    pub fn bare_host(&self) -> &str {
        self.host
            .rsplit_once('@')
            .map_or(self.host.as_str(), |(_, host)| host)
    }
}

/// The `info` response, including the Tailscale-decorated config schema.
///
/// When Tailscale is absent, logged out, or has no usable peers, `ssh_host`
/// carries no `oneOf` and the desktop renders exactly the plain text field it
/// renders today. That degradation is structural, not a feature flag.
pub fn info_response() -> serde_json::Value {
    let mut ssh_host = serde_json::json!({
        "type": "string",
        "title": "Server",
        "description": "hostname, IP, or user@host",
    });
    let devices = crate::tailscale::Tailnet::detect().schema_options();
    if !devices.is_empty() {
        ssh_host["oneOf"] = serde_json::Value::Array(devices);
    }

    serde_json::json!({
        "ok": true,
        "name": "SSH",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Run agents on a remote host over SSH, supervised by systemd --user.",
        "config_schema": {
            "type": "object",
            "required": ["ssh_host"],
            "properties": {
                "ssh_host": ssh_host,
                "ssh_user": { "type": "string", "title": "User" },
                "ssh_port": { "type": "integer", "title": "Port", "default": 22 },
                "ssh_identity_file": {
                    "type": "string",
                    "title": "SSH identity file (optional)",
                    "description": "Defaults to your ~/.ssh/config and agent",
                },
                "buzz_acp_path": {
                    "type": "string",
                    "title": "buzz-acp path on the server (optional)",
                    "description": "Defaults to whatever is on the server's PATH",
                },
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors `validate_provider_config` (`backend.rs`): a config key whose
    /// word-split contains a forbidden word is rejected *by the desktop*, which
    /// would silently drop the field. Reimplemented here so the schema cannot
    /// drift into a name the desktop refuses to forward.
    fn desktop_rejects_key(key: &str) -> bool {
        let mut words = Vec::new();
        let mut current = String::new();
        let chars: Vec<char> = key.chars().collect();
        for (i, &ch) in chars.iter().enumerate() {
            if ch == '_' || ch == '-' || ch == '.' {
                if !current.is_empty() {
                    words.push(current.to_lowercase());
                    current.clear();
                }
            } else if ch.is_uppercase() {
                let prev_lower = current.chars().last().is_some_and(char::is_lowercase);
                let acronym_end = current.chars().last().is_some_and(char::is_uppercase)
                    && chars.get(i + 1).is_some_and(|c| c.is_lowercase());
                if prev_lower || acronym_end {
                    words.push(current.to_lowercase());
                    current.clear();
                }
                current.push(ch);
            } else {
                current.push(ch);
            }
        }
        if !current.is_empty() {
            words.push(current.to_lowercase());
        }
        ["secret", "password", "token", "key", "credential"]
            .iter()
            .any(|forbidden| words.iter().any(|word| word == forbidden))
    }

    #[test]
    fn schema_keys_are_accepted_by_the_desktop_validator() {
        let info = info_response();
        let properties = info["config_schema"]["properties"].as_object().unwrap();
        assert!(properties.contains_key("ssh_identity_file"));
        for key in properties.keys() {
            assert!(
                !desktop_rejects_key(key),
                "config key {key:?} would be rejected by validate_provider_config"
            );
        }
        // The name this field must never have.
        assert!(desktop_rejects_key("ssh_key_path"));
    }

    #[test]
    fn info_schema_has_no_one_of_without_tailscale_devices() {
        // Nothing in this test environment advertises a tailnet, so the schema
        // must degrade to the plain text field.
        let info = info_response();
        let host = &info["config_schema"]["properties"]["ssh_host"];
        if crate::tailscale::Tailnet::detect()
            .schema_options()
            .is_empty()
        {
            assert!(host.get("oneOf").is_none());
        }
    }

    #[test]
    fn secret_never_renders_its_value() {
        let secret: Secret = serde_json::from_str("\"nsec1verysecretvalue\"").unwrap();
        assert_eq!(format!("{secret:?}"), "[REDACTED]");
        assert_eq!(format!("{secret}"), "[REDACTED]");
        assert_eq!(secret.expose(), "nsec1verysecretvalue");
        assert!(!secret.is_empty());
    }

    #[test]
    fn redact_strips_credential_prefixes() {
        let out = redact("failed: nsec1abc123 and sprt_tok_xyz789 rejected");
        assert!(!out.contains("nsec1abc123"));
        assert!(!out.contains("sprt_tok_xyz789"));
        assert_eq!(out, "failed: [REDACTED] and [REDACTED] rejected");
    }

    #[test]
    fn config_parses_port_from_number_or_string() {
        let request = serde_json::json!({
            "provider_config": { "ssh_host": "vps", "ssh_user": "ubuntu", "ssh_port": 2222 }
        });
        let config = SshConfig::from_request(&request).unwrap();
        assert_eq!(config.port, Some(2222));
        assert_eq!(config.target(), "ubuntu@vps");

        let request = serde_json::json!({
            "provider_config": { "ssh_host": "vps", "ssh_port": "2222" }
        });
        assert_eq!(SshConfig::from_request(&request).unwrap().port, Some(2222));
    }

    #[test]
    fn config_rejects_hosts_ssh_would_read_as_options() {
        for host in ["-oProxyCommand=touch /tmp/pwn", "vps example", ""] {
            let request = serde_json::json!({ "provider_config": { "ssh_host": host } });
            assert!(
                SshConfig::from_request(&request).is_err(),
                "accepted {host:?}"
            );
        }
    }

    #[test]
    fn config_keeps_an_inline_user_over_the_user_field() {
        let request = serde_json::json!({
            "provider_config": { "ssh_host": "root@vps", "ssh_user": "ubuntu" }
        });
        let config = SshConfig::from_request(&request).unwrap();
        assert_eq!(config.target(), "root@vps");
        assert_eq!(config.bare_host(), "vps");
    }
}
