#[tokio::test]
#[ignore = "requires explicit isolated BUZZ_TEST_DATABASE_URL; run with --ignored --exact"]
async fn rejects_dm_publish_after_membership_changes() {
    let report = buzz_relay::handlers::ingest::run_membership_revision_race_probe_for_test()
        .await
        .expect("membership revision race probe");

    assert!(
        report.used_existing_membership_advisory_lock,
        "prepared publish must share the existing buzz_channel_membership advisory transaction lock with member mutations",
    );
    assert!(
        report.removal_and_publish_were_serialized,
        "concurrent removal and prepared publish must have a total serial order",
    );
    assert!(
        report.stale_publish_rejected,
        "a prepared reply with the old revision must be rejected after the DM membership changes",
    );
    assert!(
        !report.stale_event_was_stored,
        "revision comparison and event insert must be atomic in one transaction",
    );
}
