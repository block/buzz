/// Apply the shared relay-body timeout classification while preserving the
/// snapshot paths' established detail for every non-timeout read failure.
pub(crate) async fn read_engram_submit_response(
    response: reqwest::Response,
) -> Result<String, String> {
    response.text().await.map_err(|error| {
        crate::relay::classify_body_read_error(&error, || {
            format!("failed to read relay response: {error}")
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn body_stall_surfaces_classified_timeout() {
        use std::io::{Read as _, Write as _};
        use std::time::Duration;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 64\r\n\r\n",
                );
                let _ = stream.flush();
                std::thread::sleep(Duration::from_secs(1));
            }
        });
        let response = reqwest::Client::new()
            .get(format!("http://{addr}/events"))
            .timeout(Duration::from_millis(200))
            .send()
            .await
            .expect("headers must arrive before the deadline");
        let err = read_engram_submit_response(response).await.unwrap_err();

        assert_eq!(err, "relay unreachable: request timed out");
        let _ = handle.join();
    }

    #[tokio::test]
    async fn incomplete_body_keeps_existing_non_timeout_prefix() {
        use std::io::{Read as _, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 64\r\nConnection: close\r\n\r\nshort",
                );
                let _ = stream.flush();
            }
        });
        let response = reqwest::Client::new()
            .get(format!("http://{addr}/events"))
            .send()
            .await
            .expect("headers must arrive");
        let err = read_engram_submit_response(response).await.unwrap_err();

        assert!(err.starts_with("failed to read relay response: "), "{err}");
        let _ = handle.join();
    }
}
