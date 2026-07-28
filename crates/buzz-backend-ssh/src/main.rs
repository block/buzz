//! `buzz-backend-ssh` — run managed agents on a remote host over SSH.
//!
//! A backend provider: the desktop spawns it, writes one JSON request to its
//! stdin, and reads one JSON response from its stdout (`managed_agents::backend
//! ::invoke_provider`). One process per op, no daemon, no state.
//!
//! It is deliberately **not** bundled with the desktop app. It installs to
//! `~/.local/bin` and is found on PATH by `discover_provider_candidates`, so
//! remote execution ships and updates independently of the desktop release.
//!
//! Three invariants hold across every op in this crate:
//!
//! 1. Secrets never reach a log, an error string, a `Debug` rendering, or a
//!    process argument list — locally or on the host. They travel only inside
//!    the script body on the SSH stdin channel (`ssh.rs`), and the one type
//!    that holds one renders as `[REDACTED]` (`protocol::Secret`).
//! 2. A deploy without the desktop-minted `private_key_nsec` fails closed
//!    (`deploy::Agent::from_request`).
//! 3. Tailscale is an enhancement, never a dependency: when it is absent,
//!    logged out, or empty, the `info` schema is byte-identical to the plain
//!    one (`protocol::info_response`).

mod deploy;
mod discover;
mod install;
mod protocol;
mod ssh;
mod tailscale;

use std::io::Read;

use protocol::{Failure, SshConfig};
use ssh::Session;

fn main() {
    let response = match read_request()
        .map_err(Failure::from)
        .and_then(|request| run(&request))
    {
        Ok(response) => response,
        Err(error) => {
            // Human detail on stderr, machine-readable failure on stdout. The
            // desktop needs the second; a developer reading a terminal needs
            // the first. `error` is already credential-scrubbed by whoever
            // produced it, and the desktop scrubs it again on the way in.
            eprintln!("buzz-backend-ssh: {error}");
            let mut response = serde_json::json!({ "ok": false, "error": error.message });
            // An optional key, in both directions: a desktop that does not know
            // it still renders `error`, which names the problem and carries the
            // URL as text, and a desktop that does know it finds nothing here
            // from an older provider. No negotiation, no flag.
            if let Some(url) = error.auth_url {
                response["recovery"] = serde_json::json!({ "action": "open_url", "url": url });
            }
            response
        }
    };
    println!("{response}");
    // Always exit 0. A non-zero exit makes `invoke_provider` discard stdout
    // entirely and report raw stderr, which would throw away the structured
    // error above.
    std::process::exit(0);
}

fn read_request() -> Result<serde_json::Value, String> {
    let mut input = String::new();
    std::io::stdin()
        .read_to_string(&mut input)
        .map_err(|e| format!("failed to read the request from stdin: {e}"))?;
    serde_json::from_str(input.trim()).map_err(|e| format!("request is not valid JSON: {e}"))
}

fn run(request: &serde_json::Value) -> Result<serde_json::Value, Failure> {
    let op = request
        .get("op")
        .and_then(|v| v.as_str())
        .ok_or("request is missing 'op'")?;

    // `info` is the only op that runs before a host is configured — it is what
    // *produces* the host field — so it never opens a session. An unknown op is
    // rejected here too, so a typo costs a parse rather than an SSH handshake.
    match op {
        "info" => return Ok(protocol::info_response()),
        "check" | "discover_harnesses" | "probe_models" | "deploy" => {}
        _ => return Err(format!("unsupported op '{op}'").into()),
    }

    let config = SshConfig::from_request(request)?;
    let session = Session::new(&config, use_tailnet_host_key_policy(&config))?;

    match op {
        "check" => discover::check(&session),
        "discover_harnesses" => discover::discover_harnesses(&config, &session),
        "probe_models" => discover::probe_models(request, &config, &session),
        _ => deploy::deploy(request, &config, &session),
    }
}

/// Trust-on-first-use is allowed for exactly one class of address: a device
/// this machine's own Tailscale daemon lists as a peer. Reaching it already
/// required a WireGuard-authenticated tunnel, so `accept-new` adds no exposure
/// and removes the "paste the host, get `Host key verification failed`" cliff.
///
/// For anything the user typed, `accept-new` would silently make the trust
/// decision on their behalf — a genuine MITM window — so the answer is no.
fn use_tailnet_host_key_policy(config: &SshConfig) -> bool {
    tailscale::Tailnet::detect().contains(config.bare_host())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_needs_no_host_and_opens_no_session() {
        // `info` is what the desktop calls to *build* the host field, so it
        // must succeed with no configuration at all.
        let response = run(&serde_json::json!({ "op": "info", "request_id": "abc" })).unwrap();
        assert_eq!(response["ok"], true);
        assert_eq!(response["name"], "SSH");
        assert!(response["config_schema"]["properties"]["ssh_host"].is_object());
    }

    #[test]
    fn an_unknown_op_fails_before_any_connection_is_attempted() {
        let error = run(&serde_json::json!({
            "op": "teleport",
            "provider_config": { "ssh_host": "vps.invalid" },
        }))
        .unwrap_err();
        assert!(error.message.contains("teleport"), "{error}");
    }

    #[test]
    fn a_request_without_an_op_is_a_structured_error() {
        assert!(run(&serde_json::json!({})).is_err());
    }

    #[test]
    fn ops_that_touch_the_host_require_a_host() {
        // Validated before the session opens, so a misconfigured provider
        // reports the missing field instead of a connection failure.
        for op in ["check", "discover_harnesses", "probe_models", "deploy"] {
            let error = run(&serde_json::json!({ "op": op })).unwrap_err();
            assert!(error.message.contains("provider_config"), "{op}: {error}");
        }
    }

    #[test]
    fn a_tailnet_host_is_the_only_thing_that_relaxes_host_key_checking() {
        // Nothing in a test environment advertises a tailnet, so the policy
        // must come back strict for every address.
        let config = SshConfig {
            host: "vps.example.com".into(),
            ..SshConfig::default()
        };
        if tailscale::Tailnet::detect().schema_options().is_empty() {
            assert!(!use_tailnet_host_key_policy(&config));
        }
    }
}
