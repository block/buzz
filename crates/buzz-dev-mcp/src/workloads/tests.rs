use super::*;

#[tokio::test]
async fn closed_owner_rejects_late_admission_and_waits_for_existing_work() {
    let owner = Workloads::default();
    let work = owner.enter().unwrap();
    owner.close();
    assert!(owner.enter().is_none());
    assert!(owner.cancel.is_cancelled());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), owner.drain())
            .await
            .is_err()
    );
    drop(work);
    owner.drain().await.unwrap();
    owner.drain().await.unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn mcp_eof_reaps_separate_shell_group_without_stopping_peer() {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("children");
    let state = std::sync::Arc::new(
        crate::shell::SharedState::new(dir.path().into(), crate::shim::Shim::install().unwrap())
            .unwrap(),
    );
    let (client, server) = tokio::io::duplex(64 * 1024);
    let (reader, writer) = tokio::io::split(server);
    let owned = state.clone();
    let server_task = tokio::spawn(async move {
        crate::serve_connection(owned, reader, writer)
            .await
            .map_err(|e| e.to_string())
    });
    let (mut reader, mut writer) = tokio::io::split(client);
    let output = tokio::spawn(async move {
        let mut bytes = vec![];
        reader.read_to_end(&mut bytes).await.unwrap();
        bytes
    });
    for message in [
        serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
            "protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"teardown-test","version":"1"}
        }}),
        serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"shell","arguments":{"command":format!("sleep 30 & echo \"$$ $!\" > '{}'; wait", marker.display()),"timeout_ms":10000}
        }}),
    ] {
        writer
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
    }
    let mut peer = tokio::process::Command::new("/bin/sleep")
        .arg("30")
        .process_group(0)
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let pids = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(text) = std::fs::read_to_string(&marker) {
                let pids: Vec<i32> = text
                    .split_whitespace()
                    .filter_map(|p| p.parse().ok())
                    .collect();
                if pids.len() == 2 {
                    break pids;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    let group = pids.as_ref().ok().map(|pids| {
        std::process::Command::new("/bin/ps")
            .args(["-o", "pgid=", "-p", &pids[0].to_string()])
            .output()
            .unwrap()
    });
    // Always trigger cleanup before asserting fixture readiness.
    writer.shutdown().await.unwrap();
    drop(writer);
    // Finish before rmcp's five-second EOF response-drain timeout: cleanup
    // starts at EOF, not only after the transport has already given up.
    let result = tokio::time::timeout(Duration::from_secs(4), server_task).await;
    state.workloads.close();
    let peer_alive = peer.try_wait().unwrap().is_none();
    peer.kill().await.unwrap();
    peer.wait().await.unwrap();
    let pids = pids.unwrap();
    let group = group.unwrap();
    assert!(group.status.success());
    assert_eq!(
        String::from_utf8_lossy(&group.stdout).trim(),
        pids[0].to_string()
    );
    result.unwrap().unwrap().unwrap();
    output.await.unwrap();
    assert!(peer_alive, "a separate placement is not a teardown target");
    // A real shell and its child occupied their own group, not the MCP/root
    // group. EOF must make the natural shell owner cancel and reap them.
    for pid in pids {
        let gone = tokio::time::timeout(Duration::from_secs(3), async {
            while kill(Pid::from_raw(pid), None).is_ok() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        if gone.is_err() {
            let _ = kill(Pid::from_raw(pid), Signal::SIGKILL);
        }
        assert!(gone.is_ok(), "managed child {pid} survived MCP EOF");
    }
    assert!(state.workloads.enter().is_none());
}

#[tokio::test]
async fn abandoned_owned_child_prevents_success_even_when_task_count_is_zero() {
    let owner = Workloads::default();
    drop(owner.child());
    assert!(owner.drain().await.is_err());
    assert!(owner.drain().await.is_err());
}
