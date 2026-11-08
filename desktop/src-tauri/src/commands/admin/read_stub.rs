//! Raw HTTP stub for the read and lift tests: each slot is the exact response
//! bytes, so a test can lie about `Content-Length` to force truncation.

use std::sync::{Arc, Mutex};

/// Serve `responses` in order and record each request's raw bytes.
pub(super) fn serve_raw(responses: Vec<Vec<u8>>) -> (u16, Arc<Mutex<Vec<String>>>) {
    use std::io::{Read, Write};
    super::client::init_admin_client().expect("client builds");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    std::thread::spawn(move || {
        for response in responses {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 8192];
            let n = stream.read(&mut buf).unwrap_or(0);
            log.lock()
                .unwrap()
                .push(String::from_utf8_lossy(&buf[..n]).into_owned());
            let _ = stream.write_all(&response);
            let _ = stream.flush();
        }
    });
    (port, seen)
}

/// A complete response with a truthful `Content-Length`.
pub(super) fn response(status: &str, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .into_bytes()
}

/// The NIP-98 signer pubkey in a recorded request's Authorization header.
pub(super) fn signer_of(request: &str) -> String {
    use base64::Engine as _;
    let token = request
        .lines()
        .find_map(|l| l.strip_prefix("authorization: Nostr "))
        .or_else(|| {
            request
                .lines()
                .find_map(|l| l.strip_prefix("Authorization: Nostr "))
        })
        .expect("NIP-98 authorization header");
    let json = base64::engine::general_purpose::STANDARD
        .decode(token.trim())
        .unwrap();
    let event: serde_json::Value = serde_json::from_slice(&json).unwrap();
    event["pubkey"].as_str().unwrap().to_string()
}
