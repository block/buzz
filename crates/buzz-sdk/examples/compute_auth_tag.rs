//! Compute a NIP-OA auth tag for an agent keypair.
//!
//! The owner's secret key never travels on argv or in the environment: argv is
//! world-readable through `/proc/<pid>/cmdline` for the life of the process,
//! and an environment leaks into every child. The key is read from a file
//! (root-readable 0600 is the expected shape) or from stdin.
//!
//! Usage:
//!   cargo run --release --example compute_auth_tag -- \
//!       --owner-key-file <path> <agent_pubkey_hex> [--conditions <str>]
//!   cargo run --release --example compute_auth_tag -- \
//!       --owner-key-stdin <agent_pubkey_hex> [--conditions <str>]   < owner.key
//!
//! Prints the JSON auth tag to stdout. A positional argument that looks like a
//! secret key is refused before anything else happens.

use std::io::Read;

use buzz_sdk::nip_oa;
use nostr::{Keys, PublicKey};

/// Where the owner secret comes from. Never argv.
#[derive(Debug, PartialEq, Eq)]
enum KeySource {
    File(String),
    Stdin,
}

#[derive(Debug, PartialEq, Eq)]
struct Invocation {
    key_source: KeySource,
    agent_pubkey_hex: String,
    conditions: String,
}

const USAGE: &str = "Usage: compute_auth_tag (--owner-key-file <path> | --owner-key-stdin) \
                     <agent_pubkey_hex> [--conditions <str>]";

/// Does this argument have the unmistakable shape of a secret key? Used only
/// to refuse it. Only the bech32 form is recognisable: a 64-hex secret and a
/// 64-hex x-only pubkey are the same shape, so hex cannot be told apart here —
/// the old positional form is caught instead by its argument count.
fn looks_like_secret(arg: &str) -> bool {
    arg.trim().starts_with("nsec1")
}

/// Parse argv (without argv[0]). The owner secret has no argument to arrive
/// through; a positional that parses as a secret key is refused rather than
/// interpreted, so the pre-2026-09 form `<owner_secret_hex> <agent_pubkey>`
/// fails loudly instead of silently working.
fn parse_args(args: &[String]) -> Result<Invocation, String> {
    let mut key_source: Option<KeySource> = None;
    let mut positional: Vec<String> = Vec::new();
    let mut conditions = String::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--owner-key-file" => {
                let path = iter
                    .next()
                    .ok_or_else(|| format!("--owner-key-file needs a path\n{USAGE}"))?;
                if key_source.replace(KeySource::File(path.clone())).is_some() {
                    return Err(format!("the owner key source was given twice\n{USAGE}"));
                }
            }
            "--owner-key-stdin" => {
                if key_source.replace(KeySource::Stdin).is_some() {
                    return Err(format!("the owner key source was given twice\n{USAGE}"));
                }
            }
            "--conditions" => {
                conditions = iter
                    .next()
                    .ok_or_else(|| format!("--conditions needs a value\n{USAGE}"))?
                    .clone();
            }
            other if other.starts_with("--") => {
                return Err(format!("unknown flag {other}\n{USAGE}"));
            }
            other => positional.push(other.to_string()),
        }
    }

    // Refuse first, before deciding what the positionals mean. The pre-2026-09
    // form put the owner secret in position 0 and the agent pubkey in position
    // 1; two positionals are refused with that explanation, and the values are
    // never echoed.
    if positional.iter().any(|p| looks_like_secret(p)) || positional.len() > 1 {
        return Err(
            "refused: the owner secret must not be on argv (it would be readable in \
             /proc/<pid>/cmdline for the life of the process). Pass it with \
             --owner-key-file <path> or --owner-key-stdin, and give only the agent pubkey \
             as a positional argument."
                .to_string(),
        );
    }

    let key_source = key_source.ok_or_else(|| {
        format!("the owner key must come from --owner-key-file or --owner-key-stdin\n{USAGE}")
    })?;
    if positional.len() != 1 {
        return Err(format!(
            "expected the agent pubkey hex as the only positional argument\n{USAGE}"
        ));
    }
    let agent_pubkey_hex = positional.remove(0);
    PublicKey::from_hex(&agent_pubkey_hex).map_err(|e| format!("invalid agent pubkey hex: {e}"))?;

    Ok(Invocation {
        key_source,
        agent_pubkey_hex,
        conditions,
    })
}

/// Read the owner secret: first non-empty line, trimmed. The value is never
/// printed, and no error message includes it.
fn read_owner_secret(source: &KeySource) -> Result<String, String> {
    let mut raw = String::new();
    match source {
        KeySource::File(path) => {
            raw = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read the owner key file {path}: {e}"))?;
        }
        KeySource::Stdin => {
            std::io::stdin()
                .read_to_string(&mut raw)
                .map_err(|e| format!("cannot read the owner key from stdin: {e}"))?;
        }
    }
    let secret = raw
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .ok_or_else(|| "the owner key source is empty".to_string())?;
    Ok(secret.to_string())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let invocation = match parse_args(&args) {
        Ok(inv) => inv,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };

    let secret = match read_owner_secret(&invocation.key_source) {
        Ok(secret) => secret,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };
    // Deliberately not `expect(...)` with the input: the parse error must not
    // echo what was read.
    let owner_keys = match Keys::parse(&secret) {
        Ok(keys) => keys,
        Err(_) => {
            eprintln!("the owner key source does not contain a valid secret key");
            std::process::exit(1);
        }
    };
    let agent_pubkey =
        PublicKey::from_hex(&invocation.agent_pubkey_hex).expect("validated in parse_args");

    match nip_oa::compute_auth_tag(&owner_keys, &agent_pubkey, &invocation.conditions) {
        Ok(tag_json) => println!("{tag_json}"),
        Err(e) => {
            eprintln!("failed to compute auth tag: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::ToBech32;

    fn agent_hex() -> String {
        Keys::generate().public_key().to_hex()
    }

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn file_and_stdin_forms_parse_and_carry_no_secret() {
        let agent = agent_hex();
        let inv = parse_args(&argv(&["--owner-key-file", "/etc/buzz/owner.key", &agent])).unwrap();
        assert_eq!(
            inv.key_source,
            KeySource::File("/etc/buzz/owner.key".into())
        );
        assert_eq!(inv.agent_pubkey_hex, agent);
        assert_eq!(inv.conditions, "");

        let inv = parse_args(&argv(&[
            "--owner-key-stdin",
            &agent,
            "--conditions",
            "exp=1",
        ]))
        .unwrap();
        assert_eq!(inv.key_source, KeySource::Stdin);
        assert_eq!(inv.conditions, "exp=1");
    }

    #[test]
    fn a_secret_on_argv_is_refused_before_anything_else() {
        let owner = Keys::generate();
        let agent = agent_hex();
        let secret_hex = owner.secret_key().to_secret_hex();
        // The pre-2026-09 positional form: two positionals, refused as such.
        let err = parse_args(&argv(&[&secret_hex, &agent])).unwrap_err();
        assert!(err.contains("refused"), "{err}");
        assert!(
            !err.contains(&secret_hex),
            "the refusal must not echo the secret"
        );
        // Even next to a legitimate key source.
        let err = parse_args(&argv(&["--owner-key-stdin", &secret_hex, &agent])).unwrap_err();
        assert!(err.contains("refused"), "{err}");
        assert!(!err.contains(&secret_hex));
        // A lone 64-hex positional is indistinguishable from a pubkey, so it
        // is not "refused" — but without a key source it still cannot run.
        assert!(parse_args(&argv(&[&secret_hex])).is_err());
        // nsec form is unmistakable and refused even alone.
        let nsec = owner.secret_key().to_bech32().unwrap();
        let err = parse_args(&argv(&["--owner-key-stdin", &nsec, &agent])).unwrap_err();
        assert!(err.contains("refused"), "{err}");
        assert!(!err.contains(&nsec));
    }

    #[test]
    fn missing_source_or_pubkey_is_an_error() {
        let agent = agent_hex();
        assert!(parse_args(&argv(&[&agent])).is_err(), "no key source");
        assert!(
            parse_args(&argv(&["--owner-key-stdin"])).is_err(),
            "no agent pubkey"
        );
        assert!(
            parse_args(&argv(&["--owner-key-stdin", "not-hex"])).is_err(),
            "invalid agent pubkey"
        );
        assert!(
            parse_args(&argv(&[
                "--owner-key-stdin",
                "--owner-key-file",
                "x",
                &agent
            ]))
            .is_err(),
            "two sources"
        );
        assert!(
            parse_args(&argv(&["--secret", "x", &agent])).is_err(),
            "unknown flag"
        );
    }

    #[test]
    fn the_secret_is_read_from_a_file_and_the_tag_verifies() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let dir = std::env::temp_dir().join(format!("compute-auth-tag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("owner.key");
        std::fs::write(&path, format!("{}\n", owner.secret_key().to_secret_hex())).unwrap();

        let secret = read_owner_secret(&KeySource::File(path.to_string_lossy().into())).unwrap();
        let keys = Keys::parse(&secret).unwrap();
        assert_eq!(keys.public_key(), owner.public_key());

        let tag_json = nip_oa::compute_auth_tag(&keys, &agent.public_key(), "").unwrap();
        let tag = nip_oa::parse_auth_tag(&tag_json).unwrap();
        assert_eq!(tag.as_slice()[1], owner.public_key().to_hex());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_empty_source_is_an_error_that_names_no_value() {
        let dir =
            std::env::temp_dir().join(format!("compute-auth-tag-empty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("owner.key");
        std::fs::write(&path, "\n\n").unwrap();
        let err = read_owner_secret(&KeySource::File(path.to_string_lossy().into())).unwrap_err();
        assert!(err.contains("empty"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
