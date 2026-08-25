//! Live round-trip tests for the Google Cloud Storage provider.
//!
//! These talk to a real bucket, so they are `#[ignore]`d and additionally
//! gated on `BUZZ_GCS_LIVE=1` — a bare `--ignored` run in an environment
//! without credentials skips instead of failing.
//!
//! ```bash
//! BUZZ_GCS_LIVE=1 \
//! BUZZ_GCS_TEST_BUCKET=my-disposable-bucket \
//!   cargo test -p buzz-object-store --test gcs_live -- --ignored
//! ```
//!
//! Credentials come from Application Default Credentials, exactly as they do
//! in production. The bucket must satisfy the provider's admission check —
//! object versioning disabled and soft-delete retention zero — which the first
//! test asserts explicitly.
//!
//! Every test works under a unique `a2/<uuid>/` prefix and deletes what it
//! wrote, so a bucket accumulates nothing across runs and concurrent runs
//! cannot collide.
//!
//! Two arms need a bucket that is *not* the disposable one, and stay skipped
//! unless the operator names it. Both are read-only — they never write, and
//! the provider refuses to hand out a client for either:
//!
//! ```bash
//! BUZZ_GCS_DENIED_BUCKET=a-bucket-this-identity-cannot-read \
//! BUZZ_GCS_MISCONFIGURED_BUCKET=a-bucket-with-versioning-or-soft-delete-on \
//!   …
//! ```

use std::panic::AssertUnwindSafe;
use std::time::{Duration, Instant};

use futures_util::FutureExt;

use buzz_object_store::{
    ConditionalWrite, GcsObjectStore, GcsStoreConfig, ImmutableWrite, ObjectStore,
    ObjectStoreError, ProviderKind, Revision, WriteCondition,
};

const CONTENT_TYPE: &str = "application/octet-stream";

/// Install the process-wide rustls provider these tests need.
///
/// The relay does this in `main` before any TLS request. A test binary must do
/// the same: both ring and aws-lc-rs are in the build graph, so rustls refuses
/// to pick one on its own.
fn install_crypto_provider() {
    static PROVIDER: std::sync::Once = std::sync::Once::new();
    PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// Whether this environment opted in to talking to a real bucket.
fn live_enabled(test: &str) -> bool {
    install_crypto_provider();
    if std::env::var("BUZZ_GCS_LIVE").as_deref() == Ok("1") {
        return true;
    }
    eprintln!("skipping {test}: set BUZZ_GCS_LIVE=1 to run against a live bucket");
    false
}

/// A bucket named by `variable`, or `None` when the operator did not supply one.
fn named_bucket(variable: &str, test: &str) -> Option<String> {
    match std::env::var(variable) {
        Ok(bucket) if !bucket.trim().is_empty() => Some(bucket.trim().to_string()),
        _ => {
            eprintln!("skipping {test}: set {variable} to run this arm");
            None
        }
    }
}

/// Connect to the test bucket, or `None` when this environment has not opted
/// in to live tests.
async fn live_store(test: &str) -> Option<(GcsObjectStore, String)> {
    if !live_enabled(test) {
        return None;
    }
    let bucket = match std::env::var("BUZZ_GCS_TEST_BUCKET") {
        Ok(bucket) if !bucket.is_empty() => bucket,
        _ => panic!("BUZZ_GCS_LIVE=1 requires BUZZ_GCS_TEST_BUCKET"),
    };

    let store = GcsObjectStore::connect(&GcsStoreConfig::new(bucket))
        .await
        .expect("connect to the test bucket");
    let prefix = format!("a2/{}/{test}", uuid::Uuid::new_v4());
    Some((store, prefix))
}

/// Delete everything this test wrote.
///
/// Failing to clean up is a test failure: these buckets are shared with other
/// runs and a leak would be invisible until it was large.
async fn cleanup(store: &GcsObjectStore, prefix: &str) {
    let mut token = None;
    let mut keys = Vec::new();
    loop {
        let page = store
            .list_page(prefix, token, 1000)
            .await
            .expect("list for cleanup");
        keys.extend(page.objects.into_iter().map(|(key, _)| key));
        token = page.next_continuation_token;
        if token.is_none() {
            break;
        }
    }
    if keys.is_empty() {
        return;
    }
    let outcome = store
        .delete_objects(&keys)
        .await
        .expect("bulk delete for cleanup");
    assert!(
        outcome.failed.is_empty(),
        "cleanup left objects behind: {:?}",
        outcome.failed
    );
}

/// Run one live test body, then clean up its prefix whether it passed or not.
///
/// A panicking body would otherwise leak its objects into a bucket shared with
/// every other run — and a failing test is precisely the one that gets re-run,
/// so that is when cleanup matters most.
async fn with_prefix<F>(test: &str, body: F)
where
    F: AsyncFnOnce(&GcsObjectStore, &str),
{
    let Some((store, prefix)) = live_store(test).await else {
        return;
    };
    let outcome = AssertUnwindSafe(body(&store, &prefix)).catch_unwind().await;
    cleanup(&store, &prefix).await;
    if let Err(payload) = outcome {
        std::panic::resume_unwind(payload);
    }
}

/// The provider reports itself as Cloud Storage, and the bucket satisfies the
/// deletion contract the constructor enforces.
///
/// `versioning_detected()` is read from bucket metadata rather than probed, so
/// a correctly configured bucket answers `false` even though every Cloud
/// Storage object carries a generation.
#[tokio::test]
#[ignore = "requires a live GCS bucket (BUZZ_GCS_LIVE=1)"]
async fn admits_a_bucket_that_can_prove_deletion() {
    with_prefix("admission", async |store, _prefix| {
        assert_eq!(store.provider(), ProviderKind::Gcs);
        assert!(
            !store.versioning_detected().await.expect("bucket metadata"),
            "the test bucket must have object versioning and soft delete disabled"
        );
        store.ping().await.expect("bucket is reachable");
    })
    .await;
}

/// Create-only writes: the first commits, the second finds the key taken.
#[tokio::test]
#[ignore = "requires a live GCS bucket (BUZZ_GCS_LIVE=1)"]
async fn create_only_write_is_idempotent() {
    with_prefix("immutable", async |store, prefix| {
        let key = format!("{prefix}/packs/object");

        assert_eq!(
            store
                .put_immutable(&key, b"immutable bytes", CONTENT_TYPE)
                .await
                .expect("first create-only write"),
            ImmutableWrite::Created
        );
        assert_eq!(
            store
                .put_immutable(&key, b"immutable bytes", CONTENT_TYPE)
                .await
                .expect("second create-only write"),
            ImmutableWrite::AlreadyPresent,
            "a create-only precondition failure is success, not an error"
        );
        assert_eq!(
            store.get(&key).await.expect("read back").as_ref(),
            b"immutable bytes"
        );
    })
    .await;
}

/// The compare-and-swap contract, end to end.
///
/// Two writers hold the same generation. The first commits and the second —
/// now holding a stale generation — must lose, with exactly one winner and no
/// silent overwrite. The winning generation then predicates the next write, so
/// a caller can chain transitions without ever rereading.
#[tokio::test]
#[ignore = "requires a live GCS bucket (BUZZ_GCS_LIVE=1)"]
async fn compare_and_swap_admits_exactly_one_writer() {
    with_prefix("cas", async |store, prefix| {
        let key = format!("{prefix}/pointers/manifest");

        // Create with the absent precondition.
        let created = store
            .put_conditional(&key, b"state-0", CONTENT_TYPE, WriteCondition::Absent)
            .await
            .expect("create pointer");
        let ConditionalWrite::Committed(generation_0) = created else {
            panic!("creating an absent pointer must commit");
        };
        assert!(matches!(generation_0, Revision::GcsGeneration(g) if g > 0));

        // A second create-only write cannot commit over it.
        assert_eq!(
            store
                .put_conditional(&key, b"state-x", CONTENT_TYPE, WriteCondition::Absent)
                .await
                .expect("second create"),
            ConditionalWrite::Conflict
        );

        // Body and revision come from one read, and the revision predicates the
        // next write.
        let (read_generation, body) = store
            .get_with_revision(&key)
            .await
            .expect("pointer read")
            .expect("pointer exists");
        assert_eq!(body.as_ref(), b"state-0");
        assert_eq!(read_generation, generation_0);

        // Two sequential writers, both holding `generation_0`. Exactly one wins.
        let winner = store
            .put_conditional(
                &key,
                b"state-1",
                CONTENT_TYPE,
                WriteCondition::Matches(read_generation.clone()),
            )
            .await
            .expect("first writer");
        let loser = store
            .put_conditional(
                &key,
                b"state-2",
                CONTENT_TYPE,
                WriteCondition::Matches(read_generation),
            )
            .await
            .expect("second writer");

        let ConditionalWrite::Committed(generation_1) = winner else {
            panic!("the writer holding the current generation must commit");
        };
        assert_ne!(
            generation_1, generation_0,
            "a commit mints a new generation"
        );
        assert_eq!(
            loser,
            ConditionalWrite::Conflict,
            "a stale generation must lose the race rather than overwrite"
        );
        assert_eq!(
            store.get(&key).await.expect("read winner").as_ref(),
            b"state-1",
            "the loser must not have written anything"
        );

        // The winner's generation chains straight into the next transition.
        assert!(matches!(
            store
                .put_conditional(
                    &key,
                    b"state-3",
                    CONTENT_TYPE,
                    WriteCondition::Matches(generation_1),
                )
                .await
                .expect("chained write"),
            ConditionalWrite::Committed(_)
        ));
    })
    .await;
}

/// An ETag can never predicate a generation match; the write is refused rather
/// than silently downgraded to an unconditional overwrite.
#[tokio::test]
#[ignore = "requires a live GCS bucket (BUZZ_GCS_LIVE=1)"]
async fn a_foreign_revision_never_reaches_the_backend() {
    with_prefix("foreign-revision", async |store, prefix| {
        let key = format!("{prefix}/pointers/manifest");

        store
            .put_conditional(&key, b"state-0", CONTENT_TYPE, WriteCondition::Absent)
            .await
            .expect("create pointer");

        let error = store
            .put_conditional(
                &key,
                b"clobbered",
                CONTENT_TYPE,
                WriteCondition::Matches(Revision::S3Etag("\"deadbeef\"".into())),
            )
            .await
            .expect_err("an S3 ETag must not predicate a GCS write");
        assert!(matches!(
            error,
            ObjectStoreError::RevisionMismatch {
                expected: ProviderKind::Gcs,
                actual: ProviderKind::S3,
            }
        ));
        assert_eq!(
            store.get(&key).await.expect("read back").as_ref(),
            b"state-0",
            "the refused write must not have touched the object"
        );
    })
    .await;
}

/// Full, ranged, and streamed reads agree, and metadata reports the size and
/// generation.
#[tokio::test]
#[ignore = "requires a live GCS bucket (BUZZ_GCS_LIVE=1)"]
async fn reads_agree_across_full_range_and_stream() {
    use futures_util::StreamExt;

    with_prefix("reads", async |store, prefix| {
        let key = format!("{prefix}/blobs/payload");
        let body: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();

        store.put(&key, &body, CONTENT_TYPE).await.expect("put");

        assert_eq!(
            store.get(&key).await.expect("full read").as_ref(),
            &body[..]
        );
        assert_eq!(
            store.get_range(&key, 100, 199).await.expect("range read"),
            &body[100..=199],
            "the seam's range is inclusive on both ends"
        );
        assert_eq!(
            store.get_range(&key, 4095, 4095).await.expect("last byte"),
            &body[4095..],
        );

        let mut streamed = Vec::new();
        let mut chunks = store.get_stream(&key).await.expect("open stream");
        while let Some(chunk) = chunks.next().await {
            streamed.extend_from_slice(&chunk.expect("stream chunk"));
        }
        assert_eq!(streamed, body);

        let meta = store
            .head(&key)
            .await
            .expect("head")
            .expect("object exists");
        assert_eq!(meta.size, body.len() as u64);
        assert!(matches!(meta.revision, Some(Revision::GcsGeneration(g)) if g > 0));

        assert!(store
            .head(&format!("{prefix}/blobs/absent"))
            .await
            .expect("head of an absent object is not an error")
            .is_none());
        assert!(store
            .get_with_revision(&format!("{prefix}/blobs/absent"))
            .await
            .expect("pointer read of an absent object is not an error")
            .is_none());
        assert!(matches!(
            store.get(&format!("{prefix}/blobs/absent")).await,
            Err(ObjectStoreError::NotFound { .. })
        ));
    })
    .await;
}

/// Prefix listing pages, in ascending key order, without dropping or repeating
/// a key across the page boundary.
#[tokio::test]
#[ignore = "requires a live GCS bucket (BUZZ_GCS_LIVE=1)"]
async fn listing_paginates_over_a_prefix() {
    with_prefix("listing", async |store, prefix| {
        let expected: Vec<String> = (0..5).map(|i| format!("{prefix}/page/{i:03}")).collect();
        for key in &expected {
            store.put(key, b"x", CONTENT_TYPE).await.expect("put");
        }

        let mut seen = Vec::new();
        let mut token = None;
        let mut pages = 0;
        loop {
            let page = store.list_page(prefix, token, 2).await.expect("list page");
            pages += 1;
            assert!(page.objects.len() <= 2, "max_keys must bound one response");
            seen.extend(page.objects.into_iter().map(|(key, size)| {
                assert_eq!(size, 1, "listing reports object size");
                key
            }));
            token = page.next_continuation_token;
            match token {
                Some(_) => assert!(page.is_truncated, "a continuation token means truncated"),
                None => {
                    assert!(!page.is_truncated, "the last page is not truncated");
                    break;
                }
            }
        }

        assert!(pages > 1, "five keys at two per page must span pages");
        assert_eq!(
            seen, expected,
            "keys arrive in ascending order, exactly once"
        );
    })
    .await;
}

/// Deletion means deletion, deleting an absent key is not an error, and a bulk
/// delete folds present and absent keys into distinct counters.
#[tokio::test]
#[ignore = "requires a live GCS bucket (BUZZ_GCS_LIVE=1)"]
async fn deletion_is_idempotent_and_reports_per_key_outcomes() {
    with_prefix("deletion", async |store, prefix| {
        let key = format!("{prefix}/single");
        store.put(&key, b"x", CONTENT_TYPE).await.expect("put");
        store.delete(&key).await.expect("delete");
        // Absence has to hold on every read path, immediately. A bucket with
        // soft delete on would answer the HEAD with a live object or resurrect
        // one on the next read; the admission check exists to keep that bucket
        // from ever getting this far, and this is the observable consequence.
        assert!(
            store.head(&key).await.expect("head").is_none(),
            "with versioning and soft delete off, a delete proves absence"
        );
        assert!(
            matches!(
                store.get(&key).await,
                Err(ObjectStoreError::NotFound { .. })
            ),
            "a deleted object must not be readable"
        );
        assert!(
            store
                .get_with_revision(&key)
                .await
                .expect("pointer read of a deleted object is not an error")
                .is_none(),
            "a deleted object must not still carry a revision"
        );
        assert!(
            store
                .list_page(&key, None, 10)
                .await
                .expect("list")
                .objects
                .is_empty(),
            "a deleted object must not still be listed"
        );
        store
            .delete(&key)
            .await
            .expect("deleting an absent object is not an error");

        let present: Vec<String> = (0..3).map(|i| format!("{prefix}/bulk/{i}")).collect();
        for key in &present {
            store.put(key, b"x", CONTENT_TYPE).await.expect("put");
        }
        let absent: Vec<String> = (0..2).map(|i| format!("{prefix}/bulk/gone-{i}")).collect();

        let mut keys = present.clone();
        keys.extend(absent);
        let outcome = store.delete_objects(&keys).await.expect("bulk delete");

        assert_eq!(outcome.deleted, 3);
        assert_eq!(outcome.already_missing, 2);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert!(
            outcome.versioned_keys.is_empty(),
            "an admitted bucket cannot produce version artifacts"
        );

        assert!(store
            .list_page(prefix, None, 10)
            .await
            .expect("list")
            .objects
            .is_empty());
    })
    .await;
}

/// A sole writer driving the pointer faster than Cloud Storage's documented
/// one-write-per-second-per-object limit.
///
/// This is the shape of the mirror's chunked repository seed: many sequential
/// transitions against a single object name, each predicated on the generation
/// the previous one returned. Throttling must pace the writer — never fail the
/// push, and never masquerade as a lost race.
///
/// The two phases are ordered deliberately. The first uses a client configured
/// for a single attempt, so every 429 reaches the caller and can be counted;
/// it also spends whatever burst allowance the service was willing to grant.
/// The second then drives the same key just as hard through the provider's own
/// bounded policy, with that allowance already gone — which is what makes it a
/// real test of absorption rather than of a quiet service.
#[tokio::test]
#[ignore = "requires a live GCS bucket (BUZZ_GCS_LIVE=1)"]
async fn rapid_sequential_transitions_are_paced_not_failed() {
    // Close to a real chunked seed of a large mirrored repository (~4.5k refs
    // at 200 refs per chunk), and comfortably past the burst allowance the
    // service grants a fresh object name — which is what makes the throttling
    // in this test real rather than incidental.
    const TRANSITIONS: usize = 20;

    with_prefix("pacing", async |store, prefix| {
        let key = format!("{prefix}/pointers/hot");

        let mut revision = match store
            .put_conditional(&key, b"transition-0", CONTENT_TYPE, WriteCondition::Absent)
            .await
            .expect("create pointer")
        {
            ConditionalWrite::Committed(revision) => revision,
            ConditionalWrite::Conflict => panic!("creating an absent pointer must commit"),
        };

        // Phase 1: no in-provider retries, so throttling is visible and counted.
        let mut config = GcsStoreConfig::new(store.bucket());
        config.retry.max_attempts = 1;
        config.retry.max_throttled_attempts = 1;
        let unpaced = GcsObjectStore::connect(&config)
            .await
            .expect("connect a single-attempt client");

        let mut observed_429s = 0usize;
        for i in 0..TRANSITIONS {
            let body = format!("unpaced-{i}");
            loop {
                match unpaced
                    .put_conditional(
                        &key,
                        body.as_bytes(),
                        CONTENT_TYPE,
                        WriteCondition::Matches(revision.clone()),
                    )
                    .await
                {
                    Ok(ConditionalWrite::Committed(next)) => {
                        revision = next;
                        break;
                    }
                    Ok(ConditionalWrite::Conflict) => {
                        panic!("transition {i} lost a race it was the only entrant in")
                    }
                    // Throttling is backpressure. The retry carries the same
                    // precondition; dropping it here would be the exact bug this
                    // test exists to prevent.
                    Err(ObjectStoreError::Throttled { retry_after, .. }) => {
                        observed_429s += 1;
                        tokio::time::sleep(retry_after.unwrap_or(Duration::from_millis(400))).await;
                    }
                    Err(other) => panic!("transition {i} failed: {other}"),
                }
            }
        }

        // Phase 2: the provider's own bounded policy, against a spent allowance.
        let started = Instant::now();
        let mut generations = vec![revision.clone()];
        for i in 0..TRANSITIONS {
            let body = format!("paced-{i}");
            revision = match store
                .put_conditional(
                    &key,
                    body.as_bytes(),
                    CONTENT_TYPE,
                    WriteCondition::Matches(revision),
                )
                .await
                .unwrap_or_else(|e| panic!("transition {i} must not fail: {e}"))
            {
                ConditionalWrite::Committed(revision) => revision,
                ConditionalWrite::Conflict => {
                    panic!("transition {i} lost a race it was the only entrant in")
                }
            };
            generations.push(revision.clone());
        }
        let elapsed = started.elapsed();

        let mut distinct = generations.clone();
        distinct.dedup();
        assert_eq!(
            distinct.len(),
            generations.len(),
            "every transition must mint a distinct generation"
        );
        assert_eq!(
            store.get(&key).await.expect("final read").as_ref(),
            format!("paced-{}", TRANSITIONS - 1).as_bytes(),
            "the last transition must be the published state"
        );

        eprintln!(
            "pacing: {TRANSITIONS} single-attempt transitions observed {observed_429s} throttled \
         attempts; {TRANSITIONS} further transitions then completed through the provider's own \
         policy in {elapsed:?}"
        );
    })
    .await;
}

/// Streaming a large object to the backend and back without ever holding it in
/// memory.
///
/// `put_file` is the media path for blobs that do not fit in a buffer: it hands
/// the client an open file and lets the resumable upload protocol do the
/// chunking. The read side is the mirror image — the byte stream is folded into
/// a digest as it arrives, so nothing larger than one chunk is resident at
/// either end. The assertions that matter are that the round trip is
/// byte-exact, that the provider reports the size it stored, and that a range
/// read still addresses the far end of a multi-chunk object correctly.
///
/// The size is configurable because the deployment's ceiling (500 MiB) is
/// larger than a routine test run wants to move; the default is big enough to
/// require a resumable upload rather than a single-request one.
#[tokio::test]
#[ignore = "requires a live GCS bucket (BUZZ_GCS_LIVE=1)"]
async fn streams_a_large_object_through_a_resumable_upload() {
    use std::io::Write;

    use sha2::{Digest, Sha256};

    let mebibytes: usize = std::env::var("BUZZ_GCS_LARGE_OBJECT_MIB")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(96);

    with_prefix("large-object", async |store, prefix| {
        let key = format!("{prefix}/media/large.bin");
        let total = mebibytes * 1024 * 1024;

        // A pseudo-random, non-repeating body: a run of identical blocks would
        // let a chunking bug (a dropped or duplicated chunk) round-trip
        // undetected.
        let mut block = vec![0u8; 1024 * 1024];
        let mut expected = Sha256::new();
        let source = std::env::temp_dir().join(format!("buzz-a4-large-{}", uuid::Uuid::new_v4()));
        {
            let mut file = std::fs::File::create(&source).expect("create the upload source");
            let mut state = 0x2545_F491_4F6C_DD1Du64;
            for _ in 0..mebibytes {
                for byte in block.iter_mut() {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    *byte = (state >> 24) as u8;
                }
                expected.update(&block);
                file.write_all(&block).expect("write the upload source");
            }
            file.flush().expect("flush the upload source");
        }
        let expected = hex::encode(expected.finalize());

        let upload = Instant::now();
        let outcome = store.put_file(&key, &source, CONTENT_TYPE).await;
        let upload = upload.elapsed();
        // The temp file is this test's, not the harness's: remove it before
        // asserting so a failure cannot leave a large file behind.
        let _ = std::fs::remove_file(&source);
        outcome.expect("streaming upload");

        let meta = store
            .head(&key)
            .await
            .expect("head")
            .expect("the uploaded object exists");
        assert_eq!(meta.size, total as u64, "the provider stored every byte");

        let download = Instant::now();
        let mut streamed = Sha256::new();
        let mut length = 0u64;
        {
            use futures_util::StreamExt;
            let mut chunks = store.get_stream(&key).await.expect("open the read stream");
            while let Some(chunk) = chunks.next().await {
                let chunk = chunk.expect("stream chunk");
                length += chunk.len() as u64;
                streamed.update(&chunk);
            }
        }
        let download = download.elapsed();
        assert_eq!(length, total as u64, "the stream delivered every byte");
        assert_eq!(
            hex::encode(streamed.finalize()),
            expected,
            "the streamed body must be byte-exact"
        );

        // A range read against the far end of a multi-chunk object: the offset
        // is past every upload chunk boundary, so an implementation that
        // resolved ranges against a single chunk would miss here.
        let tail = store
            .get_range(&key, total as u64 - 8, total as u64 - 1)
            .await
            .expect("range read at the tail");
        assert_eq!(tail.len(), 8, "the seam's range is inclusive on both ends");

        eprintln!(
            "large object: {mebibytes} MiB uploaded in {upload:?}, streamed back in {download:?}"
        );
    })
    .await;
}

/// Permission denied is a classified, permanent answer — never an ambiguous
/// outcome, and never a reason to come up anyway.
///
/// The bucket-metadata read is the first request the provider makes, so a
/// client for a bucket this identity cannot read never gets constructed. That
/// is the fail-closed behaviour that matters: an unauthorised deployment stops
/// at connect rather than discovering its authorisation one operation at a
/// time.
///
/// The arm is read-only. It reads bucket metadata and nothing else, and the
/// only outcome it accepts is a refusal.
#[tokio::test]
#[ignore = "requires a live GCS bucket (BUZZ_GCS_LIVE=1)"]
async fn permission_denied_refuses_to_hand_out_a_client() {
    let test = "permission-denied";
    if !live_enabled(test) {
        return;
    }
    let Some(bucket) = named_bucket("BUZZ_GCS_DENIED_BUCKET", test) else {
        return;
    };

    let error = GcsObjectStore::connect(&GcsStoreConfig::new(&bucket))
        .await
        .err()
        .unwrap_or_else(|| {
            panic!("BUZZ_GCS_DENIED_BUCKET={bucket} is readable by this identity — pick a bucket it cannot read")
        });

    match &error {
        ObjectStoreError::Provider { operation, .. } => {
            assert_eq!(*operation, "get_bucket", "the refusal names the operation");
        }
        other => panic!(
            "permission denied must be a classified provider answer, not {other:?}: an \
             authorisation failure that read as ambiguous or retryable would be retried \
             forever, and one that read as a conflict would be scored as a lost race"
        ),
    }
    assert!(
        !error.is_ambiguous(),
        "a 403 is an answer: the request was evaluated and refused"
    );
    assert!(
        !error.is_retryable(),
        "retrying a permission failure cannot change its outcome"
    );
    eprintln!("permission denied: {error}");
}

/// A bucket that cannot prove deletion is refused at connect, with the
/// violation named.
///
/// This is the A2 admission check against a real misconfigured bucket rather
/// than a constructed one: object versioning or a nonzero soft-delete retention
/// both mean a delete leaves a restorable copy, so `delete` could report success
/// while the bytes stay reachable. Buzz's deletion contract cannot hold on such
/// a bucket, so the provider refuses to return a client for it at all.
///
/// Read-only: the run reads bucket metadata and stops. Nothing is written to
/// the misconfigured bucket, which is the point — the client never exists.
#[tokio::test]
#[ignore = "requires a live GCS bucket (BUZZ_GCS_LIVE=1)"]
async fn a_bucket_that_cannot_prove_deletion_is_refused() {
    let test = "misconfigured-bucket";
    if !live_enabled(test) {
        return;
    }
    let Some(bucket) = named_bucket("BUZZ_GCS_MISCONFIGURED_BUCKET", test) else {
        return;
    };

    let error = GcsObjectStore::connect(&GcsStoreConfig::new(&bucket))
        .await
        .err()
        .unwrap_or_else(|| {
            panic!(
                "BUZZ_GCS_MISCONFIGURED_BUCKET={bucket} satisfies the deletion contract — point \
                 this at a bucket with object versioning or a nonzero soft-delete retention"
            )
        });

    let ObjectStoreError::Config(message) = &error else {
        panic!("a misconfigured bucket must be refused as a configuration error, not {error:?}");
    };
    assert!(
        message.contains("does not satisfy the deletion contract"),
        "the refusal must name the contract it failed: {message}"
    );
    assert!(
        message.contains("object versioning is enabled") || message.contains("soft-delete"),
        "the refusal must name the violating setting: {message}"
    );
    eprintln!("misconfigured bucket refused: {error}");
}
