//! Tests for [`super::remote_probe`].
//!
//! Extracted to a sibling file to keep `remote_probe.rs` inside the desktop
//! file-size ratchet, following the `#[path = "..._tests.rs"]` convention used
//! elsewhere in this module.

use super::*;

fn targets() -> Vec<HarnessProbeTarget> {
    harness_probe_targets()
}

#[test]
fn probe_script_interpolates_no_user_input() {
    let script = build_probe_script(&targets());
    // Everything in the script must come from the compiled tables. The only
    // shell expansions present are the ones we wrote.
    assert!(script.contains(PROBE_START));
    assert!(script.contains(PROBE_END));
    assert!(script.starts_with("exec $SHELL -lc '"));
    assert!(script.trim_end().ends_with('\''));
    // The script is single-quoted into the ssh argv, so a literal single
    // quote anywhere inside would terminate the quoting early and hand the
    // remainder to the remote shell as separate words.
    let body = script
        .trim_end()
        .trim_start_matches("exec $SHELL -lc '")
        .trim_end_matches('\'');
    assert!(
        !body.contains('\''),
        "script body must contain no literal single quote: {body}"
    );
}

#[test]
fn probe_script_uses_a_login_but_not_interactive_shell() {
    // -l is required: without it, homebrew/npm-global/venv prefixes are
    // absent from PATH and a provisioned host reports as empty.
    //
    // -i must NOT be present. An interactive shell sources .zshrc/.bashrc,
    // where prompt frameworks and completion init live; several of those
    // block forever without a TTY. Verified against a real macOS zsh host:
    // -lic hung indefinitely, -lc returned the full binary set. A hang turns
    // a healthy host into a timeout, which is worse than a missed path.
    let script = build_probe_script(&targets());
    assert!(script.contains("$SHELL -lc"), "must be a login shell");
    assert!(
        !script.contains("-lic") && !script.contains("-li "),
        "probe must not request an interactive shell: {script}"
    );
}

#[test]
fn probe_script_contains_no_pipe_delimited_for_list() {
    // An unquoted `|` inside a `for … in` list is a parse error in bash and
    // zsh both, which kills the loop before it runs.
    let script = build_probe_script(&targets());
    for line in script.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("for ") {
            assert!(
                !trimmed.contains('|'),
                "for-loop list must not contain a pipe: {trimmed}"
            );
        }
    }
}

#[test]
fn probe_script_covers_every_table_harness_and_the_buzz_cli() {
    let targets = targets();
    let script = build_probe_script(&targets);
    for target in &targets {
        for command in target.acp_commands {
            assert!(
                script.contains(command),
                "probe script missing ACP command {command} for {}",
                target.id
            );
        }
        if let Some(cli) = target.underlying_cli {
            assert!(
                script.contains(cli),
                "probe script missing vendor CLI {cli} for {}",
                target.id
            );
        }
    }
    assert!(script.contains("buzz"), "probe must look for the buzz CLI");
}

#[test]
fn every_version_call_is_time_bounded_and_stdin_closed() {
    // Regression guard for a failure found on a real host: `claude --version`
    // never returned, and because the call was unbounded it truncated the
    // whole probe — every harness after it in the loop went unreported and
    // the trailing sentinel never printed. The result read as a
    // half-provisioned machine instead of a stuck command.
    let script = build_probe_script(&targets());
    let version_line = script
        .lines()
        .find(|line| line.contains("--version"))
        .expect("probe script must capture versions");

    // A killer process bounds the call...
    assert!(
        version_line.contains(&format!("sleep {VERSION_TIMEOUT_SECS}")),
        "version call must be time-bounded: {version_line}"
    );
    assert!(
        version_line.contains("kill -9"),
        "version call must kill on timeout: {version_line}"
    );
    // ...the killer's stdout is closed, or it holds the command
    // substitution's pipe open for the full sleep and every binary costs
    // the timeout even when it answers instantly...
    assert!(
        version_line.contains(">/dev/null 2>&1 &"),
        "killer must not hold the capture pipe open: {version_line}"
    );
    // ...and stdin is closed, so a harness that starts its JSON-RPC server
    // instead of printing a version gets EOF rather than blocking.
    assert!(
        version_line.contains("</dev/null"),
        "version call must close stdin: {version_line}"
    );
}

#[test]
fn version_timeout_fits_inside_the_overall_probe_budget() {
    // The per-call bound multiplies across harnesses; if it ever grew past
    // the whole-probe ceiling, a single slow host would trip the outer kill
    // and lose results that the inner bound was meant to preserve.
    assert!(
        u64::from(VERSION_TIMEOUT_SECS) < PROBE_TIMEOUT.as_secs(),
        "per-version bound must stay well inside the probe timeout"
    );
    assert!(u64::from(SSH_CONNECT_TIMEOUT_SECS) < PROBE_TIMEOUT.as_secs());
}

#[test]
fn probe_script_is_deterministic() {
    // Stability matters: a script that reorders between calls defeats
    // caching and makes failures hard to compare.
    assert_eq!(
        build_probe_script(&targets()),
        build_probe_script(&targets())
    );
}

#[test]
fn parses_binaries_user_host_and_os() {
    let raw = format!(
        "motd noise\n{PROBE_START}\n\
         BIN:openclaw:/usr/local/bin/openclaw:2026.7.1\n\
         BIN:buzz:/home/agent/.local/bin/buzz:buzz 0.1.0\n\
         USER:agent\nHOST:workstation\nOS:Linux\n{PROBE_END}\ntrailing noise\n"
    );
    let facts = parse_probe_output(&raw);
    assert_eq!(
        facts.binaries.get("openclaw"),
        Some(&(
            "/usr/local/bin/openclaw".to_string(),
            Some("2026.7.1".to_string())
        ))
    );
    assert_eq!(facts.user.as_deref(), Some("agent"));
    assert_eq!(facts.hostname.as_deref(), Some("workstation"));
    assert_eq!(facts.os.as_deref(), Some("Linux"));
}

#[test]
fn ignores_everything_outside_the_sentinels() {
    // A login shell prints banners; those lines must never be parsed as
    // results.
    let raw = format!(
        "BIN:evil:/tmp/evil:1.0\nUSER:attacker\n{PROBE_START}\n\
         BIN:goose:/usr/bin/goose:1.2\n{PROBE_END}\nBIN:also-evil:/tmp/x:1\n"
    );
    let facts = parse_probe_output(&raw);
    assert!(facts.binaries.contains_key("goose"));
    assert!(!facts.binaries.contains_key("evil"));
    assert!(!facts.binaries.contains_key("also-evil"));
    assert!(facts.user.is_none());
}

#[test]
fn unknown_version_becomes_none() {
    let raw = format!("{PROBE_START}\nBIN:goose:/usr/bin/goose:unknown\n{PROBE_END}\n");
    let facts = parse_probe_output(&raw);
    assert_eq!(
        facts.binaries.get("goose"),
        Some(&("/usr/bin/goose".to_string(), None))
    );
}

#[test]
fn version_containing_colons_survives() {
    let raw = format!(
        "{PROBE_START}\nBIN:hermes-acp:/usr/bin/hermes-acp:0.18.2 (build:2026)\n{PROBE_END}\n"
    );
    let facts = parse_probe_output(&raw);
    assert_eq!(
        facts.binaries.get("hermes-acp").unwrap().1.as_deref(),
        Some("0.18.2 (build:2026)")
    );
}

#[test]
fn adapter_without_its_vendor_cli_is_not_ready() {
    // `claude-agent-acp` present but `claude` absent: the adapter starts and
    // then fails at first use, so it must not be reported as usable.
    let mut facts = ProbeFacts::default();
    facts.binaries.insert(
        "claude-agent-acp".into(),
        ("/usr/local/bin/claude-agent-acp".into(), None),
    );
    let harnesses = assemble_harnesses(&facts, &targets());
    let claude = harnesses.iter().find(|h| h.id == "claude").unwrap();
    assert_eq!(
        claude.acp_command_path.as_deref(),
        Some("/usr/local/bin/claude-agent-acp")
    );
    assert!(claude.underlying_cli_path.is_none());
    assert!(!claude.ready, "adapter without its vendor CLI is not ready");
}

#[test]
fn adapter_with_its_vendor_cli_is_ready() {
    let mut facts = ProbeFacts::default();
    facts.binaries.insert(
        "claude-agent-acp".into(),
        ("/usr/local/bin/claude-agent-acp".into(), None),
    );
    facts
        .binaries
        .insert("claude".into(), ("/usr/local/bin/claude".into(), None));
    let harnesses = assemble_harnesses(&facts, &targets());
    let claude = harnesses.iter().find(|h| h.id == "claude").unwrap();
    assert!(claude.ready);
    assert_eq!(
        claude.underlying_cli_path.as_deref(),
        Some("/usr/local/bin/claude")
    );
}

#[test]
fn self_contained_harness_is_ready_on_its_own() {
    // openclaw's ACP command IS the vendor CLI, so there is no second
    // binary to require.
    let mut facts = ProbeFacts::default();
    facts.binaries.insert(
        "openclaw".into(),
        ("/usr/local/bin/openclaw".into(), Some("2026.7.1".into())),
    );
    let harnesses = assemble_harnesses(&facts, &targets());
    let openclaw = harnesses.iter().find(|h| h.id == "openclaw").unwrap();
    assert!(openclaw.ready);
    assert_eq!(openclaw.version.as_deref(), Some("2026.7.1"));
}

#[test]
fn absent_harness_is_reported_not_omitted() {
    // The UI needs a row per known harness so it can show install hints for
    // the missing ones.
    let harnesses = assemble_harnesses(&ProbeFacts::default(), &targets());
    assert_eq!(harnesses.len(), targets().len());
    assert!(harnesses.iter().all(|h| !h.ready));
    assert!(harnesses.iter().all(|h| h.acp_command_path.is_none()));
    assert!(harnesses.iter().all(|h| !h.install_hint.is_empty()
        || h.source == HarnessSource::Builtin
        || h.source == HarnessSource::Preset));
}

#[test]
fn first_listed_acp_command_wins() {
    // claude lists claude-agent-acp before claude-code-acp; when both are
    // present the preference order must decide, matching local discovery.
    let mut facts = ProbeFacts::default();
    facts
        .binaries
        .insert("claude".into(), ("/usr/bin/claude".into(), None));
    facts.binaries.insert(
        "claude-code-acp".into(),
        ("/usr/bin/claude-code-acp".into(), None),
    );
    facts.binaries.insert(
        "claude-agent-acp".into(),
        ("/usr/bin/claude-agent-acp".into(), None),
    );
    let harnesses = assemble_harnesses(&facts, &targets());
    let claude = harnesses.iter().find(|h| h.id == "claude").unwrap();
    assert_eq!(claude.acp_command.as_deref(), Some("claude-agent-acp"));
}

// ── failure classification ────────────────────────────────────────────────

#[test]
fn classifies_password_wall() {
    let kind = classify_ssh_failure("alice@workstation: Permission denied (publickey,password).");
    assert_eq!(kind, Some(HostProbeErrorKind::PasswordRequired));
}

#[test]
fn classifies_keyboard_interactive_as_password_wall() {
    let kind =
        classify_ssh_failure("user@host: Permission denied (publickey,keyboard-interactive).");
    assert_eq!(kind, Some(HostProbeErrorKind::PasswordRequired));
}

#[test]
fn publickey_only_denial_is_not_a_password_wall() {
    // A bare (publickey) denial is a missing or rejected key. Telling the
    // user to install a key they already have would be wrong, so this stays
    // unclassified and the raw message is surfaced instead.
    assert_eq!(
        classify_ssh_failure("user@host: Permission denied (publickey)."),
        None
    );
}

#[test]
fn classifies_host_key_problems() {
    assert_eq!(
        classify_ssh_failure("Host key verification failed."),
        Some(HostProbeErrorKind::HostKeyProblem)
    );
    assert_eq!(
        classify_ssh_failure("WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!"),
        Some(HostProbeErrorKind::HostKeyProblem)
    );
}

#[test]
fn classifies_a_first_seen_key_under_strict_checking() {
    // What `StrictHostKeyChecking=yes` actually emits for an unknown host.
    // Under `accept-new` this text never appeared, because ssh accepted the
    // key and wrote it to known_hosts instead of refusing.
    assert_eq!(
        classify_ssh_failure(
            "No ED25519 host key is known for workstation and you have requested strict \
             checking.\r\nHost key verification failed."
        ),
        Some(HostProbeErrorKind::HostKeyProblem)
    );
}

#[test]
fn an_untrusted_key_and_a_changed_key_read_differently() {
    // Both are refused, but a changed key is the warning ssh exists to give.
    // If the two rendered identically the user would learn to dismiss it.
    let unknown = failure_message(
        &HostProbeErrorKind::HostKeyProblem,
        "workstation",
        "No ED25519 host key is known for workstation and you have requested strict checking.",
    );
    let changed = failure_message(
        &HostProbeErrorKind::HostKeyProblem,
        "workstation",
        "WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED!",
    );

    assert!(
        unknown.contains("not yet trusted") && unknown.contains("ssh workstation"),
        "first-seen message must name the remedy: {unknown}"
    );
    assert!(
        changed.contains("CHANGED") && changed.contains("intercepted"),
        "changed-key message must state the risk: {changed}"
    );
    assert_ne!(unknown, changed);
    // Neither may offer an in-app way out. Buzz never writes known_hosts, so
    // a message hinting otherwise would describe a button that cannot exist.
    assert!(
        unknown.contains("does not accept host keys on your behalf"),
        "the first-seen message must say why Buzz cannot just proceed: {unknown}"
    );
    assert!(
        changed.contains("will \nnot probe") || changed.contains("will not probe"),
        "the changed-key message must state the refusal: {changed}"
    );
}

#[test]
fn the_probe_never_accepts_a_host_key_on_the_users_behalf() {
    // The argument list is the whole enforcement: there is no code path that
    // inspects known_hosts, so strictness lives entirely in this flag. A
    // revert to `accept-new` would silently restore side-effecting probes.
    let host = SshHost {
        host: "workstation".to_string(),
        hostname: None,
        user: None,
        port: None,
        identity_file: None,
    };
    let args = ssh_probe_args(&host);

    assert!(
        args.contains(&"StrictHostKeyChecking=yes".to_string()),
        "probe must use strict host-key checking: {args:?}"
    );
    assert!(
        !args.iter().any(|arg| arg.contains("accept-new")),
        "accept-new writes a first-seen key into known_hosts: {args:?}"
    );
    assert!(
        !args.iter().any(|arg| arg.contains("UserKnownHostsFile")),
        "the probe must not redirect the user's known_hosts: {args:?}"
    );
    assert!(args.contains(&"BatchMode=yes".to_string()));
}

#[test]
fn a_declared_port_is_passed_through_and_an_absent_one_is_not() {
    // Pairs with the ssh_config stanza-boundary fix: this is the consumer
    // that turns a mis-attributed `Port` into a connection to the wrong
    // port, so it must forward exactly what was parsed and nothing else.
    let mut host = SshHost {
        host: "workstation".to_string(),
        hostname: None,
        user: None,
        port: None,
        identity_file: None,
    };
    assert!(
        !ssh_probe_args(&host).contains(&"-p".to_string()),
        "no declared port must mean no -p, leaving ssh to apply its own default"
    );

    host.port = Some("2222".to_string());
    let args = ssh_probe_args(&host);
    let port_flag = args.iter().position(|arg| arg == "-p").expect("-p");
    assert_eq!(args[port_flag + 1], "2222");
}

#[test]
fn classifies_unreachable_hosts() {
    for stderr in [
        "ssh: Could not resolve hostname nope: nodename nor servname provided",
        "ssh: connect to host x port 22: Connection refused",
        "ssh: connect to host x port 22: Operation timed out",
        "ssh: connect to host x port 22: No route to host",
    ] {
        assert_eq!(
            classify_ssh_failure(stderr),
            Some(HostProbeErrorKind::Unreachable),
            "failed to classify: {stderr}"
        );
    }
}

#[test]
fn unrecognized_stderr_stays_unclassified() {
    assert_eq!(classify_ssh_failure("something entirely new"), None);
}

#[test]
fn password_message_names_the_remedy_and_promises_no_storage() {
    let message = failure_message(
        &HostProbeErrorKind::PasswordRequired,
        "workstation",
        "Permission denied (password).",
    );
    assert!(message.contains("never stores SSH passwords"));
    assert!(message.contains("ssh-copy-id workstation"));
}

// ── localhost path ────────────────────────────────────────────────────────

#[test]
fn localhost_probe_returns_the_same_shape_as_a_remote_one() {
    // Runs the real script against this machine. Asserts on shape, not on
    // which harnesses happen to be installed on the test runner.
    let result = probe_localhost();
    assert_eq!(result.host, LOCALHOST_ID);
    assert!(
        result.ok,
        "localhost probe should succeed; error: {:?}",
        result.error
    );
    assert_eq!(result.harnesses.len(), harness_probe_targets().len());
    assert!(result.os.is_some(), "uname should report an OS");
    // Every entry that claims readiness must carry a resolved path.
    for harness in &result.harnesses {
        if harness.ready {
            assert!(
                harness.acp_command_path.is_some(),
                "{} claims ready with no path",
                harness.id
            );
        }
    }
}

/// Run the real `run_probe` over a shell script that emits a chosen stdout,
/// standing in for whatever a remote login shell happened to produce.
fn probe_with_stdout(script: &str) -> HostProbeResult {
    let mut command = Command::new("/bin/sh");
    command.arg("-c").arg(script);
    run_probe(command, "workstation", &targets(), Instant::now())
}

#[test]
fn a_probe_cut_off_after_the_start_marker_is_not_reported_as_success() {
    // A dropped session mid-probe. Only PROBE_START was required, so this
    // returned `ok: true` carrying whichever harnesses happened to be
    // enumerated before the connection died — indistinguishable from a host
    // where the rest genuinely are not installed.
    let result = probe_with_stdout(&format!(
        "printf '%s\\nBIN:goose:/usr/bin/goose:1.2\\n' '{PROBE_START}'"
    ));

    assert!(
        !result.ok,
        "an incomplete probe must not be reported as a successful one"
    );
    assert_eq!(result.error_kind, Some(HostProbeErrorKind::Truncated));
    assert!(
        result.harnesses.is_empty(),
        "partial facts must be withheld, not shown as a complete answer"
    );
    assert!(result.buzz_cli_path.is_none());
    assert!(result.os.is_none());
    let error = result.error.expect("a truncated probe must explain itself");
    assert!(error.contains("incomplete"), "unhelpful message: {error}");
}

#[test]
fn a_complete_probe_with_the_same_facts_does_succeed() {
    // The control for the case above: identical output plus the closing
    // marker. Without this, requiring PROBE_END could pass by rejecting
    // everything.
    let result = probe_with_stdout(&format!(
        "printf '%s\\nBIN:goose:/usr/bin/goose:1.2\\nOS:Linux\\n%s\\n' \
         '{PROBE_START}' '{PROBE_END}'"
    ));

    assert!(result.ok, "error: {:?}", result.error);
    assert_eq!(result.error_kind, None);
    assert_eq!(result.os.as_deref(), Some("Linux"));
    assert_eq!(result.harnesses.len(), targets().len());
}

#[test]
fn a_truncated_probe_is_distinguished_from_one_that_never_started() {
    // Both are failures, and both must stay distinct: "never started" is an
    // ssh-level problem to classify from stderr, while "truncated" means
    // authentication already succeeded. Collapsing them would point the user
    // at the wrong layer.
    let never_started = probe_with_stdout("printf 'motd only\\n'");
    assert!(!never_started.ok);
    assert_ne!(
        never_started.error_kind,
        Some(HostProbeErrorKind::Truncated),
        "no start marker is not a truncated probe"
    );
}

#[test]
fn unreachable_host_reports_a_status_rather_than_erroring() {
    let host = SshHost {
        host: "buzz-nonexistent-test-host.invalid".to_string(),
        hostname: None,
        user: None,
        port: None,
        identity_file: None,
    };
    let result = probe_ssh_host(&host);
    assert!(!result.ok);
    assert!(result.error.is_some());
    assert!(result.harnesses.is_empty());
    // Whatever the local resolver does, this must not be reported as a
    // password wall — that would send the user chasing the wrong fix.
    assert_ne!(
        result.error_kind,
        Some(HostProbeErrorKind::PasswordRequired)
    );
}
