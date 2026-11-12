//! Historical ownership must flow through the real version-addressed GET.
use super::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut request = Vec::new();
    loop {
        let mut chunk = [0; 4096];
        let n = socket.read(&mut chunk).await.unwrap();
        assert!(n > 0 && request.len() + n <= 65536);
        request.extend_from_slice(&chunk[..n]);
        if let Some(end) = request.windows(4).position(|b| b == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&request[..end]).unwrap();
            let size = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            if request.len() >= end + 4 + size {
                break;
            }
        }
    }
    String::from_utf8(request).unwrap()
}

#[tokio::test]
async fn historical_claim_protects_shared_file_and_bad_claim_aborts_before_deletion() {
    for bad_claim in [false, true] {
        let shared = "a".repeat(64);
        let exclusive = "b".repeat(64);
        let community = uuid::Uuid::from_u128(9);
        let event = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let claim_key = format!("_uploads/{community}/{shared}/{event}.json");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (stop, mut stopped) = tokio::sync::oneshot::channel();
        let shared_server = shared.clone();
        let exclusive_server = exclusive.clone();
        let server = tokio::spawn(async move {
            let mut reads = Vec::new();
            let mut deletions = Vec::new();
            let mut prefixes = Vec::new();
            loop {
                let (mut socket, _) = tokio::select! {
                    _ = &mut stopped => break,
                    accepted = listener.accept() => accepted.unwrap(),
                };
                tokio::time::timeout(Duration::from_secs(5), async {
                    let request = read_request(&mut socket).await;
                    let method = request.split_whitespace().next().unwrap();
                    let target = request.split_whitespace().nth(1).unwrap();
                    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
                    let params: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
                    let body = if method == "GET" && params.contains_key("versions") {
                        let prefix = &params["prefix"];
                        prefixes.push(prefix.clone());
                        let entries = if prefix == "_uploads/" {
                            ["old", "new"].map(|version| format!("<Version><Key>{claim_key}</Key><VersionId>{version}</VersionId><Size>1</Size></Version>")).join("")
                        } else if prefix == "_meta/" {
                            String::new()
                        } else {
                            assert!(prefix == &format!("{shared_server}.") || prefix == &format!("{exclusive_server}."));
                            format!("<Version><Key>{prefix}png</Key><VersionId>blob</VersionId><Size>1</Size></Version>")
                        };
                        format!("<ListVersionsResult><IsTruncated>false</IsTruncated>{entries}</ListVersionsResult>")
                    } else if method == "GET" {
                        assert_eq!(url.path(), format!("/test/{claim_key}"));
                        // Without versionId this server returns latest ownership,
                        // exactly as S3 would; the shared-file assertion must fail.
                        let version = params.get("versionId").map(String::as_str).unwrap_or("new");
                        reads.push(version.to_owned());
                        if bad_claim && version == "old" { "{}".into() } else {
                            serde_json::to_string(&json!({
                                "version": UPLOAD_RECORD_VERSION, "event_id": event, "sha256": shared_server,
                                "ext": "png", "mime_type": "image/png", "size": 1, "uploaded_at": 1,
                                "community_id": community.to_string(), "community_host": "media.example",
                                "uploader_id": if version == "old" { BOB } else { ALICE }, "uploader_npub": "unused"
                            })).unwrap()
                        }
                    } else {
                        assert_eq!(method, "POST");
                        let key = request.split("<Key>").nth(1).unwrap().split("</Key>").next().unwrap();
                        let version = request.split("<VersionId>").nth(1).unwrap().split("</VersionId>").next().unwrap();
                        deletions.push(key.to_owned());
                        format!("<DeleteResult><Deleted><Key>{key}</Key><VersionId>{version}</VersionId></Deleted></DeleteResult>")
                    };
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                    socket.write_all(response.as_bytes()).await.unwrap();
                }).await.expect("bounded request");
            }
            (reads, deletions, prefixes)
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
        // Shared is storage-only: discovery and historical sharing both require GETs.
        let db = FakeDb {
            candidates: vec![exclusive.clone()],
            ..Default::default()
        };
        let result = tokio::time::timeout(
            Duration::from_secs(10),
            run_handler(&db, &media, ALICE, true, &mut Vec::new()),
        )
        .await;
        let _ = stop.send(());
        let (reads, deletions, prefixes) = server.await.unwrap();
        let result = result.expect("bounded handler");
        if bad_claim {
            assert!(result.is_err());
            assert!(deletions.is_empty());
            assert!(db.removed.borrow().is_empty());
        } else {
            let result = result.unwrap();
            assert_eq!(result["selected_files"], 2);
            assert_eq!(result["kept_files"], 1);
            assert_eq!(deletions, [format!("{exclusive}.png")]);
            assert_eq!(reads, ["old", "new", "old", "new"]);
            assert_eq!(
                prefixes,
                [
                    "_uploads/".into(),
                    "_uploads/".into(),
                    "_meta/".into(),
                    format!("{shared}."),
                    format!("{exclusive}.")
                ]
            );
        }
    }
}
