//! Real harness startup cancellation: no relay/provider or user configuration.
#![cfg(unix)]
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct TestDir(std::path::PathBuf);
impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("buzz-stop-startup-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Owner(Child);
impl Drop for Owner {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn wait_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !path.exists() {
        assert!(Instant::now() < deadline, "missing {}", path.display());
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn gone(pid: &str) -> bool {
    !Command::new("/bin/kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}
fn exercise(partial: bool, relay_error: bool) {
    let dir = TestDir::new();
    let script = dir.path().join("agent.sh");
    fs::write(
        &script,
        r#"
        if mkdir "$HOME/first" 2>/dev/null; then slot=first; else slot=second; fi
        echo $$ > "$HOME/$slot.pid"
        if [ "$MODE" = relay ] || { [ "$MODE" = partial ] && [ "$slot" = first ]; }; then
            read init
            echo '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1}}'
        fi
        cat >/dev/null
        echo done > "$HOME/$slot.done"
    "#,
    )
    .unwrap();
    let log = fs::File::create(dir.path().join("harness.log")).unwrap();
    let mut owner = Owner(
        Command::new(env!("CARGO_BIN_EXE_buzz-acp"))
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", dir.path())
            .env("XDG_CONFIG_HOME", dir.path())
            .env("TMPDIR", dir.path())
            .env(
                "MODE",
                if relay_error {
                    "relay"
                } else if partial {
                    "partial"
                } else {
                    "early"
                },
            )
            .env("BUZZ_PRIVATE_KEY", "1".repeat(64))
            .args([
                "--relay-url",
                if relay_error {
                    "not-a-url"
                } else {
                    "ws://127.0.0.1:1"
                },
                "--agent-command",
                "/bin/sh",
                "--agent-args",
                script.to_str().unwrap(),
                "--agents",
                if partial { "2" } else { "1" },
                "--no-memory",
                "--no-presence",
            ])
            .current_dir(dir.path())
            .stdin(Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap(),
    );
    wait_file(
        &dir.path()
            .join(if partial { "second.pid" } else { "first.pid" }),
    );
    if !relay_error {
        assert!(Command::new("/bin/kill")
            .args(["-TERM", &owner.0.id().to_string()])
            .status()
            .unwrap()
            .success());
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while owner.0.try_wait().unwrap().is_none() {
        assert!(
            Instant::now() < deadline,
            "{}",
            fs::read_to_string(dir.path().join("harness.log")).unwrap()
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    for slot in if partial {
        vec!["first", "second"]
    } else {
        vec!["first"]
    } {
        assert_eq!(
            fs::read_to_string(dir.path().join(format!("{slot}.done")))
                .unwrap()
                .trim(),
            "done"
        );
        assert!(gone(
            fs::read_to_string(dir.path().join(format!("{slot}.pid")))
                .unwrap()
                .trim()
        ));
    }
}
#[test]
fn sigterm_during_eager_initialize_drains_child() {
    exercise(false, false);
}
#[test]
fn sigterm_during_partial_pool_initialize_drains_all_slots() {
    exercise(true, false);
}
#[test]
fn relay_startup_error_drains_initialized_pool() {
    exercise(false, true);
}
