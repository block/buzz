//! Unit tests for `managed_agents/storage/logs.rs`.
//!
//! Kept in a sibling file so `logs.rs` stays closer to the 1000-line gate;
//! `#[path]`-included from there.

use std::fs::File;
use std::io::Write as _;
use std::path::Path;

use tempfile::NamedTempFile;

fn write_log(content: &str) -> NamedTempFile {
    let mut file = NamedTempFile::new().expect("temp log");
    file.write_all(content.as_bytes()).expect("write log");
    file
}
#[test]
fn meaningful_agent_error_from_log_promotes_wrapped_llm_auth() {
    let file =
        write_log("noise\nAgent reported error (code -32001): llm auth: 401 unauthorized: ...\n");
    let result = super::meaningful_agent_error_from_log(file.path()).unwrap();
    assert!(result.message.contains("llm auth"));
    assert_eq!(result.code, Some(-32001));
}

#[test]
fn meaningful_agent_error_from_log_promotes_unwrapped_llm_auth() {
    let file = write_log("noise\nllm auth: denied\n");
    let result = super::meaningful_agent_error_from_log(file.path()).unwrap();
    assert_eq!(result.message, "Agent reported error: llm auth: denied");
    assert_eq!(result.code, Some(-32001));
}

#[test]
fn meaningful_agent_error_from_log_promotes_bare_model_not_found() {
    let file = write_log("noise\nllm model not found: (some-model) 404\n");
    let result = super::meaningful_agent_error_from_log(file.path()).unwrap();
    assert_eq!(
        result.message,
        "Agent reported error: llm model not found: (some-model) 404"
    );
    assert_eq!(result.code, Some(-32002));
}

#[test]
fn meaningful_agent_error_from_log_promotes_legacy_format() {
    let file = write_log("noise\nAgent reported error: llm: 500 internal\n");
    let result = super::meaningful_agent_error_from_log(file.path()).unwrap();
    assert_eq!(result.message, "Agent reported error: llm: 500 internal");
    assert_eq!(result.code, None);
}

#[test]
fn meaningful_agent_error_from_log_does_not_promote_midline_auth_text() {
    let file = write_log("noise before llm auth: denied\n");
    assert!(super::meaningful_agent_error_from_log(file.path()).is_none());
}

#[test]
fn strips_ansi_from_typical_tracing_line() {
    let input = "\x1b[2m2026-05-27T15:16:32\x1b[0m \x1b[32m INFO\x1b[0m \x1b[2mbuzz_acp\x1b[0m\x1b[2m:\x1b[0m starting";
    assert_eq!(
        strip_ansi_escapes::strip_str(input),
        "2026-05-27T15:16:32  INFO buzz_acp: starting"
    );
}
// ── harness-log selection tests ────────────────────────────────────────

const PUBKEY_A: &str = "aa11223344556677889900aabbccddeeff00112233445566778899aabbccddee";
const PUBKEY_B: &str = "bb11223344556677889900aabbccddeeff00112233445566778899aabbccddee";

/// Write `name` into `dir` and stamp it `age_secs` before now, so selection
/// order is asserted against explicit mtimes rather than write order.
fn write_log_in(dir: &Path, name: &str, age_secs: u64) {
    let path = dir.join(name);
    let file = File::create(&path).expect("create log");
    file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(age_secs))
        .expect("stamp mtime");
}

#[test]
fn newest_agent_log_prefers_pair_scoped_when_it_is_freshest() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_log_in(dir.path(), &format!("{PUBKEY_A}.log"), 600);
    write_log_in(dir.path(), &format!("{PUBKEY_A}__cafe.log"), 5);

    assert_eq!(
        super::newest_agent_log_in_dir(dir.path(), PUBKEY_A),
        Some(dir.path().join(format!("{PUBKEY_A}__cafe.log")))
    );
}

#[test]
fn newest_agent_log_prefers_legacy_when_it_is_freshest() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_log_in(dir.path(), &format!("{PUBKEY_A}.log"), 5);
    write_log_in(dir.path(), &format!("{PUBKEY_A}__cafe.log"), 600);

    assert_eq!(
        super::newest_agent_log_in_dir(dir.path(), PUBKEY_A),
        Some(dir.path().join(format!("{PUBKEY_A}.log"))),
        "mtime decides, not the filename shape"
    );
}

#[test]
fn newest_agent_log_finds_sole_pair_scoped_log() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_log_in(dir.path(), &format!("{PUBKEY_A}__cafe.log"), 5);

    assert_eq!(
        super::newest_agent_log_in_dir(dir.path(), PUBKEY_A),
        Some(dir.path().join(format!("{PUBKEY_A}__cafe.log")))
    );
}

#[test]
fn newest_agent_log_picks_freshest_of_several_relays() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_log_in(dir.path(), &format!("{PUBKEY_A}__aaa.log"), 900);
    write_log_in(dir.path(), &format!("{PUBKEY_A}__bbb.log"), 5);
    write_log_in(dir.path(), &format!("{PUBKEY_A}__ccc.log"), 300);

    assert_eq!(
        super::newest_agent_log_in_dir(dir.path(), PUBKEY_A),
        Some(dir.path().join(format!("{PUBKEY_A}__bbb.log")))
    );
}

#[test]
fn newest_agent_log_ignores_other_agents_and_non_log_files() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_log_in(dir.path(), &format!("{PUBKEY_B}__cafe.log"), 1);
    write_log_in(dir.path(), &format!("{PUBKEY_A}__cafe.log.gz"), 2);
    write_log_in(dir.path(), &format!("{PUBKEY_A}__cafe.log"), 600);

    assert_eq!(
        super::newest_agent_log_in_dir(dir.path(), PUBKEY_A),
        Some(dir.path().join(format!("{PUBKEY_A}__cafe.log"))),
        "a fresher log belonging to another agent must never be selected"
    );
}

#[test]
fn newest_agent_log_is_none_when_agent_has_no_logs() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_log_in(dir.path(), &format!("{PUBKEY_B}.log"), 1);

    assert_eq!(super::newest_agent_log_in_dir(dir.path(), PUBKEY_A), None);
}

#[test]
fn newest_agent_log_is_none_when_dir_is_missing() {
    let dir = tempfile::tempdir().expect("temp dir");
    let missing = dir.path().join("absent");

    assert_eq!(super::newest_agent_log_in_dir(&missing, PUBKEY_A), None);
}

#[test]
fn newest_agent_log_breaks_mtime_ties_deterministically() {
    let dir = tempfile::tempdir().expect("temp dir");
    write_log_in(dir.path(), &format!("{PUBKEY_A}__aaa.log"), 60);
    write_log_in(dir.path(), &format!("{PUBKEY_A}__bbb.log"), 60);

    assert_eq!(
        super::newest_agent_log_in_dir(dir.path(), PUBKEY_A),
        Some(dir.path().join(format!("{PUBKEY_A}__bbb.log"))),
        "equal mtimes must resolve to the same file on every read_dir order"
    );
}
// ── install logs ─────────────────────────────────────────────────────────────

/// Install output can carry registry tokens and proxy credentials a failing
/// installer echoed, and the file is written unattended. `0o600` must come from
/// the create itself: a post-write `chmod` leaves a window where the umask
/// decides, and a crash inside it leaves the log readable to other local users.
#[cfg(unix)]
#[test]
fn install_log_is_created_owner_only_without_post_write_chmod() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("install-goose.log");

    let mut file = super::open_install_log_file(&path).expect("open install log");
    file.write_all(b"npm ERR!\n").expect("write");

    let mode = std::fs::metadata(&path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600, "install logs must be owner-only");
}

/// A run starts a new current file and keeps the previous run as `.1`, so the
/// two runs are never mixed and the history on disk stays bounded at two.
#[test]
fn install_log_session_keeps_the_previous_run_as_dot_one() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("install-goose.log");

    let mut first = super::start_install_log_session(&path).expect("first session");
    first.write_all(b"run-one\n").expect("write");
    let mut second = super::start_install_log_session(&path).expect("second session");
    second.write_all(b"run-two\n").expect("write");

    assert_eq!(
        std::fs::read_to_string(&path).expect("read current"),
        "run-two\n",
        "the current file must hold only the newest run"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("install-goose.log.1")).expect("read .1"),
        "run-one\n",
        "the previous run must be preserved as .1"
    );
}

/// The third run must still rotate when `.1` already exists. Windows `rename`
/// does not replace its destination, so a rename-only rotation silently stops
/// working here and leaves the current file to grow across every later run —
/// the old `.1` is removed first precisely so this cannot happen. Runs on the
/// Windows target too: this is the path that fails there.
#[test]
fn install_log_session_replaces_an_existing_dot_one() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("install-goose.log");
    let rotated = dir.path().join("install-goose.log.1");
    // Seed the state a rename-only rotation cannot get out of: both files exist.
    std::fs::write(&path, b"previous-run\n").expect("seed current");
    std::fs::write(&rotated, b"ancient-run\n").expect("seed .1");

    let mut file = super::start_install_log_session(&path).expect("session");
    file.write_all(b"fresh-run\n").expect("write");

    assert_eq!(
        std::fs::read_to_string(&path).expect("read current"),
        "fresh-run\n",
        "the current file must restart even when .1 was already present"
    );
    assert_eq!(
        std::fs::read_to_string(&rotated).expect("read .1"),
        "previous-run\n",
        ".1 must be replaced by the run that just ended, not kept"
    );
}

/// Records written after the session starts append to it — a run's later
/// records must not erase its earlier ones.
#[test]
fn install_log_appends_within_a_session() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("install-goose.log");

    let mut session = super::start_install_log_session(&path).expect("session");
    session.write_all(b"header\n").expect("write");
    for record in ["first\n", "second\n"] {
        let mut file = super::open_install_log_file(&path).expect("open install log");
        file.write_all(record.as_bytes()).expect("write");
    }

    assert_eq!(
        std::fs::read_to_string(&path).expect("read back"),
        "header\nfirst\nsecond\n"
    );
}

/// A runtime id becomes part of a filename. Ids reach this from user-defined
/// custom harnesses as well as the catalog, so anything that could traverse or
/// escape the logs directory is rejected rather than sanitized — a rejected id
/// simply means no log, while a silently rewritten one could collide with
/// another runtime's log.
#[test]
fn install_log_filename_rejects_ids_that_would_escape_the_logs_dir() {
    for id in [
        "../../etc/passwd",
        "goose/../../evil",
        "sub/dir",
        "back\\slash",
        "with.dot",
        "",
    ] {
        assert!(
            super::install_log_filename(id).is_err(),
            "id {id:?} must not be accepted as a filename component"
        );
    }
}

/// Ordinary catalog and custom-harness ids are accepted — the guard must not
/// reject the ids it exists to serve.
#[test]
fn install_log_filename_accepts_ordinary_runtime_ids() {
    for id in ["goose", "claude-code", "buzz_agent", "codex2"] {
        assert_eq!(
            super::install_log_filename(id).expect("id must be usable in a log filename"),
            format!("install-{id}.log")
        );
    }
}
