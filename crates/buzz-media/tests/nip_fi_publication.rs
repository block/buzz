//! Contract tests for immutable protected media publication keys.

use buzz_media::{media_publication_manifest_key, upload_record_plan_key};

#[test]
fn publication_and_moderation_plan_keys_are_exact_digest_addresses() {
    assert_eq!(
        media_publication_manifest_key([0xab; 32]),
        format!("_publications/media/{}.json", "ab".repeat(32))
    );
    assert_eq!(
        upload_record_plan_key([0xcd; 32]),
        format!("_publication-plans/moderation/{}.json", "cd".repeat(32))
    );
}

#[test]
fn distinct_payload_digests_cannot_alias_staged_keys() {
    assert_ne!(
        media_publication_manifest_key([0x01; 32]),
        media_publication_manifest_key([0x02; 32])
    );
    assert_ne!(
        upload_record_plan_key([0x01; 32]),
        upload_record_plan_key([0x02; 32])
    );
}
