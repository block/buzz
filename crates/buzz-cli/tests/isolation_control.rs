#[cfg(target_os = "macos")]
#[test]
fn forged_child_endpoint_and_receipt_are_rejected() {
    use std::io::Write;
    use std::net::TcpListener;
    use std::process::Command;
    use std::thread;

    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let address = listener.local_addr().unwrap();
    let fake = serde_json::json!({
        "version": 1,
        "enforcement": "forged",
        "identity_pubkey": "ab".repeat(32),
        "run_id": "forged",
        "run_root": "/",
        "allowed_read_roots": ["/"],
        "allowed_write_roots": ["/"],
        "denied_roots": []
    });
    let body = serde_json::to_vec(&fake).unwrap();
    let server = thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len());
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.write_all(&body);
        }
    });

    let output = Command::new(env!("CARGO_BIN_EXE_buzz"))
        .args(["agents", "isolation-explain", "--pubkey", &"ab".repeat(32)])
        .env("BUZZ_PRIVATE_KEY", "11".repeat(32))
        .env("BUZZ_RELAY_URL", "ws://127.0.0.1:1")
        .env("BUZZ_FILESYSTEM_ISOLATION_ATTESTATION", fake.to_string())
        .env(
            "BUZZ_FILESYSTEM_ISOLATION_CONTROL_URL",
            format!("http://{address}/v1/isolation"),
        )
        .env("BUZZ_FILESYSTEM_ISOLATION_CONTROL_TOKEN", "aa".repeat(32))
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("\"forged\""));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("\"/\""));
    drop(server);
}
