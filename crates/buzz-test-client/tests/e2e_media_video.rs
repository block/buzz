//! End-to-end video upload tests (Blossom protocol, MP4/H.264).
//!
//! Requires: relay running at localhost:3000, MinIO running at localhost:9000.
//! All tests are `#[ignore]` so they don't run in CI by default.
//!
//! # Running
//!
//! ```text
//! cargo test -p buzz-test-client --test e2e_media_video -- --ignored --nocapture
//! ```

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use reqwest::{Client, StatusCode};
use sha2::{Digest, Sha256};
use std::time::Duration;

fn relay_http_url() -> String {
    std::env::var("RELAY_HTTP_URL").unwrap_or_else(|_| "http://localhost:3000".to_string())
}

/// Extract the host:port authority from the relay URL for use as the `server` tag.
fn relay_server_authority() -> String {
    let url = relay_http_url();
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or("localhost:3000")
        .to_string()
}

fn http_client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build HTTP client")
}

fn sign_blossom_auth(keys: &Keys, sha256: &str) -> nostr::Event {
    let now = Timestamp::now().as_secs();
    let exp_str = (now + 55).to_string();
    let server = relay_server_authority();
    let tags = vec![
        Tag::parse(["t", "upload"]).expect("t tag"),
        Tag::parse(["x", sha256]).expect("x tag"),
        Tag::parse(["expiration", &exp_str]).expect("expiration tag"),
        Tag::parse(["server", &server]).expect("server tag"),
    ];
    EventBuilder::new(Kind::from(24242), "Upload test")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign blossom auth")
}

/// Sign a kind:24242 Blossom *read* auth event. Reads are authenticated
/// unconditionally, so blob and range GETs must present one of these -- without it
/// the 206 and 416 range behaviour below would never be reached.
fn sign_blossom_get_auth(keys: &Keys, sha256: &str) -> nostr::Event {
    let now = Timestamp::now().as_secs();
    let exp_str = (now + 55).to_string();
    let server = relay_server_authority();
    let tags = vec![
        Tag::parse(["t", "get"]).expect("t tag"),
        Tag::parse(["x", sha256]).expect("x tag"),
        Tag::parse(["expiration", &exp_str]).expect("expiration tag"),
        Tag::parse(["server", &server]).expect("server tag"),
    ];
    EventBuilder::new(Kind::from(24242), "Get test")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign blossom get auth")
}

fn blossom_auth_header(event: &nostr::Event) -> String {
    format!(
        "Nostr {}",
        URL_SAFE_NO_PAD.encode(event.as_json().as_bytes())
    )
}

/// Build a minimal but structurally valid fast-start MP4 (H.264, 1s, 320×240)
/// that is unique to this call.
///
/// Layout: ftyp | moov(mvhd + trak(tkhd + mdia(mdhd + hdlr + minf(vmhd + dinf + stbl)))) | mdat | free
/// This is enough for `infer` to detect video/mp4 and for the `mp4` crate to parse.
///
/// The trailing `free` box carries a random nonce so every test uploads its own
/// blob. Tests run in parallel against one relay, and under
/// `BUZZ_MEDIA_READ_MODE=enforce` any event that publishes a hash records a
/// sticky publication marker, even if the event is later rejected. That marker
/// ends uploader-preview reads of the hash, so a shared blob let one test's
/// publish turn another test's GET into a 404.
fn build_test_mp4() -> Vec<u8> {
    fn box_wrap(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let size = (8 + payload.len()) as u32;
        let mut b = Vec::new();
        b.extend_from_slice(&size.to_be_bytes());
        b.extend_from_slice(fourcc);
        b.extend_from_slice(payload);
        b
    }

    // ftyp
    let ftyp = {
        let mut b = Vec::new();
        b.extend_from_slice(&20u32.to_be_bytes());
        b.extend_from_slice(b"ftyp");
        b.extend_from_slice(b"isom");
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(b"isom");
        b
    };

    // mvhd (version 0, timescale=1000, duration=1000ms)
    let mvhd_payload = {
        let mut b = vec![0u8; 4]; // version=0, flags=0
        b.extend_from_slice(&0u32.to_be_bytes()); // creation_time
        b.extend_from_slice(&0u32.to_be_bytes()); // modification_time
        b.extend_from_slice(&1000u32.to_be_bytes()); // timescale
        b.extend_from_slice(&1000u32.to_be_bytes()); // duration
        b.extend_from_slice(&0x00010000u32.to_be_bytes()); // rate
        b.extend_from_slice(&0x0100u16.to_be_bytes()); // volume
        b.extend_from_slice(&[0u8; 10]); // reserved
                                         // identity matrix (9 × u32)
        for &v in &[0x00010000u32, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        b.extend_from_slice(&[0u8; 24]); // pre_defined
        b.extend_from_slice(&2u32.to_be_bytes()); // next_track_id
        b
    };
    let mvhd = box_wrap(b"mvhd", &mvhd_payload);

    // tkhd
    let tkhd_payload = {
        let mut b = vec![0u8, 0, 0, 3]; // version=0, flags=3
        b.extend_from_slice(&0u32.to_be_bytes()); // creation
        b.extend_from_slice(&0u32.to_be_bytes()); // modification
        b.extend_from_slice(&1u32.to_be_bytes()); // track_id
        b.extend_from_slice(&0u32.to_be_bytes()); // reserved
        b.extend_from_slice(&1000u32.to_be_bytes()); // duration
        b.extend_from_slice(&[0u8; 8]); // reserved
        b.extend_from_slice(&0i16.to_be_bytes()); // layer
        b.extend_from_slice(&0i16.to_be_bytes()); // alternate_group
        b.extend_from_slice(&0u16.to_be_bytes()); // volume
        b.extend_from_slice(&0u16.to_be_bytes()); // reserved
        for &v in &[0x00010000u32, 0, 0, 0, 0x00010000, 0, 0, 0, 0x40000000] {
            b.extend_from_slice(&v.to_be_bytes());
        }
        b.extend_from_slice(&(320u32 << 16).to_be_bytes()); // width 16.16
        b.extend_from_slice(&(240u32 << 16).to_be_bytes()); // height 16.16
        b
    };
    let tkhd = box_wrap(b"tkhd", &tkhd_payload);

    // mdhd
    let mdhd_payload = {
        let mut b = vec![0u8; 4];
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&1000u32.to_be_bytes()); // timescale
        b.extend_from_slice(&1000u32.to_be_bytes()); // duration
        b.extend_from_slice(&0u16.to_be_bytes()); // language
        b.extend_from_slice(&0u16.to_be_bytes()); // pre_defined
        b
    };
    let mdhd = box_wrap(b"mdhd", &mdhd_payload);

    // hdlr (video)
    let hdlr_payload = {
        let mut b = vec![0u8; 4];
        b.extend_from_slice(&0u32.to_be_bytes()); // pre_defined
        b.extend_from_slice(b"vide");
        b.extend_from_slice(&[0u8; 12]); // reserved
        b.extend_from_slice(b"VideoHandler\0");
        b
    };
    let hdlr = box_wrap(b"hdlr", &hdlr_payload);

    // vmhd
    let vmhd_payload = {
        let mut b = vec![0u8, 0, 0, 1]; // flags=1
        b.extend_from_slice(&0u16.to_be_bytes());
        b.extend_from_slice(&[0u8; 6]);
        b
    };
    let vmhd = box_wrap(b"vmhd", &vmhd_payload);

    // dinf -> dref -> url
    let url_box = box_wrap(b"url ", &[0, 0, 0, 1]);
    let dref_payload = {
        let mut b = vec![0u8; 4];
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&url_box);
        b
    };
    let dref = box_wrap(b"dref", &dref_payload);
    let dinf = box_wrap(b"dinf", &dref);

    // stsd -> avc1 (H.264)
    let avc1_entry = {
        let mut b = vec![0u8; 6]; // reserved
        b.extend_from_slice(&1u16.to_be_bytes()); // data_ref_idx
        b.extend_from_slice(&[0u8; 2]); // pre_defined
        b.extend_from_slice(&[0u8; 2]); // reserved
        b.extend_from_slice(&[0u8; 12]); // pre_defined
        b.extend_from_slice(&320u16.to_be_bytes()); // width
        b.extend_from_slice(&240u16.to_be_bytes()); // height
        b.extend_from_slice(&0x00480000u32.to_be_bytes()); // horiz_res
        b.extend_from_slice(&0x00480000u32.to_be_bytes()); // vert_res
        b.extend_from_slice(&0u32.to_be_bytes()); // reserved
        b.extend_from_slice(&1u16.to_be_bytes()); // frame_count
        b.extend_from_slice(&[0u8; 32]); // compressorname
        b.extend_from_slice(&0x0018u16.to_be_bytes()); // depth
        b.extend_from_slice(&(-1i16).to_be_bytes()); // pre_defined
                                                     // avcC
        let avcc = vec![
            0x01, 0x42, 0x00, 0x1E, 0xFF, 0xE1, 0x00, 0x00, 0x01, 0x00, 0x00,
        ];
        b.extend_from_slice(&box_wrap(b"avcC", &avcc));
        b
    };
    let avc1 = box_wrap(b"avc1", &avc1_entry);
    let stsd_payload = {
        let mut b = vec![0u8; 4];
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&avc1);
        b
    };
    let stsd = box_wrap(b"stsd", &stsd_payload);

    // Minimal sample tables
    let stts = box_wrap(b"stts", &{
        let mut b = vec![0u8; 4];
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&1000u32.to_be_bytes());
        b
    });
    let stsc = box_wrap(b"stsc", &{
        let mut b = vec![0u8; 4];
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        b
    });
    let stsz = box_wrap(b"stsz", &{
        let mut b = vec![0u8; 4];
        b.extend_from_slice(&0u32.to_be_bytes());
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        b
    });
    let stco = box_wrap(b"stco", &{
        let mut b = vec![0u8; 4];
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(&28u32.to_be_bytes());
        b
    });

    let stbl_payload = [&stsd[..], &stts, &stsc, &stsz, &stco].concat();
    let stbl = box_wrap(b"stbl", &stbl_payload);
    let minf_payload = [&vmhd[..], &dinf, &stbl].concat();
    let minf = box_wrap(b"minf", &minf_payload);
    let mdia_payload = [&mdhd[..], &hdlr, &minf].concat();
    let mdia = box_wrap(b"mdia", &mdia_payload);
    let trak_payload = [&tkhd[..], &mdia].concat();
    let trak = box_wrap(b"trak", &trak_payload);
    let moov_payload = [&mvhd[..], &trak].concat();
    let moov = box_wrap(b"moov", &moov_payload);
    let mdat = box_wrap(b"mdat", &[]);
    let nonce = box_wrap(b"free", uuid::Uuid::new_v4().as_bytes());

    [ftyp, moov, mdat, nonce].concat()
}

/// Upload a valid MP4 video via Blossom, verify the BlobDescriptor includes
/// video-specific fields (duration, dim) and the blob is retrievable.
#[tokio::test]
#[ignore]
async fn test_video_upload_and_get() {
    let client = http_client();
    let keys = Keys::generate();
    let mp4 = build_test_mp4();
    let sha256 = hex::encode(Sha256::digest(&mp4));

    let auth = sign_blossom_auth(&keys, &sha256);
    let url = format!("{}/upload", relay_http_url());

    let resp = client
        .put(&url)
        .header("Authorization", blossom_auth_header(&auth))
        .header("X-SHA-256", &sha256)
        .header("Content-Type", "video/mp4")
        .body(mp4.clone())
        .send()
        .await
        .expect("upload request");

    assert_eq!(resp.status(), StatusCode::OK, "upload should succeed");

    let desc: serde_json::Value = resp.json().await.expect("json body");
    assert_eq!(desc["sha256"].as_str().unwrap(), sha256);
    assert_eq!(desc["type"].as_str().unwrap(), "video/mp4");
    assert!(desc["size"].as_u64().unwrap() > 0);
    // Video descriptor should have duration
    assert!(
        desc.get("duration").is_some(),
        "video descriptor should include duration"
    );

    // GET the blob back
    let get_url = desc["url"].as_str().unwrap();
    let get_resp = client
        .get(get_url)
        .header(
            "Authorization",
            blossom_auth_header(&sign_blossom_get_auth(&keys, &sha256)),
        )
        .send()
        .await
        .expect("GET blob");
    assert_eq!(get_resp.status(), StatusCode::OK);
    let body = get_resp.bytes().await.expect("body bytes");
    assert_eq!(body.len(), mp4.len());
}

/// The relay ignores the Content-Type header and sniffs magic bytes. MP4
/// uploaded with a spoofed image/jpeg header is detected as video/mp4 and
/// accepted through the streaming media validator, including on the legacy
/// compatibility route.
#[tokio::test]
#[ignore]
async fn test_video_content_type_header_ignored() {
    let client = http_client();
    let keys = Keys::generate();
    let mp4 = build_test_mp4();
    let sha256 = hex::encode(Sha256::digest(&mp4));

    let auth = sign_blossom_auth(&keys, &sha256);
    let url = format!("{}/media/upload", relay_http_url());

    // Upload MP4 bytes but claim it's image/jpeg
    let resp = client
        .put(&url)
        .header("Authorization", blossom_auth_header(&auth))
        .header("X-SHA-256", &sha256)
        .header("Content-Type", "image/jpeg")
        .body(mp4)
        .send()
        .await
        .expect("upload request");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "MP4 with spoofed Content-Type should be accepted, got {}",
        resp.status()
    );
    let desc: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(desc["type"].as_str().unwrap(), "video/mp4");
    println!(
        "✅ MP4 with spoofed Content-Type → 200 as video/mp4 (header ignored, magic bytes used)"
    );
}

/// Range request on a video blob should return 206 Partial Content.
#[tokio::test]
#[ignore]
async fn test_video_range_request_206() {
    let client = http_client();
    let keys = Keys::generate();
    let mp4 = build_test_mp4();
    let sha256 = hex::encode(Sha256::digest(&mp4));

    // Upload first
    let auth = sign_blossom_auth(&keys, &sha256);
    let url = format!("{}/media/upload", relay_http_url());
    let resp = client
        .put(&url)
        .header("Authorization", blossom_auth_header(&auth))
        .header("X-SHA-256", &sha256)
        .header("Content-Type", "video/mp4")
        .body(mp4.clone())
        .send()
        .await
        .expect("upload");
    assert_eq!(resp.status(), StatusCode::OK);
    let desc: serde_json::Value = resp.json().await.unwrap();
    let blob_url = desc["url"].as_str().unwrap();

    // Range request: first 100 bytes
    let range_resp = client
        .get(blob_url)
        .header(
            "Authorization",
            blossom_auth_header(&sign_blossom_get_auth(&keys, &sha256)),
        )
        .header("Range", "bytes=0-99")
        .send()
        .await
        .expect("range GET");

    assert_eq!(range_resp.status(), StatusCode::PARTIAL_CONTENT);
    assert!(range_resp.headers().get("content-range").is_some());
    assert!(range_resp
        .headers()
        .get("accept-ranges")
        .is_some_and(|v| v == "bytes"));
    let body = range_resp.bytes().await.unwrap();
    assert_eq!(body.len(), 100);
    assert_eq!(&body[..], &mp4[..100]);
}

/// Unsatisfiable range request should return 416.
#[tokio::test]
#[ignore]
async fn test_video_range_request_416() {
    let client = http_client();
    let keys = Keys::generate();
    let mp4 = build_test_mp4();
    let sha256 = hex::encode(Sha256::digest(&mp4));

    // Upload first
    let auth = sign_blossom_auth(&keys, &sha256);
    let url = format!("{}/media/upload", relay_http_url());
    let resp = client
        .put(&url)
        .header("Authorization", blossom_auth_header(&auth))
        .header("X-SHA-256", &sha256)
        .header("Content-Type", "video/mp4")
        .body(mp4.clone())
        .send()
        .await
        .expect("upload");
    assert_eq!(resp.status(), StatusCode::OK);
    let desc: serde_json::Value = resp.json().await.unwrap();
    let blob_url = desc["url"].as_str().unwrap();

    // Request a range beyond the file size
    let range_resp = client
        .get(blob_url)
        .header(
            "Authorization",
            blossom_auth_header(&sign_blossom_get_auth(&keys, &sha256)),
        )
        .header(
            "Range",
            format!("bytes={}-{}", mp4.len() + 1000, mp4.len() + 2000),
        )
        .send()
        .await
        .expect("range GET");

    assert_eq!(
        range_resp.status(),
        StatusCode::RANGE_NOT_SATISFIABLE,
        "out-of-range request should return 416"
    );
}

/// Upload without auth should return 401.
#[tokio::test]
#[ignore]
async fn test_video_upload_no_auth_returns_401() {
    let client = http_client();
    let mp4 = build_test_mp4();
    let url = format!("{}/media/upload", relay_http_url());

    let resp = client
        .put(&url)
        .header("Content-Type", "video/mp4")
        .body(mp4)
        .send()
        .await
        .expect("upload request");

    assert_eq!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "upload without auth should return 401"
    );
}

fn relay_ws_url() -> String {
    relay_http_url()
        .replace("http://", "ws://")
        .replace("https://", "wss://")
}

/// An 8x8 PNG of random pixels, so every call has its own blob hash.
/// Publishing a hash leaves a sticky publication marker. Until the hash has a
/// readable reference (never, if the event is rejected), the relay refuses
/// uploader previews of it. A blob of its own keeps a publishing test from
/// reaching the fixed blobs, like `tiny_jpeg`, that other media tests read.
fn unique_png() -> Vec<u8> {
    let mut png = Vec::new();
    image::RgbImage::from_fn(8, 8, |_, _| image::Rgb(rand::random()))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .expect("encode PNG");
    png
}

/// Upload a video + poster, then send a message with imeta `image` field
/// referencing the poster. The relay must accept the event.
#[tokio::test]
#[ignore]
async fn test_video_poster_imeta_accepted_via_ws() {
    use buzz_test_client::BuzzTestClient;

    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    // 1. Create a channel
    let channel_uuid = uuid::Uuid::new_v4();
    let channel_id = channel_uuid.to_string();
    let create_event = EventBuilder::new(Kind::from(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["name", &format!("video-poster-test-{channel_id}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&create_event).unwrap())
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "channel creation failed");

    // 2. Upload video
    let mp4 = build_test_mp4();
    let video_sha = hex::encode(Sha256::digest(&mp4));
    let video_auth = sign_blossom_auth(&keys, &video_sha);
    let video_resp = client
        .put(format!("{}/media/upload", relay_http_url()))
        .header("Authorization", blossom_auth_header(&video_auth))
        .header("X-SHA-256", &video_sha)
        .header("Content-Type", "video/mp4")
        .body(mp4.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(video_resp.status(), StatusCode::OK, "video upload failed");
    let video_desc: serde_json::Value = video_resp.json().await.unwrap();
    let video_size = video_desc["size"].as_u64().unwrap();

    // 3. Upload poster (unique PNG)
    let poster = unique_png();
    let poster_sha = hex::encode(Sha256::digest(&poster));
    let poster_auth = sign_blossom_auth(&keys, &poster_sha);
    let poster_resp = client
        .put(format!("{}/media/upload", relay_http_url()))
        .header("Authorization", blossom_auth_header(&poster_auth))
        .header("X-SHA-256", &poster_sha)
        .body(poster.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(poster_resp.status(), StatusCode::OK, "poster upload failed");

    // 4. Send message with imeta referencing both video and poster
    let mut ws = BuzzTestClient::connect(&relay_ws_url(), &keys)
        .await
        .unwrap();

    let base = relay_http_url();
    let event = EventBuilder::new(
        Kind::from(9),
        format!("![video]({base}/media/{video_sha}.mp4)"),
    )
    .tags(vec![
        Tag::parse(["h", &channel_id]).unwrap(),
        Tag::parse([
            "imeta",
            &format!("url {base}/media/{video_sha}.mp4"),
            "m video/mp4",
            &format!("x {video_sha}"),
            &format!("size {video_size}"),
            &format!("image {base}/media/{poster_sha}.png"),
        ])
        .unwrap(),
    ])
    .sign_with_keys(&keys)
    .unwrap();

    let ok = ws.send_event(event).await.unwrap();
    assert!(
        ok.accepted,
        "video+poster imeta must be accepted: {:?}",
        ok.message
    );

    ws.disconnect().await.unwrap();
}

/// Send a message with imeta `image` pointing to the video URL (not an image).
/// The relay must reject this — poster must be an image file, not video.
#[tokio::test]
#[ignore]
async fn test_video_poster_imeta_rejects_video_as_poster() {
    use buzz_test_client::BuzzTestClient;

    let client = http_client();
    let keys = Keys::generate();
    let pubkey_hex = keys.public_key().to_hex();

    // 1. Create channel
    let channel_uuid = uuid::Uuid::new_v4();
    let channel_id = channel_uuid.to_string();
    let create_event = EventBuilder::new(Kind::from(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["name", &format!("poster-reject-test-{channel_id}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", &pubkey_hex)
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&create_event).unwrap())
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());

    // 2. Upload video
    let mp4 = build_test_mp4();
    let video_sha = hex::encode(Sha256::digest(&mp4));
    let video_auth = sign_blossom_auth(&keys, &video_sha);
    let video_resp = client
        .put(format!("{}/media/upload", relay_http_url()))
        .header("Authorization", blossom_auth_header(&video_auth))
        .header("X-SHA-256", &video_sha)
        .header("Content-Type", "video/mp4")
        .body(mp4.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(video_resp.status(), StatusCode::OK);
    let video_desc: serde_json::Value = video_resp.json().await.unwrap();
    let video_size = video_desc["size"].as_u64().unwrap();

    // 3. Send message with imeta `image` pointing to the VIDEO (not an image)
    let mut ws = BuzzTestClient::connect(&relay_ws_url(), &keys)
        .await
        .unwrap();

    let base = relay_http_url();
    let event = EventBuilder::new(Kind::from(9), "bad poster")
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse([
                "imeta",
                &format!("url {base}/media/{video_sha}.mp4"),
                "m video/mp4",
                &format!("x {video_sha}"),
                &format!("size {video_size}"),
                // BAD: image field points to the video itself (.mp4 extension)
                &format!("image {base}/media/{video_sha}.mp4"),
            ])
            .unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();

    let ok = ws.send_event(event).await.unwrap();
    assert!(
        !ok.accepted,
        "video URL as poster must be rejected, but was accepted"
    );

    ws.disconnect().await.unwrap();
}
