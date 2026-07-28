//! Installing the host-side tools from copies that live on the desktop.
//!
//! Deploy used to *verify* `buzz-acp` and fail with guidance when the host had
//! none. It now verifies **or installs**: when the deploy payload carries
//! `buzz_acp_binary` — a path on the desktop machine to a Linux `buzz-acp` —
//! and the host resolves none, the binary rides along inside the same script
//! that already carries the agent's identity and is installed to
//! `~/.local/bin/buzz-acp`. There is no second op, no provisioning step, and no
//! new UI state: install is an invisible, idempotent property of deploy.
//!
//! The same machinery ships a second tool: the **`buzz` CLI**. A local agent
//! gets it because the desktop bundles it as a sidecar and prepends
//! `~/.local/bin` and the bundle directory to the spawned harness's `PATH`
//! (`managed_agents::runtime::path::build_augmented_path`). A remote agent is
//! told by its own system prompt to answer with `buzz messages send
//! --reply-to …`, so a host without the CLI produces an agent that hunts the
//! filesystem for a command that is not there. [`CLI`] closes that gap: same
//! probe, same base64 transport, same sha256-before-`mv` rule, installed to
//! `~/.local/bin/buzz`, resolvable at runtime because the unit's env file pins
//! `PATH="$HOME/.local/bin:$PATH"` (`deploy::deploy_script`).
//!
//! **The two tools differ on exactly one axis: what "the host has none, and the
//! desktop pushed none" means.** `buzz-acp` *is* the agent — no binary, no
//! unit worth starting — so it is fail-closed ([`Missing::Fatal`], exit 90).
//! The CLI makes an agent faster and more capable but nothing about the harness
//! depends on it, so its absence is a `WARNING:` line on stderr and the deploy
//! continues ([`Missing::Warn`]). Everything else — the encoding, the digest,
//! the atomic install, the push-when-missing staleness rule — is one shared
//! code path, because a second copy of it would be a second place to get the
//! integrity rules wrong.
//!
//! Integrity failures are fatal for **both** tools. A payload that fails to
//! decode or fails its digest is evidence that the stream itself was damaged —
//! and that stream is also carrying the minted nsec and the unit — so
//! continuing is not obviously safe, and "nothing is ever installed unverified"
//! stays one rule rather than two. Only the absent-payload case is asymmetric.
//!
//! The transport is the constraint that shapes everything here. The script
//! travels on the SSH stdin channel (`ssh.rs`) as text, so raw bytes cannot be
//! embedded: a NUL, a heredoc delimiter, or a stray newline in the middle of an
//! ELF section would corrupt the *script*, not just the payload. base64 makes
//! that impossible by construction — the encoded alphabet is
//! `A-Za-z0-9+/=`, which contains no shell metacharacter, no newline, and (the
//! detail that keeps the heredoc safe) no `_`, so no encoded line can collide
//! with a `BUZZ_…_B64_EOF` delimiter.
//!
//! **Resolution is `PATH` *or* `~/.local/bin/<tool>`,** never `PATH` alone.
//! A non-interactive SSH command reads no profile, so the install destination
//! is not on the ambient `PATH` — which is precisely why the unit's env file
//! pins `PATH="$HOME/.local/bin:$PATH"` itself. A `command -v`-only rule would
//! therefore never find the copy a previous deploy installed, and because
//! deploy is the start path, every start would re-stream the binary and swap it
//! underneath a running fleet. [`resolve`] is that rule, and the probe asks the
//! same question so the two can never disagree.
//!
//! **Staleness rule: push-when-missing only.** A host that already resolves a
//! tool keeps the copy it has, whatever its version. Deploy is also the start
//! path — `start_managed_agent` re-enters it — so a version-comparing rule
//! would silently reinstall underneath a running fleet on every start, and a
//! desktop pinned to an older artifact would *downgrade* the host. Refreshing
//! an existing install is a deliberate act and belongs to a follow-up that
//! fetches release artifacts by version; see `docs/remote-agents.md`.

use base64::Engine as _;
use sha2::{Digest, Sha256};

/// Refuse to embed anything larger than this. A release `buzz-acp` is 10-30 MB
/// and the `buzz` CLI is smaller; base64 inflates either by a third and the
/// result travels as one script on the SSH stdin channel, so a wrong path (a
/// disk image, a core dump, a directory of them) must fail here rather than
/// stream for minutes and then fail on the host.
const MAX_BINARY_BYTES: usize = 200 * 1024 * 1024;

/// base64 line width. GNU `base64` wraps at 76 by default and `-d` ignores
/// newlines; one 40 MB line is legal but pathological for anything that reads
/// the script line-wise — including this crate's own tests.
const LINE_WIDTH: usize = 76;

/// Where an installed tool lands. Unexpanded on purpose: it is emitted into the
/// script and expanded by the *host's* shell, whose `$HOME` is the only one
/// that matters.
const INSTALL_DIR: &str = "$HOME/.local/bin";

/// The marker every non-fatal host-side complaint carries, so `deploy` can
/// forward exactly those lines and nothing else from a successful run's stderr
/// (`deploy::deploy`). Structural, not decorative: a successful deploy's remote
/// stderr is otherwise discarded, and a warning nobody sees is not a warning.
pub const WARNING_PREFIX: &str = "WARNING: ";

/// What it means for the host to resolve no copy of a tool while the desktop
/// pushed none either.
///
/// This is the *whole* difference between `buzz-acp` and the `buzz` CLI. See
/// the module docs: the harness cannot run without the former and runs fine
/// (just slower and blinder) without the latter.
#[derive(Clone, Copy)]
enum Missing {
    /// Stop the deploy: `code` is the script's exit status.
    Fatal { code: u16, message: &'static str },
    /// Say so on stderr and carry on.
    Warn { message: &'static str },
}

impl Missing {
    /// The shell that reacts to an empty `$var` after [`resolve`] ran.
    fn block(self, var: &str) -> String {
        match self {
            Self::Fatal { code, message } => {
                format!(r#"if [ -z "${var}" ]; then echo "{message}" >&2; exit {code}; fi"#)
            }
            Self::Warn { message } => {
                format!(r#"if [ -z "${var}" ]; then echo "{WARNING_PREFIX}{message}" >&2; fi"#)
            }
        }
    }
}

/// One host-side tool this crate can verify or install.
///
/// Constructible only through the two constants below: everything else in the
/// crate names [`ACP`] or [`CLI`] rather than describing a tool of its own, so
/// there is exactly one place where a tool's name, delimiter and
/// missing-on-host policy are decided together.
#[derive(Clone, Copy)]
pub struct Tool {
    /// The binary's name — the `command -v` argument in the default case, and
    /// the file name under [`INSTALL_DIR`].
    pub name: &'static str,
    /// The heredoc delimiter for this tool's encoded payload. Contains `_`,
    /// which is not in the base64 alphabet, so no data line can ever terminate
    /// the heredoc early. `delimiters_cannot_appear_in_encoded_data` pins that.
    delimiter: &'static str,
    /// The shell variable the resolution block leaves the absolute path in.
    var: &'static str,
    /// What an unresolved, un-pushed tool means for the deploy.
    missing: Missing,
}

/// The harness itself. Fail-closed: an agent without it cannot exist.
pub const ACP: Tool = Tool {
    name: "buzz-acp",
    delimiter: "BUZZ_ACP_B64_EOF",
    var: "acp",
    missing: Missing::Fatal {
        code: 90,
        message: "buzz-acp not found on the server's PATH or in ~/.local/bin — install it, or set \
                  'buzz-acp path on the server'",
    },
};

/// The agent-facing CLI. An enhancement, never load-bearing — hence
/// [`Missing::Warn`]. The message avoids backticks and `$` on purpose: it is
/// interpolated into a double-quoted `echo` on the host, where either would be
/// a command substitution.
pub const CLI: Tool = Tool {
    name: "buzz",
    delimiter: "BUZZ_CLI_B64_EOF",
    var: "cli",
    missing: Missing::Warn {
        message: "no 'buzz' CLI on the server's PATH or in ~/.local/bin — agents on this host \
                  cannot reply with 'buzz messages send' and will degrade to slower replies; \
                  install it there, or set BUZZ_CLI_PUSH_BINARY on the desktop and redeploy",
    },
};

/// Where an install of `tool` lands, and the second half of the resolution rule
/// — `~/.local/bin` is the documented convention and the destination below, but
/// it is **not** on a non-interactive SSH `PATH`: the remote shell reads no
/// profile, which is exactly why the unit's env file has to pin
/// `PATH="$HOME/.local/bin:$PATH"` itself. Resolving by `PATH` alone would
/// therefore never see the copy the previous deploy installed, and since deploy
/// is the start path, every agent start would re-stream tens of megabytes and
/// swap the binary underneath a running fleet.
fn install_path(tool: Tool) -> String {
    format!("{INSTALL_DIR}/{}", tool.name)
}

/// A binary read from the desktop's filesystem, encoded for the script and
/// fingerprinted for the host to check.
///
/// The bytes are not secret — but they must not corrupt the script stream that
/// *is* carrying secrets, which is why only the encoded form is kept.
///
/// Deliberately not `Debug`, like `deploy::Agent` and `ssh::Output`: a derived
/// one would put tens of megabytes of base64 one `{:?}` away from a log line.
pub struct Payload {
    /// base64, wrapped to [`LINE_WIDTH`], one trailing newline per line.
    encoded: String,
    /// Lowercase hex SHA-256 of the raw bytes. Travels in the script in the
    /// clear: it is a fingerprint, not a credential.
    sha256: String,
}

/// The size rejection, or `None` when `len` is within the cap.
///
/// Split out so the boundary is testable without materializing a 200 MB file,
/// and applied twice in [`Payload::read`] — once to the metadata, once to the
/// bytes actually read.
fn oversized(tool: Tool, len: u64, path: &str) -> Option<String> {
    (len > MAX_BINARY_BYTES as u64).then(|| {
        format!(
            "the {} binary to push is {len} bytes, over the {MAX_BINARY_BYTES}-byte limit: {path}",
            tool.name
        )
    })
}

impl Payload {
    /// Read, validate and encode the binary at `path` on the **desktop**.
    ///
    /// Every rejection here is a failure the host could only report as
    /// something far less legible: an `Exec format error` from systemd five
    /// seconds after a deploy that looked successful, or a multi-minute stream
    /// of a file that was never a binary. `tool` names the offender in each
    /// message — the desktop can push two, and "the binary to push" would leave
    /// the reader guessing which env var to fix.
    pub fn read(tool: Tool, path: &str) -> Result<Self, String> {
        let name = tool.name;
        let metadata = std::fs::metadata(path)
            .map_err(|e| format!("cannot read the {name} binary to push ({path}): {e}"))?;
        if !metadata.is_file() {
            return Err(format!("the {name} binary to push is not a file: {path}"));
        }
        // Checked before the read, so a wrong path costs a `stat` rather than
        // pulling a disk image into memory.
        if let Some(error) = oversized(tool, metadata.len(), path) {
            return Err(error);
        }

        let bytes = std::fs::read(path)
            .map_err(|e| format!("cannot read the {name} binary to push ({path}): {e}"))?;
        // Re-checked against the bytes actually read: the metadata above is a
        // separate syscall, and the file may have grown between the two.
        if let Some(error) = oversized(tool, bytes.len() as u64, path) {
            return Err(error);
        }
        if bytes.is_empty() {
            return Err(format!("the {name} binary to push is empty: {path}"));
        }
        // Deploy targets are Linux + `systemd --user` throughout, and the
        // desktop pushing the binary is routinely macOS or Windows. Without
        // this check a Mach-O or PE binary installs cleanly and the unit then
        // restart-loops on `Exec format error` every five seconds, with the
        // deploy having reported success.
        if !bytes.starts_with(b"\x7fELF") {
            return Err(format!(
                "the {name} binary to push is not a Linux (ELF) executable: {path}"
            ));
        }

        let sha256 = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Ok(Self {
            encoded: wrap(&base64::engine::general_purpose::STANDARD.encode(&bytes)),
            sha256,
        })
    }

    /// The fingerprint the host checks the decoded file against. Tests assert
    /// against it; the script embeds it through [`resolve_or_install`].
    #[cfg(test)]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// The encoded body, so tests can corrupt it the way a truncated stream
    /// would. The script embeds it through [`resolve_or_install`].
    #[cfg(test)]
    pub fn encoded(&self) -> &str {
        &self.encoded
    }
}

/// base64 output is ASCII, so every byte offset is a character boundary and
/// the line breaks can be taken on the `str` directly — no byte round trip,
/// and nothing to unwrap.
fn wrap(encoded: &str) -> String {
    let mut out = String::with_capacity(encoded.len() + encoded.len() / LINE_WIDTH + 1);
    let mut rest = encoded;
    while !rest.is_empty() {
        let (line, tail) = rest.split_at(LINE_WIDTH.min(rest.len()));
        out.push_str(line);
        out.push('\n');
        rest = tail;
    }
    out
}

/// The script that asks the host which of `tools` it already resolves.
///
/// Deploy is also the *start* path, so without this a desktop with the push
/// seams engaged would stream tens of megabytes of base64 on every single agent
/// start, to a host that has had the binaries since the first one. The probe is
/// one cheap round trip — one, whatever the number of tools — that keeps the
/// payloads off the wire in that case.
///
/// It answers by *printing the name* of each tool it resolved rather than by an
/// exit status, because two tools cannot share one boolean. Each line applies
/// exactly the rule [`resolve_or_install`] applies on the host ([`resolve`]) —
/// `PATH` *or* [`install_path`] — because a probe that only consulted `PATH`
/// would answer "missing" forever for a binary this crate itself installed, and
/// the payload would ride along on every start.
///
/// Each pair is `(tool, already-quoted command)`; the command differs from the
/// tool's own name only for `buzz-acp`, which the operator may pin to an
/// absolute path. Names are compile-time constants, so nothing attacker-shaped
/// reaches the `echo`.
///
/// It remains an optimization and never the decision: the deploy script
/// re-checks on the host and installs only into an empty `$var`, so a host that
/// gains or loses a tool between the two round trips still ends up correct.
pub fn probe_script(tools: &[(Tool, String)]) -> String {
    tools
        .iter()
        .map(|(tool, command)| {
            format!(
                "if command -v {command} >/dev/null 2>&1 || [ -x \"{path}\" ]; then echo {name}; \
                 fi\n",
                path = install_path(*tool),
                name = tool.name,
            )
        })
        .collect()
}

/// Whether [`probe_script`]'s output says the host already has `tool`.
pub fn probe_found(stdout: &str, tool: Tool) -> bool {
    stdout.lines().any(|line| line.trim() == tool.name)
}

/// The `WARNING:` lines a successful deploy's remote stderr carries, scrubbed.
///
/// A deploy that succeeded discards the rest of that stderr (it is host noise),
/// so this is the one channel by which the script can tell a human something
/// short of a failure — today, that the host has no `buzz` CLI.
pub fn warnings(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with(WARNING_PREFIX))
        .map(crate::protocol::redact)
        .collect()
}

/// The host-side resolution rule, shared by every caller so the probe, the
/// push path and the un-pushed path can never disagree.
///
/// Leaves the tool's `$var` holding an absolute path, or empty when the host
/// has none. `command -v` covers a `PATH` install and an absolute configured
/// path; [`install_path`] covers the documented `~/.local/bin` convention,
/// which a non-interactive SSH `PATH` does not contain.
fn resolve(tool: Tool, command: &str) -> String {
    let var = tool.var;
    let install_path = install_path(tool);
    format!(
        r#"{var}=$(command -v {command} 2>/dev/null || true)
if [ -z "${var}" ] && [ -x "{install_path}" ]; then {var}="{install_path}"; fi"#
    )
}

/// The deploy script's resolution block for one tool.
///
/// `command` is the already-`quote()`d command or path to resolve. Resolution
/// is [`resolve`] in both cases — `PATH`, then the `~/.local/bin` convention —
/// so a deploy that carries no binary still finds one an earlier deploy (or the
/// operator, following the documented convention) put there.
///
/// With a payload it becomes resolve-or-install, in that order: an installed
/// copy is never replaced, and a host that had none ends the block with `$var`
/// holding the absolute path of the copy just installed — which, for [`ACP`],
/// is what the unit's `ExecStart` is substituted from later in the same pass.
///
/// Without one, the tool's [`Missing`] policy decides: exit for the harness,
/// a warning for the CLI.
pub fn resolve_or_install(tool: Tool, command: &str, push: Option<&Payload>) -> String {
    let resolve = resolve(tool, command);
    let var = tool.var;

    let Some(payload) = push else {
        let missing = tool.missing.block(var);
        return format!(
            r#"{resolve}
{missing}"#
        );
    };

    // Every failure below removes the temp file before exiting, and the file is
    // only made executable *after* the digest matches, so no path through this
    // block can leave a runnable half-written binary in `~/.local/bin`.
    //
    // The `|| { ... }` on the `base64` line binds to the whole redirected
    // command; the heredoc body begins on the following line either way, so the
    // decode is guarded rather than left to `set -e`, which would exit before
    // the temp file could be removed.
    //
    // Integrity failures exit for BOTH tools, including the non-load-bearing
    // CLI: a payload that arrives damaged says the stream is damaged, and that
    // same stream carries the minted nsec and the unit. "Nothing is installed
    // unverified" stays one rule; only the *absent-payload* case is asymmetric.
    format!(
        r#"{resolve}
if [ -z "${var}" ]; then
command -v base64 >/dev/null 2>&1 || {{ echo "the server has no 'base64' (coreutils), so the desktop cannot install {name} on it" >&2; exit 92; }}
command -v sha256sum >/dev/null 2>&1 || {{ echo "the server has no 'sha256sum' (coreutils), and {name} is never installed unverified" >&2; exit 92; }}
{var}_dir="{INSTALL_DIR}"
mkdir -p "${var}_dir"
{var}_tmp="${var}_dir/.{name}.tmp.$$"
base64 -d > "${var}_tmp" <<'{delimiter}' || {{ rm -f "${var}_tmp"; echo "the pushed {name} did not decode on the server" >&2; exit 93; }}
{encoded}{delimiter}
printf '%s  %s\n' '{sha256}' "${var}_tmp" | sha256sum -c - >/dev/null 2>&1 || {{ rm -f "${var}_tmp"; echo "the pushed {name} failed its sha256 check on the server — refusing to install it" >&2; exit 94; }}
chmod 755 "${var}_tmp"
mv "${var}_tmp" "${var}_dir/{name}"
{var}="${var}_dir/{name}"
fi"#,
        name = tool.name,
        delimiter = tool.delimiter,
        encoded = payload.encoded,
        sha256 = payload.sha256,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file that is a legal ELF header followed by everything that would
    /// break a shell script if it ever reached one unencoded — including both
    /// tools' heredoc delimiters.
    fn hostile_binary() -> Vec<u8> {
        let mut bytes = b"\x7fELF\x02\x01\x01\x00".to_vec();
        bytes.extend_from_slice(b"\0\0'\"$(touch /tmp/buzz-should-not-exist)`id`\r\n");
        bytes.extend_from_slice(format!("{}\n", ACP.delimiter).as_bytes());
        bytes.extend_from_slice(format!("{}\n", CLI.delimiter).as_bytes());
        bytes.extend_from_slice(b"\\x00 \x00 ${HOME} $(id -u)\n");
        bytes.extend_from_slice(&(0u8..=255).collect::<Vec<u8>>());
        bytes
    }

    fn write_temp(name: &str, bytes: &[u8]) -> String {
        let path = std::env::temp_dir().join(format!("buzz-push-{}-{name}", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        path.display().to_string()
    }

    /// `Payload` is intentionally not `Debug` (see its doc comment), so the
    /// rejection comes out by hand — the same pattern `deploy::tests` uses for
    /// `Agent`.
    fn rejection(tool: Tool, path: &str) -> String {
        match Payload::read(tool, path) {
            Err(error) => error,
            Ok(_) => panic!("expected {path} to be rejected, it was accepted"),
        }
    }

    #[test]
    fn encoding_round_trips_bytes_that_would_break_a_shell_script() {
        let bytes = hostile_binary();
        let payload = Payload::read(ACP, &write_temp("hostile", &bytes)).unwrap();

        // The encoded form carries nothing a shell reads as syntax, which is
        // the whole reason a binary can travel inside the script at all.
        for line in payload.encoded.lines() {
            assert!(
                line.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'='),
                "encoded line left the base64 alphabet: {line}"
            );
            assert!(line.len() <= LINE_WIDTH, "unwrapped line: {}", line.len());
        }

        let decoded = base64::engine::general_purpose::STANDARD
            .decode(payload.encoded.replace('\n', ""))
            .unwrap();
        assert_eq!(decoded, bytes, "base64 round trip lost bytes");
    }

    #[test]
    fn delimiters_cannot_appear_in_encoded_data() {
        // The heredocs are only safe because `_` is outside the base64
        // alphabet: a payload that could emit its own terminator would end the
        // heredoc early and hand the rest of the binary to the shell as
        // commands. Both tools' delimiters must hold that property, and they
        // must differ so one script can carry both bodies unambiguously.
        assert!(ACP.delimiter.contains('_'));
        assert!(CLI.delimiter.contains('_'));
        assert_ne!(ACP.delimiter, CLI.delimiter);
        let payload = Payload::read(CLI, &write_temp("delimiter", &hostile_binary())).unwrap();
        // Even though the *source bytes* literally contain both delimiters.
        assert!(!payload.encoded.contains(ACP.delimiter));
        assert!(!payload.encoded.contains(CLI.delimiter));
    }

    #[test]
    fn the_digest_is_the_sha256_of_the_raw_bytes() {
        let bytes = hostile_binary();
        let payload = Payload::read(ACP, &write_temp("digest", &bytes)).unwrap();
        let expected: String = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(payload.sha256(), expected);
        assert_eq!(payload.sha256().len(), 64);
    }

    #[test]
    fn only_a_linux_executable_is_accepted() {
        // A Mach-O binary from the desktop installs cleanly and then
        // restart-loops on the host with `Exec format error`, five seconds at a
        // time, after a deploy that reported success.
        let error = rejection(ACP, &write_temp("macho", b"\xcf\xfa\xed\xfe rest"));
        assert!(error.contains("ELF"), "{error}");

        let error = rejection(ACP, &write_temp("empty", b""));
        assert!(error.contains("empty"), "{error}");

        let missing = std::env::temp_dir().join("buzz-push-does-not-exist");
        let error = rejection(ACP, &missing.display().to_string());
        assert!(error.contains("cannot read"), "{error}");

        // A directory `stat`s fine and `read` would fail with something far
        // less legible, so it is refused by shape rather than by errno.
        let error = rejection(ACP, &std::env::temp_dir().display().to_string());
        assert!(error.contains("not a file"), "{error}");
    }

    #[test]
    fn every_rejection_names_the_tool_it_is_about() {
        // The desktop can push two binaries from two env vars. "the binary to
        // push is not a Linux (ELF) executable" would leave the reader guessing
        // which one to fix.
        let macho = write_temp("macho-named", b"\xcf\xfa\xed\xfe rest");
        assert!(rejection(ACP, &macho).contains("the buzz-acp binary"));
        assert!(rejection(CLI, &macho).contains("the buzz binary"));

        let error = oversized(CLI, u64::MAX, "/tmp/wrong").unwrap();
        assert!(error.contains("the buzz binary to push"), "{error}");
    }

    #[test]
    fn the_size_cap_rejects_at_the_boundary_and_names_the_path() {
        // Exercised through `oversized` rather than by writing a 200 MB file:
        // the boundary is the whole content of the rule, and a real artifact
        // (10-30 MB) must pass it untouched.
        assert_eq!(oversized(ACP, MAX_BINARY_BYTES as u64, "/x"), None);
        assert_eq!(oversized(ACP, 30 * 1024 * 1024, "/x"), None);
        let error = oversized(ACP, MAX_BINARY_BYTES as u64 + 1, "/tmp/wrong-file").unwrap();
        assert!(error.contains("limit"), "{error}");
        assert!(error.contains("/tmp/wrong-file"), "{error}");
        // A `u64` length from a huge file must not wrap on the way to the
        // comparison, which an `as usize` on a 32-bit target would do.
        assert!(oversized(ACP, u64::MAX, "/x").is_some());
    }

    #[test]
    fn no_payload_still_resolves_the_install_destination_and_then_fails_with_exit_90() {
        let resolved = resolve_or_install(ACP, "'buzz-acp'", None);
        assert_eq!(
            resolved,
            r#"acp=$(command -v 'buzz-acp' 2>/dev/null || true)
if [ -z "$acp" ] && [ -x "$HOME/.local/bin/buzz-acp" ]; then acp="$HOME/.local/bin/buzz-acp"; fi
if [ -z "$acp" ]; then echo "buzz-acp not found on the server's PATH or in ~/.local/bin — install it, or set 'buzz-acp path on the server'" >&2; exit 90; fi"#
        );
    }

    #[test]
    fn a_missing_cli_with_no_payload_warns_and_lets_the_deploy_continue() {
        // The asymmetry, pinned to the byte. `buzz-acp` missing is exit 90;
        // the CLI missing is a stderr line and nothing else, because an agent
        // without it still runs — it just cannot answer with the CLI its own
        // system prompt tells it to use.
        let resolved = resolve_or_install(CLI, "'buzz'", None);
        assert_eq!(
            resolved,
            r#"cli=$(command -v 'buzz' 2>/dev/null || true)
if [ -z "$cli" ] && [ -x "$HOME/.local/bin/buzz" ]; then cli="$HOME/.local/bin/buzz"; fi
if [ -z "$cli" ]; then echo "WARNING: no 'buzz' CLI on the server's PATH or in ~/.local/bin — agents on this host cannot reply with 'buzz messages send' and will degrade to slower replies; install it there, or set BUZZ_CLI_PUSH_BINARY on the desktop and redeploy" >&2; fi"#
        );
        // No exit, no `set -e` trip: the deploy carries on past this block.
        assert!(!resolved.contains("exit"));
    }

    #[cfg(unix)]
    #[test]
    fn a_warning_never_stops_a_script_running_under_set_e() {
        // `echo … >&2` returns 0, but the surrounding `if` is what makes that
        // true of the whole block. Prove it against a real shell rather than by
        // reading it: a non-zero last command here would abort every deploy to
        // a host without the CLI.
        let script = format!(
            "set -eu\n{}\necho reached\n",
            resolve_or_install(CLI, "'buzz'", None)
        );
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&script)
            .env("HOME", std::env::temp_dir().join("buzz-no-such-home"))
            .env("PATH", "/nonexistent")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "reached");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.starts_with(WARNING_PREFIX), "{stderr}");
    }

    #[test]
    fn warnings_are_lifted_out_of_remote_stderr_and_scrubbed() {
        let stderr = format!(
            "some host noise\n{WARNING_PREFIX}no 'buzz' CLI\nfailed with nsec1leakedleaked\n"
        );
        let lifted = warnings(&stderr);
        assert_eq!(lifted, vec![format!("{WARNING_PREFIX}no 'buzz' CLI")]);
        // Anything that reaches the desktop goes through the same scrubber the
        // failure path uses — a warning is not an exemption.
        let leaky = format!("{WARNING_PREFIX}key nsec1leakedleaked rejected");
        assert!(!warnings(&leaky)[0].contains("nsec1leakedleaked"));
        assert!(warnings("").is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn resolution_finds_the_install_destination_that_is_not_on_a_non_interactive_path() {
        // The bug this pins: `~/.local/bin` is where every install lands and is
        // NOT on a non-interactive SSH PATH, so a `command -v`-only rule
        // reported "missing" for a binary this crate itself installed — and
        // since deploy is the start path, re-streamed and re-installed it on
        // every single agent start.
        let home = std::env::temp_dir().join(format!("buzz-resolve-{}", std::process::id()));
        let local_bin = home.join(".local/bin");
        std::fs::create_dir_all(&local_bin).unwrap();
        let installed = local_bin.join("buzz-acp");
        std::fs::write(&installed, "#!/bin/sh\nexit 0\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755)).unwrap();

        // `sh -c` with an EMPTY PATH: nothing but the explicit check can find
        // it, which is exactly the remote shell's situation.
        let run = |script: &str| {
            let output = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(format!("{script}\nprintf '%s' \"$acp\"\n"))
                .env("HOME", &home)
                .env("PATH", "/nonexistent")
                .output()
                .unwrap();
            (
                output.status.success(),
                String::from_utf8_lossy(&output.stdout).to_string(),
            )
        };

        // Both the un-pushed path and the shared rule land on the installed
        // copy rather than exiting 90.
        let (ok, acp) = run(&resolve_or_install(ACP, "'buzz-acp'", None));
        assert!(ok, "resolution failed on a host that has the binary");
        assert_eq!(acp, installed.display().to_string());
        let (ok, acp) = run(&resolve(ACP, "'buzz-acp'"));
        assert!(ok);
        assert_eq!(acp, installed.display().to_string());

        // And it is still a real answer, not an unconditional one: remove the
        // file and the same block exits 90.
        std::fs::remove_file(&installed).unwrap();
        let (ok, _) = run(&resolve_or_install(ACP, "'buzz-acp'", None));
        assert!(!ok, "resolution succeeded on a host with no binary at all");
    }

    #[cfg(unix)]
    #[test]
    fn the_probe_answers_which_tools_the_host_already_has() {
        // The probe is what keeps megabytes-large payloads off the wire on
        // every start of every agent, so its answer has to be right in both
        // directions — and for two tools it has to be per-tool, which is why it
        // prints names rather than exiting 0/1. Run against a real `/bin/sh`,
        // since the whole content of the script is shell.
        //
        // `$HOME` is pinned to an empty sandbox: the probe also consults
        // `$HOME/.local/bin`, and the developer running these tests may well
        // have a `buzz` there.
        let home = std::env::temp_dir().join(format!("buzz-probe-home-{}", std::process::id()));
        let local_bin = home.join(".local/bin");
        std::fs::create_dir_all(&local_bin).unwrap();
        let run = |tools: &[(Tool, String)], path: &str| {
            let output = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(probe_script(tools))
                .env("HOME", &home)
                .env("PATH", path)
                .output()
                .unwrap();
            String::from_utf8_lossy(&output.stdout).to_string()
        };

        // `sh` itself is in /bin on every unix, so it stands in for an
        // installed binary without creating one.
        let both = [(ACP, "'sh'".to_string()), (CLI, "'buzz'".to_string())];
        let answer = run(&both, "/bin:/usr/bin");
        assert!(probe_found(&answer, ACP), "{answer}");
        assert!(!probe_found(&answer, CLI), "{answer}");

        // An absolute configured path is answered by existence, not by PATH.
        let answer = run(&[(ACP, "'/bin/sh'".to_string())], "/nonexistent");
        assert!(probe_found(&answer, ACP), "{answer}");

        // The install destination answers too, with nothing on PATH — the case
        // every host is in after its first deploy, and the one a `command -v`
        // probe got wrong forever.
        use std::os::unix::fs::PermissionsExt;
        let installed = local_bin.join("buzz");
        std::fs::write(&installed, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o755)).unwrap();
        let answer = run(&both, "/nonexistent");
        assert!(probe_found(&answer, CLI), "{answer}");
        assert!(!probe_found(&answer, ACP), "{answer}");
        // A non-executable leftover is not an install: `-x`, not `-e`.
        std::fs::set_permissions(&installed, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!probe_found(&run(&both, "/nonexistent"), CLI));
        std::fs::remove_file(&installed).unwrap();

        // The command is interpolated already-quoted, so a hostile configured
        // path is inert rather than executed.
        let canary = std::env::temp_dir().join(format!("buzz-probe-{}", std::process::id()));
        let _ = std::fs::remove_file(&canary);
        let hostile = crate::ssh::quote(&format!("$(touch {})", canary.display()));
        let answer = run(&[(ACP, hostile)], "/bin:/usr/bin");
        assert!(!probe_found(&answer, ACP), "{answer}");
        assert!(!canary.exists(), "the probe executed its own argument");

        // No tools to ask about is no script at all, not an empty round trip
        // that still says something.
        assert!(probe_script(&[]).is_empty());
    }

    #[test]
    fn the_install_block_verifies_before_it_installs() {
        // Both tools share this code path, so both are checked: the CLI is the
        // enhancement, but "nothing becomes executable before it verifies" is
        // not something an enhancement gets to opt out of.
        for tool in [ACP, CLI] {
            let payload = Payload::read(tool, &write_temp("order", &hostile_binary())).unwrap();
            let script = resolve_or_install(tool, "'x'", Some(&payload));
            let name = tool.name;
            let var = tool.var;

            let decode = script.find("base64 -d").unwrap();
            let verify = script.find("sha256sum -c").unwrap();
            let chmod = script.find("chmod 755").unwrap();
            let install = script.find(&format!(r#"mv "${var}_tmp""#)).unwrap();
            assert!(decode < verify, "decode must precede verification");
            assert!(
                verify < chmod,
                "nothing becomes executable before it verifies"
            );
            assert!(chmod < install, "the file is executable before it is moved");

            // Same directory as the target, so the `mv` is a rename and never a
            // cross-device copy that could be observed half-written.
            assert!(script.contains(&format!(r#"{var}_tmp="${var}_dir/.{name}.tmp.$$""#)));
            assert!(script.contains(&format!(r#"mv "${var}_tmp" "${var}_dir/{name}""#)));
            // Resolution wins over installation, so an existing binary is kept.
            assert!(script.contains(&format!(r#"if [ -z "${var}" ]; then"#)));
            // And the freshly installed path is what the rest of the deploy uses.
            assert!(script.contains(&format!(r#"{var}="${var}_dir/{name}""#)));
            // Missing coreutils is a clear message, never a silent skip.
            assert!(script.contains("command -v base64"));
            assert!(script.contains("command -v sha256sum"));
            // Every host-side message names the tool it is about.
            assert!(script.contains(&format!("the pushed {name} did not decode")));
            assert!(script.contains(&format!("the pushed {name} failed its sha256 check")));

            // Where the install lands and where resolution looks are two
            // expressions, so they can drift apart — and if they ever do, every
            // deploy silently re-installs forever. Pin them together.
            assert_eq!(install_path(tool), format!("{INSTALL_DIR}/{name}"));
            assert!(script.contains(&format!(r#"{var}_dir="{INSTALL_DIR}""#)));
            assert!(script.contains(&format!(r#"[ -x "{}" ]"#, install_path(tool))));

            // The heredoc delimiter is QUOTED, so the remote shell performs no
            // expansion on the body. The base64 alphabet already contains
            // nothing expandable, so this is the crate's usual second
            // independent failure rather than the only one — but an unquoted
            // delimiter would make the payload's inertness depend entirely on
            // the encoder, and no behavioural test could see the difference.
            assert!(script.contains(&format!("<<'{}'", tool.delimiter)));
            assert!(!script.contains(&format!("<<{}", tool.delimiter)));
        }
    }
}
