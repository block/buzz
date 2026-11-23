use super::{ProviderError, QUICK_TIMEOUT};
#[cfg(test)]
use std::sync::Arc;
use std::time::Duration;
#[cfg(any(not(target_os = "linux"), test))]
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;

/// `lsof` can take seconds on a busy Mac.
const SAME_USER_TIMEOUT: Duration = Duration::from_secs(5);
const SAME_USER_RETRY_DELAY: Duration = Duration::from_millis(50);
#[cfg(any(not(target_os = "linux"), test))]
const MAX_LSOF_BYTES: u64 = 1024 * 1024;

// ---------------------------------------------------------------------------
// Same-user check
// ---------------------------------------------------------------------------

/// Connect to the provider port. With `check`, confirm by numeric user ID that
/// a process of this user holds the server side of this very connection, and
/// refuse to use the connection otherwise.
pub(super) async fn connect(port: u16, check: bool) -> Result<TcpStream, ProviderError> {
    let stream = tokio::time::timeout(QUICK_TIMEOUT, TcpStream::connect(("127.0.0.1", port)))
        .await
        .map_err(|_| ProviderError::Transport("connection timed out".into()))?
        .map_err(|e| ProviderError::Transport(format!("not reachable: {}", e.kind())))?;
    if check {
        let local = stream
            .local_addr()
            .map_err(|_| ProviderError::Transport("no local address".into()))?
            .port();
        #[cfg(test)]
        if crate::desktop_provider::tests::SAME_USER_HANGS
            .try_with(|_| ())
            .is_ok()
        {
            return Err(ProviderError::Unconfirmed);
        }
        confirm_same_user(|| server_side_uid(port, local)).await?;
    }
    Ok(stream)
}

/// The accepted socket may not be observable yet. Retry only missing proof,
/// on the same open connection and within one deadline; errors and wrong UIDs are final.
async fn confirm_same_user<F>(mut owner: impl FnMut() -> F) -> Result<(), ProviderError>
where
    F: std::future::Future<Output = Result<Option<u32>, ()>>,
{
    let deadline = tokio::time::Instant::now() + SAME_USER_TIMEOUT;
    let proof = async {
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(ProviderError::Unconfirmed);
            }
            let found = owner().await;
            if tokio::time::Instant::now() >= deadline {
                return Err(ProviderError::Unconfirmed);
            }
            match found {
                Ok(Some(uid)) if uid == current_uid() => return Ok(()),
                Ok(None) => tokio::time::sleep(SAME_USER_RETRY_DELAY).await,
                _ => return Err(ProviderError::OtherUser),
            }
        }
    };
    tokio::time::timeout_at(deadline, proof)
        .await
        .unwrap_or(Err(ProviderError::Unconfirmed))
}

pub(super) fn current_uid() -> u32 {
    #[cfg(test)]
    if let Ok(uid) = crate::desktop_provider::tests::CALLER_UID.try_with(|uid| *uid) {
        return uid;
    }
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

/// The owner UID of the socket `127.0.0.1:server -> 127.0.0.1:client`.
async fn server_side_uid(server: u16, client: u16) -> Result<Option<u32>, ()> {
    #[cfg(test)]
    if let Ok(lookup) = tests::OWNER_LOOKUP.try_with(Arc::clone) {
        return lookup(server, client).await;
    }
    platform_server_side_uid(server, client).await
}

#[cfg(target_os = "linux")]
async fn platform_server_side_uid(server: u16, client: u16) -> Result<Option<u32>, ()> {
    let table = tokio::fs::read_to_string("/proc/net/tcp")
        .await
        .map_err(|_| ())?;
    Ok(proc_net_tcp_uid(&table, server, client))
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn proc_net_tcp_uid(table: &str, server: u16, client: u16) -> Option<u32> {
    let local = format!("0100007F:{server:04X}");
    let remote = format!("0100007F:{client:04X}");
    table.lines().skip(1).find_map(|line| {
        let cols: Vec<&str> = line.split_whitespace().collect();
        (cols.get(1) == Some(&local.as_str()) && cols.get(2) == Some(&remote.as_str()))
            .then(|| cols.get(7)?.parse().ok())
            .flatten()
    })
}

/// macOS: `lsof` lists the socket's owning process with its UID.
#[cfg(not(target_os = "linux"))]
async fn platform_server_side_uid(server: u16, client: u16) -> Result<Option<u32>, ()> {
    let mut command = tokio::process::Command::new("/usr/sbin/lsof");
    command.args(["-nP", "-F", "un", &format!("-iTCP@127.0.0.1:{server}")]);
    lsof_server_side_uid(command, server, client).await
}

#[cfg(any(not(target_os = "linux"), test))]
async fn lsof_server_side_uid(
    mut command: tokio::process::Command,
    server: u16,
    client: u16,
) -> Result<Option<u32>, ()> {
    let mut child = command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| ())?;
    let mut output = Vec::new();
    child
        .stdout
        .take()
        .ok_or(())?
        .take(MAX_LSOF_BYTES + 1)
        .read_to_end(&mut output)
        .await
        .map_err(|_| ())?;
    if output.len() as u64 > MAX_LSOF_BYTES || !child.wait().await.map_err(|_| ())?.success() {
        return Err(());
    }
    Ok(lsof_uid(&String::from_utf8_lossy(&output), server, client))
}

#[cfg(any(not(target_os = "linux"), test))]
pub(super) fn lsof_uid(output: &str, server: u16, client: u16) -> Option<u32> {
    let want = format!("n127.0.0.1:{server}->127.0.0.1:{client}");
    let mut uid = None;
    for line in output.lines() {
        if let Some(value) = line.strip_prefix('u') {
            uid = value.parse().ok();
        } else if line.strip_prefix('p').is_some() {
            uid = None;
        } else if line == want {
            return uid;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desktop_provider::{call, Endpoint};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::AsyncWriteExt;

    type OwnerFuture =
        std::pin::Pin<Box<dyn std::future::Future<Output = Result<Option<u32>, ()>> + Send>>;
    type OwnerLookup = Arc<dyn Fn(u16, u16) -> OwnerFuture + Send + Sync>;
    tokio::task_local! {
        pub(super) static OWNER_LOOKUP: OwnerLookup;
    }

    #[tokio::test(start_paused = true)]
    async fn missing_proof_shares_one_deadline_and_errors_are_final() {
        let started = tokio::time::Instant::now();
        let mut attempts = 0;
        let result = confirm_same_user(|| {
            attempts += 1;
            std::future::ready(Ok(None))
        })
        .await;
        assert!(matches!(result, Err(ProviderError::Unconfirmed)));
        assert_eq!(started.elapsed(), SAME_USER_TIMEOUT);
        assert!(attempts > 1 && attempts <= 101);

        for answer in [Ok(Some(current_uid().wrapping_add(1))), Err(())] {
            let mut attempts = 0;
            let result = confirm_same_user(|| {
                attempts += 1;
                std::future::ready(answer)
            })
            .await;
            assert!(matches!(result, Err(ProviderError::OtherUser)));
            assert_eq!(attempts, 1);
        }

        let result = confirm_same_user(|| async {
            tokio::time::sleep(SAME_USER_TIMEOUT).await;
            Ok(Some(current_uid()))
        })
        .await;
        assert!(
            matches!(result, Err(ProviderError::Unconfirmed)),
            "proof at the deadline is too late"
        );

        assert!(confirm_same_user(|| async {
            tokio::time::sleep(Duration::from_secs(4)).await;
            Ok(Some(current_uid()))
        })
        .await
        .is_ok());
    }

    #[tokio::test]
    async fn delayed_socket_proof_gates_the_production_request_on_the_same_connection() {
        for answer in [
            Ok(Some(current_uid())),
            Ok(Some(current_uid().wrapping_add(1))),
            Err(()),
            Ok(None),
        ] {
            let listener = Arc::new(tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap());
            let port = listener.local_addr().unwrap().port();
            let accepted = Arc::new(tokio::sync::Mutex::new(None::<TcpStream>));
            let proven = Arc::new(tokio::sync::Notify::new());
            let attempts = Arc::new(AtomicUsize::new(0));
            let lookup = {
                let (accepted, proven, attempts) =
                    (accepted.clone(), proven.clone(), attempts.clone());
                Arc::new(move |server, client| {
                    let (listener, accepted, proven, attempts) = (
                        listener.clone(),
                        accepted.clone(),
                        proven.clone(),
                        attempts.clone(),
                    );
                    Box::pin(async move {
                        let mut slot = accepted.lock().await;
                        if slot.is_none() {
                            *slot = Some(listener.accept().await.unwrap().0);
                        }
                        let stream = slot.as_ref().unwrap();
                        assert_eq!(
                            (server, client),
                            (
                                stream.local_addr().unwrap().port(),
                                stream.peer_addr().unwrap().port()
                            )
                        );
                        let mut byte = [0];
                        assert_eq!(
                            stream.try_read(&mut byte).unwrap_err().kind(),
                            std::io::ErrorKind::WouldBlock,
                            "no header, token or provider payload may precede proof"
                        );
                        if attempts.fetch_add(1, Ordering::SeqCst) < 2 {
                            Ok(None)
                        } else {
                            if answer == Ok(Some(current_uid())) {
                                proven.notify_one();
                            }
                            answer
                        }
                    }) as OwnerFuture
                }) as OwnerLookup
            };
            let responder = if answer == Ok(Some(current_uid())) {
                let (accepted, proven) = (accepted.clone(), proven.clone());
                Some(tokio::spawn(async move {
                    proven.notified().await;
                    let mut stream = accepted.lock().await.take().unwrap();
                    let mut request = Vec::new();
                    let body_at = loop {
                        let mut byte = [0];
                        stream.read_exact(&mut byte).await.unwrap();
                        request.push(byte[0]);
                        if request.ends_with(b"\r\n\r\n") {
                            break request.len();
                        }
                        assert!(request.len() < 4096);
                    };
                    let header = String::from_utf8(request.clone()).unwrap();
                    assert!(header.contains("Authorization: Bearer test-key-do-not-use\r\n")); // sadscan:disable np.http.2
                    let length: usize = header
                        .lines()
                        .find_map(|line| line.strip_prefix("Content-Length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    assert!(length < 4096);
                    request.resize(body_at + length, 0);
                    stream.read_exact(&mut request[body_at..]).await.unwrap();
                    let body: serde_json::Value =
                        serde_json::from_slice(&request[body_at..]).unwrap();
                    assert_eq!(body["params"], serde_json::json!(["synthetic-payload"]));
                    let response =
                        serde_json::json!({"id": body["id"], "result": "ok"}).to_string();
                    stream
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{response}",
                                response.len()
                            )
                            .as_bytes(),
                        )
                        .await
                        .unwrap();
                }))
            } else {
                None
            };
            let endpoint = Endpoint::parse(&format!("http://127.0.0.1:{port}/rpc")).unwrap();
            let result = OWNER_LOOKUP
                .scope(
                    lookup,
                    call(
                        &endpoint,
                        Some("test-key-do-not-use"),
                        true,
                        "nip44_encrypt",
                        &["synthetic-payload".into()],
                    ),
                )
                .await;
            if answer == Ok(None) {
                assert!(attempts.load(Ordering::SeqCst) > 3);
            } else {
                assert_eq!(attempts.load(Ordering::SeqCst), 3);
            }
            if let Some(responder) = responder {
                assert_eq!(result.unwrap(), "ok");
                responder.await.unwrap();
            } else {
                if answer == Ok(None) {
                    assert!(matches!(result, Err(ProviderError::Unconfirmed)));
                } else {
                    assert!(matches!(result, Err(ProviderError::OtherUser)));
                }
                let mut bytes = Vec::new();
                accepted
                    .lock()
                    .await
                    .as_mut()
                    .unwrap()
                    .read_to_end(&mut bytes)
                    .await
                    .unwrap();
                assert!(bytes.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn lsof_failure_and_output_overflow_are_not_missing_proof() {
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args(["-c", "exit 1"]);
        assert_eq!(lsof_server_side_uid(command, 1, 2).await, Err(()));
        let mut command = tokio::process::Command::new("/bin/sh");
        command.args(["-c", "exec yes x"]);
        assert_eq!(lsof_server_side_uid(command, 1, 2).await, Err(()));
    }
    #[tokio::test]
    async fn proof_timeout_and_cancellation_close_the_socket_and_kill_lsof() {
        for cancel in [false, true] {
            let scratch = tempfile::tempdir().unwrap();
            let pid_file = scratch.path().join("lookup.pid");
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = Endpoint::parse(&format!(
                "http://127.0.0.1:{}/rpc",
                listener.local_addr().unwrap().port()
            ))
            .unwrap();
            let lookup = Arc::new(move |server, client| {
                let pid_file = pid_file.clone();
                Box::pin(async move {
                    let mut command = tokio::process::Command::new("/bin/sh");
                    command.args(["-c", "echo $$ > \"$1\"; exec sleep 30", "lookup"]);
                    command.arg(pid_file);
                    lsof_server_side_uid(command, server, client).await
                }) as OwnerFuture
            }) as OwnerLookup;
            let request = tokio::spawn(OWNER_LOOKUP.scope(lookup, async move {
                call(
                    &endpoint,
                    Some("test-key-do-not-use"),
                    true,
                    "nip44_encrypt",
                    &["synthetic-payload".into()],
                )
                .await
            }));
            let (mut stream, _) = listener.accept().await.unwrap();
            let pid_file = scratch.path().join("lookup.pid");
            let pid = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if let Ok(pid) = tokio::fs::read_to_string(&pid_file).await {
                        if let Ok(pid) = pid.trim().parse::<u32>() {
                            break pid;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            if cancel {
                request.abort();
                assert!(request.await.unwrap_err().is_cancelled());
            } else {
                let result = tokio::time::timeout(Duration::from_secs(7), request)
                    .await
                    .unwrap()
                    .unwrap();
                assert!(matches!(result, Err(ProviderError::Unconfirmed)));
            }
            let mut bytes = Vec::new();
            tokio::time::timeout(Duration::from_secs(1), stream.read_to_end(&mut bytes))
                .await
                .unwrap()
                .unwrap();
            assert!(
                bytes.is_empty(),
                "no bytes before failed or cancelled proof"
            );
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    let alive = tokio::process::Command::new("/bin/kill")
                        .args(["-0", &pid.to_string()])
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .status()
                        .await
                        .unwrap()
                        .success();
                    if !alive {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("owned lookup child survived proof cancellation");
        }
    }
}
