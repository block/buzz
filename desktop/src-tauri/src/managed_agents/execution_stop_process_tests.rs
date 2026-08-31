//! Actual native -> ACP -> agent -> MCP -> shell teardown, with fixture relay
//! and provider. No Desktop UI, command transport, real LLM or host certificate.
use super::stop_selected_generation;
use std::{
    fs,
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        // Unwinding must not kill an owner before its separately grouped
        // descendants have had a chance to drain.
        let _ = super::super::runtime::terminate_exact_owned_group(&mut self.0);
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn running(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}
fn wait_file(path: &Path) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(text) = fs::read_to_string(path) {
            if let Ok(pid) = text.trim().parse() {
                return pid;
            }
        }
        assert!(Instant::now() < deadline, "missing {}", path.display());
        std::thread::sleep(Duration::from_millis(25));
    }
}
fn parent(pid: u32) -> u32 {
    let output = Command::new("/bin/ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}
fn spawn_run(bin: &Path, dir: &Path, port: u32, generation: &str) -> OwnedChild {
    let log = fs::File::create(dir.join("harness.log")).unwrap();
    let child = Command::new(bin.join("buzz-acp"))
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("HOME", dir)
        .env("XDG_CONFIG_HOME", dir)
        .env("TMPDIR", dir)
        .env("BUZZ_RELAY_URL", format!("ws://127.0.0.1:{port}"))
        // Public deterministic test key, never an operator identity.
        .env("BUZZ_PRIVATE_KEY", "1".repeat(64))
        .env("BUZZ_AGENT_PROVIDER", "openai")
        .env("OPENAI_COMPAT_API_KEY", "fixture-not-a-secret")
        .env("OPENAI_COMPAT_MODEL", "fixture")
        .env(
            "OPENAI_COMPAT_BASE_URL",
            format!("http://127.0.0.1:{port}/v1"),
        )
        .env("BUZZ_AGENT_HINTS_ENABLED", "false")
        .env("BUZZ_AGENT_TOOL_TIMEOUT_SECS", "600")
        .env("BUZZ_MANAGED_AGENT_START_NONCE", generation)
        .env(
            "BUZZ_STOP_RECEIPT_PATH",
            super::stop_proof_path(&dir.join("harness.log"), generation),
        )
        .args([
            "--agent-command",
            bin.join("buzz-agent").to_str().unwrap(),
            "--agent-args",
            "",
            "--mcp-command",
            bin.join("buzz-dev-mcp").to_str().unwrap(),
            "--heartbeat-interval",
            "10",
            "--heartbeat-prompt",
            "Run the fixture shell",
            "--no-memory",
            "--no-presence",
            "--no-base-prompt",
        ])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(log.try_clone().unwrap())
        .stderr(log)
        .process_group(0)
        .spawn()
        .unwrap();
    OwnedChild(child)
}
fn pids(dir: &Path, root: u32) -> Vec<u32> {
    let shell = wait_file(&dir.join("shell.pid"));
    let grandchild = wait_file(&dir.join("grandchild.pid"));
    let mcp = wait_file(&dir.join("mcp.pid"));
    let agent = parent(mcp);
    assert_eq!(parent(agent), root);
    assert_eq!(parent(shell), mcp);
    assert_eq!(parent(grandchild), shell);
    let pids = vec![root, agent, mcp, shell, grandchild];
    assert!(pids.iter().all(|pid| running(*pid)));
    let joined = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let tree = Command::new("/bin/ps")
        .args(["-o", "pid=,ppid=,pgid=,comm=", "-p", &joined])
        .output()
        .unwrap();
    assert!(tree.status.success());
    fs::write(dir.join("process-tree.txt"), &tree.stdout).unwrap();
    let text = String::from_utf8(tree.stdout).unwrap();
    eprintln!("{text}");
    for line in text.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        let pid: u32 = fields[0].parse().unwrap();
        let pgid: u32 = fields[2].parse().unwrap();
        assert_eq!(pgid, if pid == grandchild { shell } else { pid });
    }
    pids
}
fn assert_gone(pids: &[u32]) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while pids.iter().any(|pid| running(*pid)) {
        assert!(
            Instant::now() < deadline,
            "surviving selected descendants: {pids:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[test]
#[ignore = "requires current buzz-acp/agent/dev-mcp binaries and Node; see docs/host-execution.md"]
fn selected_generation_process_chain() {
    let bin = std::path::PathBuf::from(
        std::env::var_os("BUZZ_STOP_CHAIN_BIN_DIR").expect("set BUZZ_STOP_CHAIN_BIN_DIR"),
    );
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let temp = tempfile::tempdir().unwrap();
    // Keep logs on request for externally inspectable evidence, never secrets.
    let dir = std::env::var_os("BUZZ_STOP_CHAIN_ARTIFACTS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| temp.path().to_owned());
    fs::create_dir_all(&dir).unwrap();
    let ready = dir.join("fixture.port");
    let _ = fs::remove_file(&ready);
    let _fixture = OwnedChild(
        Command::new("node")
            .arg(repo.join("scripts/fixtures/stop-owner-chain.mjs"))
            .arg(&ready)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let port = wait_file(&ready);
    // Same agent identity, different relay placement. Never start two current
    // generations in one production placement just to manufacture a peer.
    let peer_ready = dir.join("peer-fixture.port");
    let _ = fs::remove_file(&peer_ready);
    let _peer_fixture = OwnedChild(
        Command::new("node")
            .arg(repo.join("scripts/fixtures/stop-owner-chain.mjs"))
            .arg(&peer_ready)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let peer_port = wait_file(&peer_ready);
    let selected_dir = dir.join("selected");
    let peer_dir = dir.join("peer");
    fs::create_dir(&selected_dir).unwrap();
    fs::create_dir(&peer_dir).unwrap();
    let selected_generation = "aa".repeat(16);
    let peer_generation = "bb".repeat(16);
    let mut selected = spawn_run(&bin, &selected_dir, port, &selected_generation);
    let mut peer = spawn_run(&bin, &peer_dir, peer_port, &peer_generation);
    let selected_pids = pids(&selected_dir, selected.0.id());
    let peer_pids = pids(&peer_dir, peer.0.id());
    eprintln!("selected root/agent/MCP/shell/grandchild={selected_pids:?}; peer={peer_pids:?}");
    assert!(
        stop_selected_generation(&mut selected.0, &selected_generation, &peer_generation).is_err()
    );
    assert!(selected_pids
        .iter()
        .chain(&peer_pids)
        .all(|pid| running(*pid)));
    let start = Instant::now();
    stop_selected_generation(&mut selected.0, &selected_generation, &selected_generation).unwrap();
    assert_gone(&selected_pids);
    let key = nostr::Keys::parse(&"1".repeat(64))
        .unwrap()
        .public_key()
        .to_hex();
    let proof = super::stop_proof_path(&selected_dir.join("harness.log"), &selected_generation);
    assert!(
        super::verified_stop_proof(
            &proof,
            &key,
            &format!("ws://127.0.0.1:{port}"),
            &selected_generation
        ),
        "missing or invalid supported proof: {}",
        proof.display()
    );
    assert!(!super::verified_stop_proof(
        &proof,
        &key,
        &format!("ws://127.0.0.1:{peer_port}"),
        &selected_generation
    ));
    assert!(!super::verified_stop_proof(
        &proof,
        &key,
        &format!("ws://127.0.0.1:{port}"),
        &peer_generation
    ));
    assert!(peer_pids.iter().all(|pid| running(*pid)));
    eprintln!(
        "selected teardown {:?}; every peer process preserved",
        start.elapsed()
    );
    assert!(
        stop_selected_generation(&mut selected.0, &selected_generation, &selected_generation)
            .is_err()
    );
    assert!(peer_pids.iter().all(|pid| running(*pid)));
    stop_selected_generation(&mut peer.0, &peer_generation, &peer_generation).unwrap();
    assert_gone(&peer_pids);
    for path in [selected_dir, peer_dir] {
        let log = fs::read_to_string(path.join("harness.log")).unwrap();
        assert!(
            log.contains("agent connection closed and child reaped"),
            "{log}"
        );

        assert!(!log.contains("teardown unconfirmed"), "{log}");
        assert!(!log.contains("killpg MCP"), "{log}");
    }
}
