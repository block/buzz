//! Secret-leak gate for the environment fence.
//!
//! These tests spawn a real PTY child and read its actual environment. An
//! assertion against the `CommandBuilder` alone would be weaker: it would not
//! prove that what the builder holds is what the kernel hands the child.

use crate::env_fence::{fence_env, WINDOWS_INHERIT_ALLOWLIST};
use crate::path::{user_shell_path, windows_shell_path};
use crate::shell::{
    is_executable_file, login_argv0, pick_windows_shell, resolve_shell, FALLBACK_SHELL,
};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::Read;

/// Secrets Buzz's own process holds. Sourced from the desktop crate's
/// `RESERVED_ENV_KEYS` (`src/managed_agents/env_vars.rs:58`); duplicated
/// rather than imported because this crate deliberately has no dependency
/// on the Tauri crate. `reserved_keys_are_covered` keeps the two in step.
const SECRET_KEYS: &[&str] = &[
    "BUZZ_PRIVATE_KEY",
    "NOSTR_PRIVATE_KEY",
    "BUZZ_AUTH_TAG",
    "BUZZ_API_TOKEN",
    "BUZZ_ACP_PRIVATE_KEY",
    "BUZZ_ACP_API_TOKEN",
    "BUZZ_RELAY_URL",
];

const CANARY: &str = "SAMI_CANARY_MUST_NOT_LEAK";

/// Uniquely-named executable seeded into Buzz's own PATH; the child must not
/// be able to run it.
const CANARY_BIN: &str = "buzz-hermit-canary-tool";

/// Creates a fixture file at `name` with `mode`, replacing any leftover from
/// a previous run.
///
/// The removal is not tidiness: a fixture written at mode `0o010` is not
/// writable by its own owner, so a second run in the same temp dir fails with
/// `Permission denied` before reaching a single assertion. Green on a fresh
/// runner, red on a persistent one — a test must not depend on which it got.
#[cfg(unix)]
fn fixture_file(name: &str, contents: &str, mode: u32) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = std::env::temp_dir().join(name);
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, contents).expect("write fixture");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("chmod fixture");
    path
}

/// Runs `env` in a real PTY child under the full fence and returns its output.
fn fenced_child_environment() -> String {
    let shell = resolve_shell(std::env::var("SHELL").ok().as_deref());
    child_environment(|cmd| fence_env(cmd, &user_shell_path(), &shell))
}

/// Runs `env` in a real PTY child and returns its raw output.
fn child_environment(build: impl FnOnce(&mut CommandBuilder)) -> String {
    child_command(build, "env")
}

/// Runs `script` in a real PTY child under `build`'s fence and returns the
/// child's output.
///
/// The child is a real process on a real PTY rather than an inspection of the
/// `CommandBuilder`: the builder is what we asked for, and the child's
/// `environ` is what the kernel actually delivered. Only the second one is the
/// property under test.
fn child_command(build: impl FnOnce(&mut CommandBuilder), script: &str) -> String {
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("openpty");

    let mut cmd = CommandBuilder::new("/bin/sh");
    build(&mut cmd);
    cmd.arg("-c");
    cmd.arg(script);

    let mut child = pair.slave.spawn_command(cmd).expect("spawn");
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().expect("reader");
    let mut out = String::new();
    reader.read_to_string(&mut out).expect("read child output");
    child.wait().expect("wait");
    out
}

/// Seeds this process with secrets so the fence has something to leak.
///
/// Note these are process-global; the tests that rely on them assert on a
/// canary value they set themselves, so a real `BUZZ_PRIVATE_KEY` in the
/// developer's environment neither masks a failure nor causes one.
fn seed_secrets() {
    for key in SECRET_KEYS {
        std::env::set_var(key, format!("{CANARY}_{key}"));
    }
}

#[test]
fn fence_keeps_secrets_out_of_the_child() {
    seed_secrets();
    let out = fenced_child_environment();

    assert!(
        !out.contains(CANARY),
        "a reserved secret reached the child environment:\n{out}"
    );
    for key in SECRET_KEYS {
        assert!(
            !out.lines().any(|line| line.starts_with(&format!("{key}="))),
            "{key} reached the child environment:\n{out}"
        );
    }
}

/// The other half of the assertion. A fence that clears in the wrong order
/// produces an empty environment: it passes the leak check above while
/// shipping a shell with no context and no error. Asserting only the negative
/// would ratify that bug.
#[test]
fn fence_still_delivers_the_terminal_contract() {
    seed_secrets();
    let out = fenced_child_environment();

    for (key, value) in [("TERM", "xterm-256color"), ("TERM_PROGRAM", "Buzz")] {
        assert!(
            out.lines().any(|line| line == format!("{key}={value}")),
            "{key} missing from child environment:\n{out}"
        );
    }
    assert!(
        out.lines().any(|line| line.starts_with("PATH=")),
        "PATH missing from child environment:\n{out}"
    );
}

/// The fence must be exhaustive, not enumerated: a secret invented tomorrow
/// is excluded because it was never allowlisted. This is the property the
/// `feat/terminal` denylist could not offer.
#[test]
fn fence_excludes_keys_it_has_never_heard_of() {
    std::env::set_var("BUZZ_SOME_FUTURE_CREDENTIAL", CANARY);
    let out = fenced_child_environment();

    assert!(
        !out.contains("BUZZ_SOME_FUTURE_CREDENTIAL"),
        "an unknown key reached the child:\n{out}"
    );
}

/// `PATH` is constructed, not inherited, so Buzz's Hermit build toolchain
/// never becomes the user's shell toolchain.
///
/// The assertion is *reachability*, not a string comparison: we seed a
/// uniquely-named executable into this process's `PATH` and prove the child
/// cannot run it. A string check would pass a fence that inherited a
/// differently-spelled toolchain directory, and would fail a fence that
/// legitimately contained the substring; `command -v` asks the question the
/// user actually asks by typing a command name.
#[test]
fn child_path_is_free_of_buzz_toolchain() {
    let dir = std::env::temp_dir().join("buzz-terminal-path-canary");
    std::fs::create_dir_all(&dir).expect("canary dir");
    let canary = dir.join(CANARY_BIN);
    std::fs::write(&canary, "#!/bin/sh\necho canary\n").expect("write canary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&canary, std::fs::Permissions::from_mode(0o755))
            .expect("chmod canary");
    }

    // Stand in for Hermit activation: Buzz's own PATH leads with a directory
    // holding a tool the user does not have.
    std::env::set_var("PATH", format!("{}:/usr/bin:/bin", dir.display()));
    assert!(
        is_executable_file(&canary),
        "test setup: canary must be executable"
    );

    let shell = resolve_shell(std::env::var("SHELL").ok().as_deref());
    let out = child_environment(|cmd| {
        fence_env(cmd, &user_shell_path(), &shell);
    });
    let path_line = out
        .lines()
        .find(|line| line.starts_with("PATH="))
        .expect("child has a PATH");
    assert!(
        !path_line.contains("buzz-terminal-path-canary"),
        "Buzz's toolchain leaked into the child PATH: {path_line}"
    );

    // The reachability arm: run `command -v` for the canary inside the fence.
    let resolved = child_command(
        |cmd| fence_env(cmd, &user_shell_path(), &shell),
        &format!("command -v {CANARY_BIN} || echo CANARY_UNREACHABLE"),
    );
    assert!(
        resolved.contains("CANARY_UNREACHABLE"),
        "a Buzz-only executable was reachable from the child shell: {resolved}"
    );
}

/// `$SHELL` is honoured when it names an executable regular file.
#[test]
fn resolve_shell_prefers_a_valid_shell_env() {
    assert_eq!(resolve_shell(Some("/bin/sh")), "/bin/sh");
}

/// The other direction: an unset `$SHELL` must fall through to the **passwd
/// database**, not to the hardcoded fallback.
///
/// Asserting merely that the result is executable is vacuous — `/bin/sh` is
/// executable, so a resolver with the passwd step deleted entirely passes it.
/// Verified: mutant M8 (drop `.or_else(passwd_shell)`) survived that weaker
/// assertion. The property is *equality with the passwd entry*, and the
/// discriminating-power guard below refuses to pass silently on a machine
/// where the two candidates coincide.
#[test]
fn resolve_shell_falls_through_to_passwd_not_the_default() {
    let Some(passwd) = crate::shell::passwd_shell() else {
        panic!("no usable passwd shell; this gate cannot run on this machine");
    };
    assert_ne!(
        passwd, FALLBACK_SHELL,
        "passwd shell equals the fallback, so this test cannot tell the \
         passwd step from its absence; it must not report success"
    );
    assert_eq!(
        resolve_shell(None),
        passwd,
        "an unset $SHELL did not resolve to the passwd entry"
    );
}

/// `access(X_OK)` returns 0 for a directory, so a `$SHELL` pointing at one
/// passes portable-pty's own check and produces a child that dies with a Rust
/// runtime panic. Requiring an executable *regular file* is what closes it.
#[test]
fn resolve_shell_rejects_a_directory_that_passes_x_ok() {
    let dir = std::env::temp_dir();
    assert!(
        !is_executable_file(&dir),
        "a directory must not qualify as a shell"
    );
    assert_ne!(
        resolve_shell(dir.to_str()),
        dir.to_str().unwrap(),
        "a directory $SHELL was accepted; the child would abort on spawn"
    );
}

/// Raw mode bits are not effective executability: a self-owned regular file
/// at mode `0o010` has `mode & 0o111 != 0` while `access(X_OK)` fails and
/// running it gives `Permission denied`. The metadata half of the predicate
/// cannot see this; only the `access` half can.
#[test]
fn resolve_shell_rejects_a_file_the_user_cannot_execute() {
    use std::os::unix::fs::PermissionsExt;

    let path = fixture_file("buzz-terminal-group-only-exec", "#!/bin/sh\ntrue\n", 0o010);
    let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
    assert!(
        mode & 0o111 != 0,
        "test setup: some class must hold an execute bit, else this arm \
         cannot discriminate the mode check from the access check"
    );

    assert!(
        !is_executable_file(&path),
        "a file the effective user cannot execute was accepted as a shell"
    );
    assert_ne!(resolve_shell(path.to_str()), path.to_str().unwrap());
}

/// A non-executable regular file falls through as well.
#[test]
fn resolve_shell_rejects_a_non_executable_file() {
    let path = fixture_file("buzz-terminal-not-a-shell", "not a shell", 0o644);
    assert_ne!(resolve_shell(path.to_str()), path.to_str().unwrap());
}

/// The login convention is `-<basename>`, applied without inspecting the
/// shell's name. Any shell — including ones that do not exist yet — gets
/// login semantics from argv0 rather than from a flag we guessed.
#[test]
fn login_argv0_is_shell_neutral() {
    for (shell, expected) in [
        ("/bin/zsh", "-zsh"),
        ("/usr/local/bin/fish", "-fish"),
        ("/opt/nu/bin/nu", "-nu"),
        (FALLBACK_SHELL, "-sh"),
    ] {
        assert_eq!(login_argv0(shell), expected);
    }
}

/// The child must be told the shell we actually spawned, not the one Buzz
/// itself was launched under. Asserting `SHELL` is merely present would pass
/// for an inherited value, which is wrong in exactly the fallback cases.
#[test]
fn child_shell_is_the_resolved_shell_not_the_inherited_one() {
    std::env::set_var("SHELL", "/definitely/not/a/real/shell");
    let resolved = resolve_shell(std::env::var("SHELL").ok().as_deref());
    assert_ne!(
        resolved, "/definitely/not/a/real/shell",
        "test setup: the bogus shell must not resolve"
    );

    let out = child_environment(|cmd| fence_env(cmd, &user_shell_path(), &resolved));
    assert!(
        out.lines().any(|line| line == format!("SHELL={resolved}")),
        "child SHELL is not the resolved shell (expected {resolved}):\n{out}"
    );
    assert!(
        !out.contains("/definitely/not/a/real/shell"),
        "the inherited SHELL reached the child:\n{out}"
    );
}

/// The Windows allowlist obeys the same law as the Unix one: no secrets, ever.
/// Windows environment names are case-insensitive, so the comparison here is
/// too — an allowlist entry of `comspec` and one of `BUZZ_PRIVATE_KEY` differ
/// only in how obviously wrong they are.
#[test]
fn windows_allowlist_admits_no_secrets() {
    for entry in WINDOWS_INHERIT_ALLOWLIST {
        let upper = entry.to_ascii_uppercase();
        assert!(
            !SECRET_KEYS
                .iter()
                .any(|secret| secret.eq_ignore_ascii_case(entry)),
            "{entry} is a reserved secret and must not be allowlisted"
        );
        assert!(
            !upper.starts_with("BUZZ") && !upper.starts_with("NOSTR"),
            "{entry}: Buzz/Nostr-namespaced keys are exactly what the fence \
             exists to withhold"
        );
    }
}

/// The keys the spawn itself depends on must be present: `ComSpec` and
/// `USERPROFILE` feed `portable-pty`'s default-program and cwd resolution
/// (`cmdbuilder.rs:671-675`, `:609-611`), and their absence is the confirmed
/// `CreateProcessW "cmd.exe" in cwd None ... (os error 2)` failure. This
/// pins them so a future trim of the list cannot silently reintroduce it.
#[test]
fn windows_allowlist_covers_the_spawn_contract() {
    for key in ["ComSpec", "SystemRoot", "PATHEXT", "USERPROFILE"] {
        assert!(
            WINDOWS_INHERIT_ALLOWLIST.contains(&key),
            "{key} is load-bearing for the Windows spawn and must stay \
             allowlisted"
        );
    }
}

/// A rooted `ComSpec` naming a real file is the user's choice; honour it
/// verbatim, including the `C:/` forward-slash and `\\server` UNC spellings
/// Windows itself accepts.
#[test]
fn windows_shell_honours_a_valid_comspec() {
    for candidate in [
        r"C:\Windows\System32\cmd.exe",
        r"D:\shells\nu.exe",
        "C:/Windows/System32/cmd.exe",
        r"\\server\share\cmd.exe",
    ] {
        assert_eq!(
            pick_windows_shell(Some(candidate), Some(r"C:\Windows"), |path| path
                == candidate),
            candidate,
            "a rooted, existing ComSpec must be used verbatim"
        );
    }
}

/// A `ComSpec` naming a file that does not exist falls through to the
/// `SystemRoot`-derived absolute fallback — the case that used to become
/// `CreateProcessW`'s os error 2, because `lpApplicationName` is never
/// path-searched.
#[test]
fn windows_shell_falls_back_when_comspec_is_missing() {
    let picked = pick_windows_shell(
        Some(r"C:\definitely\not\real\cmd.exe"),
        Some(r"D:\CustomRoot"),
        |_| false,
    );
    assert_eq!(picked, r"D:\CustomRoot\System32\cmd.exe");
}

/// The hijack guard: a relative `ComSpec=cmd.exe` is rejected even when a file
/// by that name exists, because `lpApplicationName` would execute whatever
/// `cmd.exe` sits in Buzz's current directory. `is_file` answering `true` is
/// exactly the attack scenario, so this arm discriminates the rootedness
/// check from the existence check.
#[test]
fn windows_shell_rejects_a_relative_comspec_even_if_the_file_exists() {
    for candidate in ["cmd.exe", r"tools\cmd.exe", r"\cmd.exe", "C:cmd.exe"] {
        assert_eq!(
            pick_windows_shell(Some(candidate), Some(r"C:\Windows"), |_| true),
            r"C:\Windows\System32\cmd.exe",
            "{candidate}: a ComSpec not anchored to a drive or UNC share must \
             not be spawned"
        );
    }
}

/// With no `ComSpec` and no `SystemRoot` at all — a hand-stripped environment
/// — the resolver still produces an absolute path rather than a bare name.
#[test]
fn windows_shell_survives_an_empty_environment() {
    assert_eq!(
        pick_windows_shell(None, None, |_| false),
        r"C:\Windows\System32\cmd.exe"
    );
}

/// `SystemRoot` comes from the mutable process environment, so presence alone
/// does not make the fallback absolute. A relative or empty value must not
/// recreate the same current-directory hijack rejected for `ComSpec`.
#[test]
fn windows_shell_rejects_an_unrooted_system_root() {
    for system_root in ["", "Windows", r"\Windows", "C:Windows"] {
        assert_eq!(
            pick_windows_shell(None, Some(system_root), |_| false),
            r"C:\Windows\System32\cmd.exe",
            "{system_root:?}: an unrooted SystemRoot must use the absolute fallback"
        );
    }
}

/// The user's own entries survive, in their own order, ahead of the system
/// directories we guarantee. This is the regression that stripped `ssh`,
/// `git`, and every other installed tool from the Windows terminal.
#[test]
fn windows_path_keeps_what_the_user_installed() {
    let inherited = r"C:\Users\dev\.cargo\bin;C:\Program Files\Git\cmd;C:\Windows\System32\OpenSSH";
    let path = windows_shell_path(Some(inherited), Some(r"C:\Windows"));
    let entries: Vec<&str> = path.split(';').collect();

    assert_eq!(
        &entries[..3],
        &[
            r"C:\Users\dev\.cargo\bin",
            r"C:\Program Files\Git\cmd",
            r"C:\Windows\System32\OpenSSH",
        ],
        "inherited entries must keep their order and their precedence"
    );
    for required in [
        r"C:\Windows\System32",
        r"C:\Windows",
        r"C:\Windows\System32\Wbem",
        r"C:\Windows\System32\WindowsPowerShell\v1.0",
    ] {
        assert!(
            entries.contains(&required),
            "{required} must be reachable however sparse the inherited PATH"
        );
    }
}

/// A `PATH` that already names a system directory must not gain a second copy
/// of it, whatever case or trailing separator it was written with.
#[test]
fn windows_path_does_not_duplicate_system_directories() {
    let path = windows_shell_path(
        Some(r"c:\windows\system32\;C:\tools;C:\WINDOWS\System32\Wbem"),
        Some(r"C:\Windows"),
    );
    let entries: Vec<&str> = path.split(';').collect();

    let system32 = entries
        .iter()
        .filter(|entry| {
            entry
                .trim_end_matches('\\')
                .eq_ignore_ascii_case(r"C:\Windows\System32")
        })
        .count();
    assert_eq!(
        system32, 1,
        "System32 appears once, in the form the user wrote: {path}"
    );
    assert!(
        entries.contains(&r"C:\tools"),
        "de-duplication must not drop unrelated entries: {path}"
    );
}

/// An absent or empty inherited `PATH` still yields a usable shell rather than
/// an empty string.
#[test]
fn windows_path_survives_an_empty_environment() {
    for inherited in [None, Some(""), Some(";;")] {
        let path = windows_shell_path(inherited, None);
        assert_eq!(
            path,
            r"C:\Windows\System32;C:\Windows;C:\Windows\System32\Wbem;C:\Windows\System32\WindowsPowerShell\v1.0",
            "an empty inherited PATH falls back to the system directories alone"
        );
    }
}

/// The system directories appended to PATH obey the same rootedness rule as
/// the shell fallback; otherwise an invalid `SystemRoot` would still add
/// current-directory-relative command lookup locations.
#[test]
fn windows_path_rejects_an_unrooted_system_root() {
    let expected = r"C:\tools;C:\Windows\System32;C:\Windows;C:\Windows\System32\Wbem;C:\Windows\System32\WindowsPowerShell\v1.0";

    for system_root in ["", "Windows", r"\Windows", "C:Windows"] {
        assert_eq!(
            windows_shell_path(Some(r"C:\tools"), Some(system_root)),
            expected,
            "{system_root:?}: PATH must append only rooted system directories"
        );
    }
}

/// Guards the duplication of `RESERVED_ENV_KEYS` above. If the desktop crate
/// grows a new secret, this points at the file to update.
#[test]
fn reserved_keys_are_covered() {
    // The list lives in its own file because `build.rs` `include!`s the same
    // source (see `managed_agents/reserved_env_keys.rs`); read it there rather
    // than through the module that includes it.
    let source = include_str!("../../../src/managed_agents/reserved_env_keys.rs");
    let declared: Vec<&str> = source
        .lines()
        .skip_while(|line| !line.contains("RESERVED_ENV_KEYS"))
        .take_while(|line| !line.trim_start().starts_with("];"))
        .filter_map(|line| line.trim().strip_prefix('"'))
        .filter_map(|line| line.split('"').next())
        .filter(|key| {
            // Only identity/credential keys are in scope here: the rest of
            // RESERVED_ENV_KEYS guards agent-config override, which cannot
            // apply to a child that inherits nothing.
            key.contains("PRIVATE_KEY")
                || key.contains("AUTH_TAG")
                || key.contains("API_TOKEN")
                || key.contains("RELAY_URL")
        })
        .collect();

    assert!(!declared.is_empty(), "failed to parse RESERVED_ENV_KEYS");
    for key in declared {
        assert!(
            SECRET_KEYS.contains(&key),
            "{key} is a credential in RESERVED_ENV_KEYS but is not covered by \
             this crate's SECRET_KEYS; add it here"
        );
    }
}
