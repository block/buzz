//! Exercise the production S3 adapter and handler against a local HTTP server.
use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn run_listing_case(listing: String, fail_first_pass: bool, expect_success: bool) {
    let hash = "a".repeat(64);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let deletion_response = format!("<DeleteResult><Deleted><Key>{hash}.png</Key><VersionId>v1</VersionId></Deleted></DeleteResult>");
    let (stop, mut stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        loop {
            let accepted = tokio::select! {
                _ = &mut stopped => break,
                accepted = listener.accept() => accepted,
            };
            let (mut socket, _) = accepted.unwrap();
            let mut request = Vec::new();
            tokio::time::timeout(Duration::from_secs(5), async {
                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let mut chunk = [0u8; 1024];
                    let n = socket.read(&mut chunk).await.unwrap();
                    assert_ne!(n, 0, "request headers ended prematurely");
                    request.extend_from_slice(&chunk[..n]);
                    assert!(request.len() < 65536);
                }
                let request = String::from_utf8(request).unwrap();
                let method = request.split_whitespace().next().unwrap().to_owned();
                let body = if method == "GET" {
                    if requests.is_empty() && !fail_first_pass {
                        "<ListVersionsResult><IsTruncated>false</IsTruncated></ListVersionsResult>"
                    } else {
                        &listing
                    }
                } else {
                    // Let a forbidden mutation finish so assertions expose it, rather
                    // than turning a successful deletion into a transport error.
                    &deletion_response
                };
                requests.push(method);
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            }).await.expect("bounded HTTP request");
        }
        requests
    });
    let media = MediaStorage::new(&buzz_media::MediaConfig {
        s3_endpoint: endpoint,
        s3_bucket: "test".into(),
        s3_access_key: "test".into(),
        s3_secret_key: "test".into(),
        s3_region: "us-east-1".into(),
        s3_addressing_style: buzz_media::S3AddressingStyle::Path,
        max_image_bytes: 1,
        max_gif_bytes: 1,
        max_video_bytes: 1,
        max_file_bytes: 1,
        public_base_url: "https://media.example".into(),
        upload_records_enabled: false,
        upload_ip_header: None,
        upload_port_header: None,
    })
    .unwrap();
    let db = FakeDb {
        candidates: vec![hash],
        ..Default::default()
    };
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        run_handler(&db, &media, ALICE, true, &mut Vec::new()),
    )
    .await;
    let _ = stop.send(());
    let requests = server.await.unwrap();
    let result = result.expect("bounded handler");
    if expect_success {
        let result = result.expect("valid listing permits cleanup");
        assert_eq!(result["deleted_versions"], 1);
        assert_eq!(db.removed.borrow().len(), 1);
        assert_eq!(requests, ["GET", "GET", "POST"]);
    } else {
        assert!(result.is_err(), "malformed listing must fail");
        assert!(db.removed.borrow().is_empty());
        assert_eq!(requests.len(), if fail_first_pass { 1 } else { 2 });
        assert!(
            requests.iter().all(|method| method == "GET"),
            "mutating requests: {requests:?}"
        );
    }
}

#[tokio::test]
async fn malformed_s3_listings_fail_before_any_mutation_on_either_pass() {
    let entry = format!(
        "<Version><Key>{}.png</Key><VersionId>v1</VersionId><Size>1</Size></Version>",
        "a".repeat(64)
    );
    let malformed = [
        format!("<ListVersionsResult><IsTruncated>false</IsTruncated>{entry}"),
        "<Error><Code>InternalError</Code></Error>".into(),
        format!("<ListVersionsResult>{entry}</ListVersionsResult>"),
        format!(
            "<ListVersionsResult><IsTruncated>invalid</IsTruncated>{entry}</ListVersionsResult>"
        ),
    ];
    for listing in malformed {
        for first_pass in [true, false] {
            run_listing_case(listing.clone(), first_pass, false).await;
        }
    }
}

#[tokio::test]
async fn complete_s3_listing_permits_deletion_through_real_adapter() {
    let listing = format!("<ListVersionsResult><IsTruncated>false</IsTruncated><Version><Key>{}.png</Key><VersionId>v1</VersionId><Size>1</Size></Version></ListVersionsResult>", "a".repeat(64));
    run_listing_case(listing, false, true).await;
}
