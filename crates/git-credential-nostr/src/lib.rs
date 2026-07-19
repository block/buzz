//! git-credential-nostr — NIP-98 git credential helper for Buzz.
//!
//! Git calls this via the credential helper protocol (stdin/stdout).
//! We read the request, sign a kind:27235 event, and return the base64-encoded
//! event as the credential value.  Git then sends:
//!   Authorization: Nostr <credential>

use std::io::{self, BufRead, Write};

use base64::Engine as _;
use nostr::nips::nip98::{HttpData, HttpMethod};
use nostr::types::Url;
use nostr::{EventBuilder, Keys, Tag};
use zeroize::Zeroize;

fn git_config(key: &str) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["config", "--get", key])
        .output()
        .ok()?;
    if out.status.success() {
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        None
    }
}

#[cfg(unix)]
fn check_keyfile_permissions(path: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot stat keyfile {path}: {e}"))?;
    let mode = meta.permissions().mode() & 0o777;
    if mode & 0o177 != 0 {
        return Err(format!(
            "keyfile {path} has insecure permissions (expected 0600)"
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn check_keyfile_permissions(path: &str) -> Result<(), String> {
    eprintln!("warning: cannot check keyfile permissions on this platform ({path})");
    Ok(())
}

/// Max keyfile size — nsec1 is 63 bytes; hex keys are 64 bytes. 256 is generous.
const MAX_KEYFILE_BYTES: u64 = 256;

fn load_key() -> Result<String, String> {
    if let Ok(val) = std::env::var("NOSTR_PRIVATE_KEY") {
        if !val.is_empty() {
            return Ok(val);
        }
    }
    let path = git_config("nostr.keyfile").ok_or_else(|| {
        "no nostr key configured. Set $NOSTR_PRIVATE_KEY or git config nostr.keyfile".to_string()
    })?;
    check_keyfile_permissions(&path)?;
    let meta = std::fs::metadata(&path).map_err(|e| format!("cannot stat keyfile {path}: {e}"))?;
    if !meta.is_file() {
        return Err(format!("keyfile {path} is not a regular file"));
    }
    if meta.len() > MAX_KEYFILE_BYTES {
        return Err(format!(
            "keyfile {path} exceeds {MAX_KEYFILE_BYTES}-byte size limit"
        ));
    }
    let raw =
        std::fs::read_to_string(&path).map_err(|e| format!("cannot read keyfile {path}: {e}"))?;
    Ok(raw.trim().to_string())
}

/// Load the NIP-OA owner attestation injected by Buzz Desktop/ACP.
///
/// The tag must be part of the signed NIP-98 event: Git's credential protocol
/// can return an Authorization value, but it cannot add a separate HTTP header.
fn load_auth_tag() -> Result<Option<Tag>, String> {
    let raw = std::env::var("BUZZ_AUTH_TAG")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| git_config("nostr.authtag"));

    raw.map(|value| {
        let parts: Vec<String> =
            serde_json::from_str(&value).map_err(|e| format!("invalid NIP-OA auth tag: {e}"))?;
        if parts.len() != 4 || parts.first().map(String::as_str) != Some("auth") {
            return Err(
                "invalid NIP-OA auth tag: expected [auth, owner, conditions, signature]"
                    .to_string(),
            );
        }
        Tag::parse(parts).map_err(|e| format!("invalid NIP-OA auth tag: {e}"))
    })
    .transpose()
}

#[derive(Default)]
struct CredRequest {
    has_authtype_capability: bool,
    protocol: Option<String>,
    host: Option<String>,
    path: Option<String>,
    wwwauth: Option<String>,
}

fn parse_stdin() -> CredRequest {
    let stdin = io::stdin();
    let mut req = CredRequest::default();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.is_empty() {
            break;
        }
        if line == "capability[]=authtype" {
            req.has_authtype_capability = true;
        } else if let Some(v) = line.strip_prefix("protocol=") {
            req.protocol = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("host=") {
            req.host = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("path=") {
            req.path = Some(v.to_string());
        } else if let Some(v) = line.strip_prefix("wwwauth[]=") {
            if v.starts_with("Nostr ") && req.wwwauth.is_none() {
                req.wwwauth = Some(v.to_string());
            }
        }
    }
    req
}

fn parse_method(wwwauth: &str) -> Option<HttpMethod> {
    // Strip the scheme prefix ("Nostr ") if present, then split on commas.
    // Handles variations: `Nostr method="GET", realm="buzz"` and
    // `Nostr method="GET",realm="buzz"` (with or without space after comma).
    let params = wwwauth.strip_prefix("Nostr ").unwrap_or(wwwauth);
    for param in params.split(',') {
        let param = param.trim();
        if let Some(rest) = param.strip_prefix("method=\"") {
            let end = rest.find('"')?;
            return rest[..end].parse().ok();
        }
    }
    None
}

/// Run the credential helper. Returns exit code.
/// Reads from stdin, writes to stdout. Errors go to stderr only.
pub fn run() -> i32 {
    match std::env::args().nth(1).as_deref() {
        Some("get") | None => {}
        Some(_) => return 0, // store, erase, or unknown → silent exit 0
    }

    let req = parse_stdin();

    if !req.has_authtype_capability {
        println!();
        let _ = io::stdout().flush();
        return 0;
    }

    macro_rules! require {
        ($opt:expr, $msg:expr) => {
            match $opt {
                Some(v) => v,
                None => {
                    eprintln!("error: {}", $msg);
                    return 1;
                }
            }
        };
    }

    // No Nostr challenge from the server — this isn't a Buzz remote.
    // Exit silently so git falls through to the next credential helper.
    // This check comes FIRST so non-Buzz remotes never hit validation errors.
    let wwwauth = match req.wwwauth.as_deref() {
        Some(v) => v,
        None => return 0,
    };
    let method = match parse_method(wwwauth) {
        Some(m) => m,
        None => return 0,
    };

    let protocol = require!(
        req.protocol.as_deref(),
        "missing protocol in credential request"
    );
    let host = require!(req.host.as_deref(), "missing host in credential request");
    let path = require!(
        req.path.as_deref(),
        "credential.useHttpPath must be true for NIP-98 auth"
    );

    let repo_path = path
        .split_once("/info/refs")
        .map(|(prefix, _)| prefix)
        .or_else(|| path.strip_suffix("/git-upload-pack"))
        .or_else(|| path.strip_suffix("/git-receive-pack"))
        .unwrap_or(path);
    let url = format!("{protocol}://{host}/{repo_path}");

    let mut raw_key = match load_key() {
        Ok(k) => k,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };

    let keys = match Keys::parse(&raw_key) {
        Ok(k) => k,
        Err(e) => {
            raw_key.zeroize();
            eprintln!("error: invalid nostr private key: {e}");
            return 1;
        }
    };
    raw_key.zeroize();

    let parsed_url = Url::parse(&url).unwrap_or_else(|e| panic!("invalid URL {url:?}: {e}"));
    let http_data = HttpData::new(parsed_url, method);
    let auth_tag = match load_auth_tag() {
        Ok(tag) => tag,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    let builder = EventBuilder::http_auth(http_data);
    let builder = match auth_tag {
        Some(tag) => builder.tag(tag),
        None => builder,
    };
    let event = match builder.sign_with_keys(&keys) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: failed to sign NIP-98 event: {e}");
            return 1;
        }
    };

    let json = match serde_json::to_string(&event) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("error: failed to serialize event: {e}");
            return 1;
        }
    };

    let credential = base64::engine::general_purpose::STANDARD.encode(json.as_bytes());

    println!("capability[]=authtype");
    println!("authtype=Nostr");
    println!("credential={credential}");
    println!("ephemeral=true");
    println!("quit=true");
    println!();
    let _ = io::stdout().flush();
    0
}
