use super::*;
use std::time::Duration;

async fn shell(script: &str) -> AcpClient {
    AcpClient::spawn("/bin/sh", &["-c".into(), script.into()], &[], false)
        .await
        .unwrap()
}

#[tokio::test]
async fn shutdown_closes_stdin_drains_full_stdout_and_reaps() {
    let mut client = shell("cat >/dev/null; head -c 524288 /dev/zero; exit 0").await;
    client.shutdown().await;
    assert!(client.child.try_wait().unwrap().unwrap().success());
    assert!(client.write_ndjson(&serde_json::json!({})).await.is_err());
    // Repeated shutdown cannot signal a recycled PID.
    client.shutdown().await;
}

#[tokio::test]
async fn shutdown_escalates_hung_owner_but_preserves_peer() {
    let mut selected = shell("exec sleep 600").await;
    let mut peer = shell("exec sleep 600").await;
    selected
        .shutdown_with_grace(Duration::from_millis(30))
        .await;
    assert!(!selected.child.try_wait().unwrap().unwrap().success());
    assert!(peer.child.try_wait().unwrap().is_none());
    peer.shutdown_with_grace(Duration::from_millis(30)).await;
}

#[tokio::test]
async fn supported_result_requires_successful_root_exit_and_is_idempotent() {
    for (exit, expected) in [(0, true), (7, false)] {
        let mut client = shell(&format!(r#"
            read init
            echo '{{"jsonrpc":"2.0","id":0,"result":{{"protocolVersion":1,"_meta":{{"buzzOwnedWorkShutdown":1}}}}}}'
            read stop
            echo '{{"jsonrpc":"2.0","id":1,"result":{{"v":1,"ownedWorkStopped":true}}}}'
            exit {exit}
        "#)).await;
        client.initialize().await.unwrap();
        client.shutdown().await;
        assert_eq!(client.teardown_confirmed, expected);
        client.shutdown().await;
        assert_eq!(client.teardown_confirmed, expected);
    }
    let mut unsupported = shell("cat >/dev/null; exit 0").await;
    unsupported.shutdown().await;
    assert!(!unsupported.teardown_confirmed);
}
