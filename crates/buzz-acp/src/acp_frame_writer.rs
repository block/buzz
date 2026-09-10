//! Cancellation-safe ownership of the sole ACP stdin writer.

use tokio::io::{AsyncWrite, AsyncWriteExt};

/// Lend the writer back only after a complete frame has been written. Dropping
/// this future (control select, outer deadline, or task abort) or returning an
/// I/O error drops the owned writer instead. For ChildStdin this closes the pipe;
/// the empty slot also rejects every subsequent request/cleanup write as a
/// transport error, using the existing pool retirement and retry policy.
pub(super) async fn write_frame<W: AsyncWrite + Unpin>(
    slot: &mut Option<W>,
    body: &[u8],
) -> std::io::Result<()> {
    let mut writer = slot.take().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "ACP stdin closed by an incomplete or failed frame write",
        )
    })?;
    writer.write_all(body).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await?;
    *slot = Some(writer);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::AsyncReadExt;

    // A bounded duplex is a deterministic AsyncWrite seam: capacity pins the
    // exact body/LF suspension point; the production ChildStdin test below
    // exercises the real transport and prompt/cleanup ownership as well.
    async fn aborted_frame(capacity: usize) {
        let (writer, mut reader) = tokio::io::duplex(capacity);
        let mut slot = Some(writer);
        assert!(
            tokio::time::timeout(Duration::from_millis(20), write_frame(&mut slot, b"{}"))
                .await
                .is_err()
        );
        assert!(slot.is_none());
        assert_eq!(
            write_frame(&mut slot, b"cancel").await.unwrap_err().kind(),
            std::io::ErrorKind::BrokenPipe
        );
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes, b"{}"[..capacity]);
    }

    #[tokio::test]
    async fn aborted_body_closes_before_reuse() {
        aborted_frame(1).await;
    }

    #[tokio::test]
    async fn aborted_lf_closes_before_reuse() {
        aborted_frame(2).await;
    }

    #[tokio::test]
    async fn io_failure_closes_before_reuse() {
        let (writer, reader) = tokio::io::duplex(4);
        let mut slot = Some(writer);
        drop(reader);
        assert!(write_frame(&mut slot, b"{}").await.is_err());
        assert!(slot.is_none());
        assert!(write_frame(&mut slot, b"cancel").await.is_err());
    }

    #[tokio::test]
    async fn io_failure_after_prefix_closes_before_reuse() {
        let (writer, mut reader) = tokio::io::duplex(1);
        let mut slot = Some(writer);
        let (result, ()) = tokio::join!(write_frame(&mut slot, b"{}"), async move {
            assert_eq!(reader.read_u8().await.unwrap(), b'{');
            drop(reader);
        });
        assert!(result.is_err());
        assert!(slot.is_none());
        assert!(write_frame(&mut slot, b"cancel").await.is_err());
    }

    #[tokio::test]
    async fn completed_frames_keep_writer() {
        let (writer, mut reader) = tokio::io::duplex(32);
        let mut slot = Some(writer);
        write_frame(&mut slot, b"{}").await.unwrap();
        write_frame(&mut slot, b"{\"cancel\":true}").await.unwrap();
        drop(slot);
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.unwrap();
        assert_eq!(bytes, b"{}\n{\"cancel\":true}\n");
    }
    async fn wait_for(path: &std::path::Path) {
        tokio::time::timeout(Duration::from_secs(5), async {
            while !path.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("child fixture did not reach checkpoint");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn real_pipe_aborted_prompt_rejects_cleanup_and_later_requests() {
        use super::super::{AcpClient, AcpError};
        let dir = std::env::temp_dir().join(format!("buzz-frame-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let release = dir.join("release");
        let capture = dir.join("capture");
        let done = dir.join("done");
        let script = format!(
            "while [ ! -e '{}' ]; do sleep 0.01; done; cat > '{}'; touch '{}'; sleep 10",
            release.display(),
            capture.display(),
            done.display()
        );
        let mut client = AcpClient::spawn("bash", &["-c".into(), script], &[], false)
            .await
            .unwrap();
        let prompt = "x".repeat(300_000);
        // Recreates the pool's control-select ownership, not its relay loop.
        // The child does not read until AFTER cleanup attempts have returned.
        tokio::select! {
            biased;
            result = client.session_prompt_with_idle_timeout(
                "sess-test", &prompt, Duration::from_secs(60), Duration::from_secs(60)
            ) => panic!("large write should remain blocked: {result:?}"),
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
        assert!(client.has_in_flight_prompt());
        assert!(
            matches!(client.cancel_with_cleanup_grace("sess-test", Duration::from_secs(5)).await,
            Err(AcpError::Io(ref e)) if e.kind() == std::io::ErrorKind::BrokenPipe)
        );
        assert!(matches!(
            client.session_cancel("sess-test").await,
            Err(AcpError::Io(_))
        ));
        assert!(matches!(client.initialize().await, Err(AcpError::Io(_))));
        std::fs::write(&release, []).unwrap();
        wait_for(&done).await;
        let bytes = std::fs::read(&capture).unwrap();
        assert!(!bytes.is_empty(), "real pipe must contain a written prefix");
        assert!(bytes.len() < prompt.len());
        assert!(!bytes.contains(&b'\n'), "no cleanup frame may be appended");
        client.shutdown().await;
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn real_pipe_completed_prompt_still_allows_normal_cancel() {
        use super::super::AcpClient;
        let dir = std::env::temp_dir().join(format!("buzz-frame-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let capture = dir.join("capture");
        let ready = dir.join("ready");
        let script = format!(
            "read -r line; printf '%s\\n' \"$line\" > '{}'; touch '{}'; \
             read -r line; printf '%s\\n' \"$line\" >> '{}'; \
             printf '%s\\n' '{{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{{\"stopReason\":\"cancelled\"}}}}'; sleep 10",
            capture.display(),
            ready.display(),
            capture.display()
        );
        let mut client = AcpClient::spawn("bash", &["-c".into(), script], &[], false)
            .await
            .unwrap();
        tokio::select! {
            biased;
            result = client.session_prompt_with_idle_timeout(
                "sess-test", "hello", Duration::from_secs(60), Duration::from_secs(60)
            ) => panic!("response should wait for cancel: {result:?}"),
            _ = wait_for(&ready) => {}
        }
        assert!(
            client
                .cancel_with_cleanup_grace("sess-test", Duration::from_secs(5))
                .await
                .is_ok()
        );
        let text = std::fs::read_to_string(&capture).unwrap();
        let frames: Vec<serde_json::Value> = text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["method"], "session/prompt");
        assert_eq!(frames[1]["method"], "session/cancel");
        assert!(client.stdin.is_some());
        client.shutdown().await;
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn application_error_does_not_close_transport() {
        use super::super::{AcpClient, AcpError};
        let script = r#"read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":0,"error":{"code":-32602,"message":"bad input"}}'
read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"stopReason":"end_turn"}}'
sleep 10"#;
        let mut client = AcpClient::spawn("bash", &["-c".into(), script.into()], &[], false)
            .await
            .unwrap();
        assert!(matches!(
            client
                .session_prompt_with_idle_timeout(
                    "sess-test",
                    "first",
                    Duration::from_secs(5),
                    Duration::from_secs(5)
                )
                .await,
            Err(AcpError::AgentError { .. })
        ));
        assert!(
            client
                .session_prompt_with_idle_timeout(
                    "sess-test",
                    "second",
                    Duration::from_secs(5),
                    Duration::from_secs(5)
                )
                .await
                .is_ok()
        );
        client.shutdown().await;
    }
}
