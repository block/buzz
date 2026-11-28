//! End-to-end audio-only MP4 (`audio/mp4`, AAC) upload tests.
//!
//! Voice notes upload as one AAC track in a fast-start MP4 and are stored as
//! `audio/mp4` with a `.m4a` URL. They pass the same container, metadata, and
//! track policy as video.
//!
//! Requires: relay running at localhost:3000, MinIO running at localhost:9000.
//! All tests are `#[ignore]` so they don't run in CI by default.
//!
//! # Running
//!
//! ```text
//! cargo test -p buzz-test-client --test e2e_media_audio -- --ignored --nocapture
//! ```

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use reqwest::{Client, StatusCode};
use sha2::{Digest, Sha256};
use std::time::Duration;

fn relay_http_url() -> String {
    std::env::var("RELAY_HTTP_URL").unwrap_or_else(|_| "http://localhost:3000".to_string())
}

fn relay_ws_url() -> String {
    relay_http_url()
        .replace("http://", "ws://")
        .replace("https://", "wss://")
}

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

fn sign_blossom(keys: &Keys, verb: &str, sha256: &str) -> String {
    let exp = (Timestamp::now().as_secs() + 55).to_string();
    let server = relay_server_authority();
    let event = EventBuilder::new(Kind::from(24242), "Audio test")
        .tags(vec![
            Tag::parse(["t", verb]).expect("t tag"),
            Tag::parse(["x", sha256]).expect("x tag"),
            Tag::parse(["expiration", &exp]).expect("expiration tag"),
            Tag::parse(["server", &server]).expect("server tag"),
        ])
        .sign_with_keys(keys)
        .expect("sign blossom auth");
    format!(
        "Nostr {}",
        URL_SAFE_NO_PAD.encode(event.as_json().as_bytes())
    )
}

fn box_wrap(fourcc: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(8 + payload.len());
    b.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
    b.extend_from_slice(fourcc);
    b.extend_from_slice(payload);
    b
}

fn full_box(fourcc: &[u8; 4], flags: u32, fields: &[u32]) -> Vec<u8> {
    let mut b = flags.to_be_bytes().to_vec();
    for field in fields {
        b.extend_from_slice(&field.to_be_bytes());
    }
    box_wrap(fourcc, &b)
}

/// Build a fast-start MP4 with one AAC track (2.5 s, timescale 1000) and no video.
///
/// Layout: ftyp | moov(mvhd + trak(tkhd + mdia(mdhd + hdlr + minf(smhd + dinf + stbl)))) | mdat
fn build_test_m4a() -> Vec<u8> {
    const DURATION: u32 = 2_500;
    let ftyp = box_wrap(b"ftyp", b"M4A \0\0\0\0M4A isommp42");

    let mut mvhd = vec![0u8; 4];
    for v in [0u32, 0, 1000, DURATION, 0x0001_0000] {
        mvhd.extend_from_slice(&v.to_be_bytes());
    }
    mvhd.extend_from_slice(&0x0100u16.to_be_bytes());
    mvhd.extend_from_slice(&[0u8; 10]);
    for v in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        mvhd.extend_from_slice(&v.to_be_bytes());
    }
    mvhd.extend_from_slice(&[0u8; 24]);
    mvhd.extend_from_slice(&2u32.to_be_bytes());
    let mvhd = box_wrap(b"mvhd", &mvhd);

    let mut tkhd = vec![0u8, 0, 0, 3];
    for v in [0u32, 0, 1, 0, DURATION, 0, 0] {
        tkhd.extend_from_slice(&v.to_be_bytes());
    }
    tkhd.extend_from_slice(&[0, 0, 0, 0, 0x01, 0x00, 0, 0]); // layer, group, volume 1.0
    for v in [
        0x0001_0000u32,
        0,
        0,
        0,
        0x0001_0000,
        0,
        0,
        0,
        0x4000_0000,
        0,
        0,
    ] {
        tkhd.extend_from_slice(&v.to_be_bytes());
    }
    let tkhd = box_wrap(b"tkhd", &tkhd);

    let mut mdhd = vec![0u8; 4];
    for v in [0u32, 0, 1000, DURATION] {
        mdhd.extend_from_slice(&v.to_be_bytes());
    }
    mdhd.extend_from_slice(&[0x55, 0xC4, 0, 0]); // language "und"
    let mdhd = box_wrap(b"mdhd", &mdhd);

    let mut hdlr = vec![0u8; 8];
    hdlr.extend_from_slice(b"soun");
    hdlr.extend_from_slice(&[0u8; 12]);
    hdlr.extend_from_slice(b"SoundHandler\0");
    let hdlr = box_wrap(b"hdlr", &hdlr);

    let smhd = box_wrap(b"smhd", &[0u8; 8]);
    let dref = box_wrap(
        b"dref",
        &[
            &[0u8, 0, 0, 0, 0, 0, 0, 1][..],
            &box_wrap(b"url ", &[0, 0, 0, 1]),
        ]
        .concat(),
    );
    let dinf = box_wrap(b"dinf", &dref);

    // mp4a sample entry with an AAC-LC (object type 0x40) esds.
    let esds = box_wrap(
        b"esds",
        &[
            0, 0, 0, 0, // version + flags
            0x03, 0x19, 0x00, 0x01, 0x00, // ES_Descriptor
            0x04, 0x11, 0x40, 0x15, 0x00, 0x00, 0x00, 0x00, 0x01, 0xF4, 0x00, 0x00, 0x01, 0xF4,
            0x00, // DecoderConfigDescriptor
            0x05, 0x02, 0x11, 0x90, // AudioSpecificConfig: AAC-LC, 48 kHz, mono
            0x06, 0x01, 0x02, // SLConfigDescriptor
        ],
    );
    let mut mp4a = vec![0u8; 6];
    mp4a.extend_from_slice(&1u16.to_be_bytes()); // data_reference_index
    mp4a.extend_from_slice(&[0u8; 8]);
    mp4a.extend_from_slice(&1u16.to_be_bytes()); // channelcount
    mp4a.extend_from_slice(&16u16.to_be_bytes()); // samplesize
    mp4a.extend_from_slice(&[0u8; 4]);
    mp4a.extend_from_slice(&(48_000u32 << 16).to_be_bytes()); // samplerate 16.16
    mp4a.extend_from_slice(&esds);
    let stsd = box_wrap(
        b"stsd",
        &[&[0u8, 0, 0, 0, 0, 0, 0, 1][..], &box_wrap(b"mp4a", &mp4a)].concat(),
    );
    let stbl = box_wrap(
        b"stbl",
        &[
            stsd,
            full_box(b"stts", 0, &[1, 1, DURATION]),
            full_box(b"stsc", 0, &[1, 1, 1, 1]),
            full_box(b"stsz", 0, &[0, 1, 0]),
            full_box(b"stco", 0, &[1, 0]),
        ]
        .concat(),
    );
    let minf = box_wrap(b"minf", &[smhd, dinf, stbl].concat());
    let mdia = box_wrap(b"mdia", &[mdhd, hdlr, minf].concat());
    let trak = box_wrap(b"trak", &[tkhd, mdia].concat());
    let moov = box_wrap(b"moov", &[mvhd, trak].concat());
    [ftyp, moov, box_wrap(b"mdat", &[])].concat()
}

async fn upload(client: &Client, keys: &Keys, bytes: &[u8]) -> reqwest::Response {
    let sha256 = hex::encode(Sha256::digest(bytes));
    client
        .put(format!("{}/upload", relay_http_url()))
        .header("Authorization", sign_blossom(keys, "upload", &sha256))
        .header("X-SHA-256", &sha256)
        .header("Content-Type", "audio/mp4")
        .body(bytes.to_vec())
        .send()
        .await
        .expect("upload request")
}

async fn create_channel(client: &Client, keys: &Keys) -> String {
    let channel_id = uuid::Uuid::new_v4().to_string();
    let event = EventBuilder::new(Kind::from(9007), "")
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse(["name", &format!("audio-test-{channel_id}")]).unwrap(),
            Tag::parse(["channel_type", "stream"]).unwrap(),
            Tag::parse(["visibility", "open"]).unwrap(),
        ])
        .sign_with_keys(keys)
        .unwrap();
    let resp = client
        .post(format!("{}/events", relay_http_url()))
        .header("X-Pubkey", keys.public_key().to_hex())
        .header("Content-Type", "application/json")
        .body(serde_json::to_string(&event).unwrap())
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "channel creation failed");
    channel_id
}

/// An audio-only AAC MP4 is stored as `audio/mp4` under a `.m4a` URL, with its
/// duration, and is served inline with its canonical type.
#[tokio::test]
#[ignore]
async fn test_audio_mp4_upload_and_get() {
    let client = http_client();
    let keys = Keys::generate();
    let m4a = build_test_m4a();
    let sha256 = hex::encode(Sha256::digest(&m4a));

    let resp = upload(&client, &keys, &m4a).await;
    assert_eq!(resp.status(), StatusCode::OK, "audio-only MP4 must upload");
    let desc: serde_json::Value = resp.json().await.expect("json body");
    assert_eq!(desc["type"], "audio/mp4");
    assert_eq!(desc["sha256"], sha256.as_str());
    assert_eq!(desc["duration"].as_f64(), Some(2.5));
    assert!(desc.get("dim").is_none(), "audio has no dimensions: {desc}");
    let url = desc["url"].as_str().unwrap();
    assert!(url.ends_with(&format!("{sha256}.m4a")), "{url}");

    let get = client
        .get(url)
        .header("Authorization", sign_blossom(&keys, "get", &sha256))
        .send()
        .await
        .expect("GET blob");
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(get.headers()["content-type"], "audio/mp4");
    assert_eq!(get.headers()["content-disposition"], "inline");
    assert_eq!(get.bytes().await.unwrap().len(), m4a.len());
}

/// Audio-only MP4 carrying a QuickTime ISO-6709 location is rejected like video.
#[tokio::test]
#[ignore]
async fn test_audio_mp4_with_location_rejected() {
    let client = http_client();
    let keys = Keys::generate();
    let mut m4a = build_test_m4a();
    // Append moov/udta/©xyz: moov starts after the 28-byte ftyp.
    let location = box_wrap(
        b"udta",
        &box_wrap(b"\xa9xyz", b"+37.7750-122.4183+015.000/"),
    );
    let moov_start = 28;
    let moov_size = u32::from_be_bytes(m4a[moov_start..moov_start + 4].try_into().unwrap());
    let moov_end = moov_start + moov_size as usize;
    m4a.splice(moov_end..moov_end, location.iter().copied());
    let new_size = moov_size + location.len() as u32;
    m4a[moov_start..moov_start + 4].copy_from_slice(&new_size.to_be_bytes());

    let resp = upload(&client, &keys, &m4a).await;
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// A voice note message referencing the `audio/mp4` blob with NIP-A0
/// `duration` and `waveform` fields is accepted.
#[tokio::test]
#[ignore]
async fn test_audio_mp4_voice_note_message_accepted() {
    let client = http_client();
    let keys = Keys::generate();
    let channel_id = create_channel(&client, &keys).await;
    let m4a = build_test_m4a();
    let resp = upload(&client, &keys, &m4a).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let desc: serde_json::Value = resp.json().await.unwrap();
    let url = desc["url"].as_str().unwrap();
    let sha256 = desc["sha256"].as_str().unwrap();
    let size = desc["size"].as_u64().unwrap();

    let mut ws = BuzzTestClient::connect(&relay_ws_url(), &keys)
        .await
        .unwrap();
    let event = EventBuilder::new(Kind::from(9), format!("[voice-note-1.m4a]({url})"))
        .tags(vec![
            Tag::parse(["h", &channel_id]).unwrap(),
            Tag::parse([
                "imeta",
                &format!("url {url}"),
                "m audio/mp4",
                &format!("size {size}"),
                &format!("x {sha256}"),
                "filename voice-note-1.m4a",
                "duration 2.5",
                "waveform 0 25 100 40",
            ])
            .unwrap(),
        ])
        .sign_with_keys(&keys)
        .unwrap();
    let ok = ws.send_event(event).await.unwrap();
    assert!(ok.accepted, "voice note must be accepted: {:?}", ok.message);
    ws.disconnect().await.unwrap();
}
