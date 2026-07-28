//! Tailscale device enumeration, via the `tailscale` CLI.
//!
//! The CLI rather than the LocalAPI: LocalAPI means three transports (unix
//! socket on Linux, named pipe on Windows, and a localhost TCP port plus a
//! `sameuserproof` token scavenged from `/Library/Tailscale` on macOS GUI
//! builds) plus a Host-header gate that 403s on the obvious guesses. The CLI is
//! one `Command::new`, one JSON parse, one code path.
//!
//! Everything here degrades silently: Tailscale absent, logged out, or stopped
//! must leave the remote flow exactly as good as it is without Tailscale, so
//! every failure maps to "no devices" and never to an error. `tailscale status
//! --help` warns that the `--json` "format [is] subject to change", so every
//! field is optional and a parse failure is just as quiet.

use std::path::PathBuf;
use std::process::Command;

use serde::Deserialize;

/// `tailscale status --json`, reduced to the fields we consume.
#[derive(Deserialize)]
struct StatusDoc {
    #[serde(rename = "BackendState")]
    backend_state: Option<String>,
    #[serde(rename = "CurrentTailnet")]
    current_tailnet: Option<CurrentTailnet>,
    #[serde(rename = "Peer")]
    peer: Option<std::collections::BTreeMap<String, Peer>>,
}

#[derive(Deserialize)]
struct CurrentTailnet {
    #[serde(rename = "MagicDNSEnabled")]
    magic_dns_enabled: Option<bool>,
}

#[derive(Deserialize)]
struct Peer {
    #[serde(rename = "HostName")]
    host_name: Option<String>,
    #[serde(rename = "DNSName")]
    dns_name: Option<String>,
    #[serde(rename = "OS")]
    os: Option<String>,
    #[serde(rename = "Online")]
    online: Option<bool>,
    #[serde(rename = "TailscaleIPs")]
    tailscale_ips: Option<Vec<String>>,
    /// Present only when the node advertises Tailscale SSH. Absence is the
    /// negative signal: it means "not SSH-ready", not "unknown".
    #[serde(rename = "sshHostKeys")]
    ssh_host_keys: Option<Vec<String>>,
}

/// A peer that could plausibly host an agent.
pub struct Device {
    /// The address to hand to `ssh`: MagicDNS FQDN when available, else the
    /// first Tailscale IP.
    pub address: String,
    /// What the picker shows. Tailscale-SSH readiness is folded in here rather
    /// than carried as a separate field: the schema's `oneOf` entries are
    /// `{const, title}` pairs, so the label is the only channel to the user.
    pub label: String,
    /// Ordering only — the label already says it. Kept separate so sorting
    /// never has to parse its own rendering.
    online: bool,
}

/// The set of tailnet peers usable as SSH targets. Empty whenever Tailscale is
/// missing, logged out, stopped, or has nothing that can host an agent.
#[derive(Default)]
pub struct Tailnet {
    devices: Vec<Device>,
}

impl Tailnet {
    /// Run `tailscale status --json` and parse it. Never fails.
    pub fn detect() -> Self {
        let Some(binary) = cli_candidates().into_iter().find(|path| path.is_file()) else {
            return Self::default();
        };
        let mut command = Command::new(binary);
        command.arg("status").arg("--json");
        crate::ssh::configure_no_window(&mut command);
        // We never branch on the exit code: a logged-out daemon exits 0 with
        // `BackendState: "NeedsLogin"` and `Peer: null`, while a missing daemon
        // socket exits 1. `parse` handles both by looking at the document.
        match command.output() {
            Ok(output) => Self::parse(&String::from_utf8_lossy(&output.stdout)),
            Err(_) => Self::default(),
        }
    }

    /// Pure half of [`Tailnet::detect`], over the raw `--json` document.
    pub fn parse(stdout: &str) -> Self {
        let Ok(doc) = serde_json::from_str::<StatusDoc>(stdout) else {
            return Self::default();
        };
        if doc.backend_state.as_deref() != Some("Running") {
            return Self::default();
        }
        let magic_dns = doc
            .current_tailnet
            .as_ref()
            .and_then(|t| t.magic_dns_enabled)
            .unwrap_or(false);

        // `Self` is deliberately not in `Peer`, so "this computer" never shows
        // up as a remote host.
        let mut devices: Vec<Device> = doc
            .peer
            .unwrap_or_default()
            .into_values()
            .filter_map(|peer| device_from_peer(&peer, magic_dns))
            .collect();
        // Online first, then by label, so the list opens on what is usable now.
        // Sorting on the flag rather than the rendered label: a host actually
        // named "· offline" would otherwise sort itself last.
        devices.sort_by(|a, b| {
            a.online
                .cmp(&b.online)
                .reverse()
                .then_with(|| a.label.cmp(&b.label))
        });
        Self { devices }
    }

    #[cfg(test)]
    fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// True when `host` is one of the enumerated tailnet addresses.
    ///
    /// This gates `StrictHostKeyChecking=accept-new`: a tailnet address is
    /// reached over an already-WireGuard-authenticated transport, so TOFU adds
    /// nothing there. A manually typed host keeps the user's own known-hosts
    /// semantics, where an unknown key is a decision rather than a default.
    pub fn contains(&self, host: &str) -> bool {
        self.devices
            .iter()
            .any(|device| device.address.eq_ignore_ascii_case(host))
    }

    /// The `oneOf` decoration for the `ssh_host` schema property. Empty when
    /// there is nothing to offer, which drops the key entirely and leaves the
    /// desktop rendering today's plain text field.
    pub fn schema_options(&self) -> Vec<serde_json::Value> {
        self.devices
            .iter()
            .map(|device| serde_json::json!({ "const": device.address, "title": device.label }))
            .collect()
    }
}

/// The Tailscale SSH re-auth URL in a subprocess's stderr, or `None`.
///
/// A tailnet ACL with the `check` action makes `ssh` print
/// `To authenticate, visit: https://login.tailscale.com/a/<token>` and then
/// block until a human clicks it — including under `BatchMode`, which is why
/// this is detectable at all.
///
/// The result is **constructed, never parsed**: the prefix is matched
/// byte-exactly and the remainder is constrained to an unreserved-character
/// token, so no host, userinfo, scheme, query or fragment from the subprocess's
/// output can survive into the returned string. A URL scraped from a
/// subprocess and handed to the OS browser opener is an injection primitive;
/// building the answer makes that structurally impossible instead of checking
/// for it afterwards. The cost is that a custom control server (Headscale)
/// prints a different host and gets no clickable link — the right trade against
/// trusting an arbitrary host from remote output.
pub fn auth_url_in(stderr: &[u8]) -> Option<String> {
    const MARKER: &str = "https://login.tailscale.com/a/";
    /// Comfortably above the ~14-character token Tailscale prints. A longer one
    /// is dropped rather than truncated: half a URL is worse than none.
    const MAX_TOKEN: usize = 128;

    let start = stderr
        .windows(MARKER.len())
        .position(|window| window == MARKER.as_bytes())?
        + MARKER.len();
    let token: String = stderr[start..]
        .iter()
        .copied()
        .take(MAX_TOKEN + 1)
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~'))
        .map(char::from)
        .collect();
    (1..=MAX_TOKEN)
        .contains(&token.len())
        .then(|| format!("{MARKER}{token}"))
}

fn device_from_peer(peer: &Peer, magic_dns: bool) -> Option<Device> {
    let os = peer.os.as_deref().unwrap_or("");
    // Phones and TVs are tailnet members but cannot host an agent.
    if matches!(os.to_ascii_lowercase().as_str(), "ios" | "android" | "tvos") {
        return None;
    }

    let fqdn = peer
        .dns_name
        .as_deref()
        .map(|name| name.trim_end_matches('.'))
        .filter(|name| !name.is_empty());
    let ip = peer
        .tailscale_ips
        .as_ref()
        .and_then(|ips| ips.first())
        .map(String::as_str);
    // MagicDNS name when it is resolvable, else the raw tailnet IP.
    let address = if magic_dns { fqdn.or(ip) } else { ip.or(fqdn) }?.to_string();

    let name = peer
        .host_name
        .as_deref()
        .filter(|name| !name.is_empty())
        .unwrap_or(&address);
    let tailscale_ssh = peer
        .ssh_host_keys
        .as_ref()
        .is_some_and(|keys| !keys.is_empty());

    let online = peer.online.unwrap_or(false);
    let mut label = name.to_string();
    if !os.is_empty() {
        label.push_str(" — ");
        label.push_str(os);
    }
    label.push_str(if online { " · online" } else { " · offline" });
    if tailscale_ssh {
        label.push_str(" · Tailscale SSH");
    }

    Some(Device {
        address,
        label,
        online,
    })
}

/// Where the `tailscale` CLI lives when PATH does not have it. macOS GUI apps
/// inherit a minimal launchd PATH and the App Store build ships the CLI only
/// inside the bundle; Windows registers an install dir but not a PATH entry.
fn cli_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let exe = if cfg!(windows) {
        "tailscale.exe"
    } else {
        "tailscale"
    };
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join(exe)));
    }
    if cfg!(windows) {
        let program_files = std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
        candidates.push(program_files.join("Tailscale").join(exe));
    } else {
        candidates.push(PathBuf::from("/usr/bin/tailscale"));
        candidates.push(PathBuf::from("/usr/local/bin/tailscale"));
        candidates.push(PathBuf::from(
            "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
        ));
    }
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNNING: &str = r#"{
      "BackendState": "Running",
      "CurrentTailnet": { "MagicDNSEnabled": true },
      "Self": { "HostName": "vmi3160506", "OS": "linux", "Online": true },
      "Peer": {
        "nodekey:aa": { "HostName": "troys-mac-mini", "DNSName": "troys-mac-mini.tailcfd703.ts.net.",
                        "OS": "macOS", "Online": false, "TailscaleIPs": ["100.64.0.2"] },
        "nodekey:bb": { "HostName": "troys_machine", "DNSName": "troys-machine.tailcfd703.ts.net.",
                        "OS": "windows", "Online": true, "TailscaleIPs": ["100.121.179.68"] },
        "nodekey:cc": { "HostName": "localhost", "DNSName": "iphone181.tailcfd703.ts.net.",
                        "OS": "iOS", "Online": true, "TailscaleIPs": ["100.64.0.4"] },
        "nodekey:dd": { "HostName": "vps-prod", "DNSName": "vps-prod.tailcfd703.ts.net.",
                        "OS": "linux", "Online": true, "TailscaleIPs": ["100.64.0.5"],
                        "sshHostKeys": ["ssh-ed25519 AAAA"] }
      }
    }"#;

    /// A logged-out daemon exits 0. Branching on the exit code gets this wrong.
    const NEEDS_LOGIN: &str = r#"{
      "BackendState": "NeedsLogin",
      "Health": ["Tailscale is stopped."],
      "Peer": null
    }"#;

    #[test]
    fn running_tailnet_yields_hostable_peers_only() {
        let tailnet = Tailnet::parse(RUNNING);
        let addresses: Vec<&str> = tailnet
            .devices()
            .iter()
            .map(|d| d.address.as_str())
            .collect();
        // iOS is filtered out; `Self` was never a peer to begin with.
        assert_eq!(
            addresses,
            [
                "troys-machine.tailcfd703.ts.net",
                "vps-prod.tailcfd703.ts.net",
                "troys-mac-mini.tailcfd703.ts.net",
            ]
        );
        // Online first, offline last.
        assert!(tailnet.devices()[2].label.contains("· offline"));
    }

    #[test]
    fn ordering_reads_the_online_flag_not_the_rendered_label() {
        // A hostname that happens to contain the offline marker must not sort
        // itself last — the flag decides, never the label text.
        let doc = RUNNING.replace(
            "\"HostName\": \"vps-prod\"",
            "\"HostName\": \"a · offline\"",
        );
        let tailnet = Tailnet::parse(&doc);
        let devices = tailnet.devices();
        assert!(
            devices[0].label.starts_with("a · offline"),
            "{:?}",
            devices[0].label
        );
        assert!(devices[0].label.ends_with("· online · Tailscale SSH"));
        assert!(devices[2].label.contains("· offline"));
    }

    #[test]
    fn ssh_host_keys_drive_the_tailscale_ssh_marker() {
        let tailnet = Tailnet::parse(RUNNING);
        let vps = tailnet
            .devices()
            .iter()
            .find(|d| d.address.starts_with("vps-prod"))
            .unwrap();
        assert_eq!(vps.label, "vps-prod — linux · online · Tailscale SSH");

        // Absent `sshHostKeys` means not SSH-ready, not unknown — so the peer
        // is still offered, just without the marker.
        let windows = tailnet
            .devices()
            .iter()
            .find(|d| d.address.starts_with("troys-machine"))
            .unwrap();
        assert_eq!(windows.label, "troys_machine — windows · online");
    }

    #[test]
    fn magic_dns_disabled_falls_back_to_the_tailscale_ip() {
        let doc = RUNNING.replace("\"MagicDNSEnabled\": true", "\"MagicDNSEnabled\": false");
        let tailnet = Tailnet::parse(&doc);
        assert!(tailnet
            .devices()
            .iter()
            .all(|d| d.address.starts_with("100.")));
    }

    #[test]
    fn non_running_and_garbage_documents_yield_nothing_quietly() {
        for doc in [
            NEEDS_LOGIN,
            r#"{"BackendState":"Stopped","Peer":{}}"#,
            "{\"BackendState\": \"Runn",
            "",
            "not json at all",
        ] {
            let tailnet = Tailnet::parse(doc);
            assert!(
                tailnet.devices().is_empty(),
                "unexpected devices for {doc:?}"
            );
            assert!(tailnet.schema_options().is_empty());
        }
    }

    #[test]
    fn schema_options_are_const_title_pairs() {
        let options = Tailnet::parse(RUNNING).schema_options();
        assert_eq!(options.len(), 3);
        assert_eq!(options[0]["const"], "troys-machine.tailcfd703.ts.net");
        assert!(options[0]["title"].as_str().unwrap().contains("windows"));
    }

    #[test]
    fn contains_matches_only_enumerated_addresses() {
        let tailnet = Tailnet::parse(RUNNING);
        assert!(tailnet.contains("VPS-PROD.tailcfd703.ts.net"));
        assert!(!tailnet.contains("vps.example.com"));
        assert!(!Tailnet::parse(NEEDS_LOGIN).contains("vps-prod.tailcfd703.ts.net"));
    }

    #[test]
    fn an_auth_url_is_lifted_out_of_the_line_ssh_prints_around_it() {
        let stderr =
            b"# To authenticate, visit:\n#\n#\thttps://login.tailscale.com/a/1a2b3c4d5e6f7g\n#\n";
        assert_eq!(
            auth_url_in(stderr).as_deref(),
            Some("https://login.tailscale.com/a/1a2b3c4d5e6f7g")
        );
    }

    #[test]
    fn stderr_without_the_marker_yields_nothing() {
        assert_eq!(auth_url_in(b""), None);
        assert_eq!(auth_url_in(b"Permission denied (publickey).\n"), None);
        // The host must match byte-exactly: a look-alike control server is not
        // the one host this function is allowed to name.
        assert_eq!(
            auth_url_in(b"https://login.tailscale.com.evil.test/a/tok"),
            None
        );
        assert_eq!(auth_url_in(b"http://login.tailscale.com/a/tok"), None);
    }

    #[test]
    fn a_marker_with_no_token_after_it_yields_nothing() {
        // Half a URL is worse than none: it would open a Tailscale 404.
        assert_eq!(auth_url_in(b"https://login.tailscale.com/a/"), None);
        assert_eq!(auth_url_in(b"https://login.tailscale.com/a/ tok"), None);
        assert_eq!(auth_url_in(b"https://login.tailscale.com/a/\n"), None);
    }

    #[test]
    fn an_overlong_token_is_dropped_rather_than_truncated() {
        // 128 is the last length that is still plausibly a real token; a
        // truncated 129th would be a valid-looking URL that goes nowhere.
        let at_cap = format!("https://login.tailscale.com/a/{}", "a".repeat(128));
        assert_eq!(auth_url_in(at_cap.as_bytes()).as_deref(), Some(&at_cap[..]));
        let over_cap = format!("https://login.tailscale.com/a/{}", "a".repeat(129));
        assert_eq!(auth_url_in(over_cap.as_bytes()), None);
    }

    #[test]
    fn nothing_after_the_token_survives_into_the_result() {
        // The whole point of constructing rather than parsing: a query, a
        // fragment, or a second URL cannot ride along into the browser.
        assert_eq!(
            auth_url_in(b"https://login.tailscale.com/a/tok?next=https://evil.test#x").as_deref(),
            Some("https://login.tailscale.com/a/tok")
        );
        // A token flush against the end of the buffer is still a whole token.
        assert_eq!(
            auth_url_in(b"visit: https://login.tailscale.com/a/tok").as_deref(),
            Some("https://login.tailscale.com/a/tok")
        );
    }

    #[test]
    fn cli_candidates_cover_the_platform_install_locations() {
        let candidates = cli_candidates();
        let joined = candidates
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("|");
        if cfg!(windows) {
            assert!(joined.contains("Tailscale\\tailscale.exe"));
        } else {
            assert!(joined.contains("/usr/bin/tailscale"));
            assert!(joined.contains("/Applications/Tailscale.app"));
        }
    }
}
