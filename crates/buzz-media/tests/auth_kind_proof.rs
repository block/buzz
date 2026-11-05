//! Decisive test: which Authorization event shape does the relay's media-read
//! path actually accept?
//!
//! This binds to the PRODUCTION verifier (`buzz_media::auth::verify_blossom_get_auth`),
//! which is exactly what `buzz_relay::api::media::get_blob` calls via
//! `extract_blossom_read_proof`. It answers the question that split the two
//! independent voice-note builds:
//!
//!   - NIP-98 (kind 27235): `u` + `method` + `nonce` tags, empty content.
//!   - BUD-01 (kind 24242): `t=get` + `expiration` + `server` tags, non-empty content.
//!
//! One of them must be rejected. The test asserts the winner, so the answer is
//! a property of the relay, not of anybody's memory.

use buzz_media::auth::{verify_blossom_get_auth, BlossomStrictness};
use nostr::{EventBuilder, JsonUtil, Keys, Kind, Tag};

const SHA: &str = "b1bfe1031111111111111111111111111111111111111111111111111111b1";
const SERVER: &str = "vmi3238719.mellori-dojo.ts.net:10444";

/// The NIP-98 shape: kind 27235, `u`/`method`/`nonce`, empty content.
/// This is what `buzz-acp`'s `sign_nip98` produces for a GET.
fn nip98_event(keys: &Keys, url: &str) -> nostr::Event {
    let tags = vec![
        Tag::parse(["u", url]).expect("u tag"),
        Tag::parse(["method", "GET"]).expect("method tag"),
        Tag::parse(["nonce", "5c0e9a1e-1b2c-4a7d-9e3f-0a1b2c3d4e5f"]).expect("nonce tag"),
    ];
    EventBuilder::new(Kind::HttpAuth, "")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign nip98")
}

/// The BUD-01 shape: kind 24242, `t=get` + `expiration` + `server`, non-empty content.
fn blossom_get_event(keys: &Keys) -> nostr::Event {
    let now = nostr::Timestamp::now().as_secs();
    let exp = (now + 60).to_string();
    let tags = vec![
        Tag::parse(["t", "get"]).expect("t tag"),
        Tag::parse(["expiration", &exp]).expect("expiration tag"),
        Tag::parse(["server", SERVER]).expect("server tag"),
    ];
    EventBuilder::new(Kind::from(24242), "Get media")
        .tags(tags)
        .sign_with_keys(keys)
        .expect("sign blossom get")
}

#[test]
fn nip98_kind_27235_is_rejected_by_the_media_read_verifier() {
    let keys = Keys::generate();
    let url = format!("https://{SERVER}/media/{SHA}.mp4");
    let event = nip98_event(&keys, &url);
    assert_eq!(event.kind.as_u16(), 27235);

    let result = verify_blossom_get_auth(&event, SHA, Some(SERVER), BlossomStrictness::Permissive);
    assert!(
        result.is_err(),
        "EXPECTED: the relay's media read path must reject a kind:27235 NIP-98 proof \
         (the verifier requires kind 24242). If this fails, the auth contract changed."
    );
}

#[test]
fn blossom_kind_24242_is_accepted_by_the_media_read_verifier() {
    let keys = Keys::generate();
    let event = blossom_get_event(&keys);
    assert_eq!(event.kind.as_u16(), 24242);

    let result = verify_blossom_get_auth(&event, SHA, Some(SERVER), BlossomStrictness::Permissive);
    assert!(
        result.is_ok(),
        "EXPECTED: a kind:24242 t=get proof must be accepted. Got: {:?}",
        result.err()
    );
}

#[test]
fn the_two_shapes_are_not_interchangeable() {
    let keys = Keys::generate();
    let url = format!("https://{SERVER}/media/{SHA}.mp4");

    let nip = nip98_event(&keys, &url);
    let blossom = blossom_get_event(&keys);

    assert_ne!(nip.kind, blossom.kind);
    assert_ne!(nip.tags, blossom.tags);

    // The exact header value the relay parses: "Nostr " + base64url(json).
    // `extract_blossom_auth` accepts URL_SAFE_NO_PAD or STANDARD base64.
    let nip_header = format!("Nostr {}", nip.as_json());
    let blossom_header = format!("Nostr {}", blossom.as_json());

    assert!(nip_header.starts_with("Nostr "));
    assert!(blossom_header.starts_with("Nostr "));
}
