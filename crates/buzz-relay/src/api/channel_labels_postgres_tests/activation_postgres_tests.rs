use super::*;

#[tokio::test]
#[ignore = "requires Postgres"]
async fn capability_is_host_bound_and_disabling_commands_preserves_metadata() {
    let mut f = Fixture::new().await;
    let mut config = (*f.state.config).clone();
    config.relay_private_key = Some(f.state.relay_keypair.secret_key().to_secret_hex());
    Arc::get_mut(&mut f.state).unwrap().config = Arc::new(config.clone());
    let info = crate::nip11::nip11_document(&f.state, &f.host).await;
    assert!(info
        .supported_extensions
        .unwrap()
        .iter()
        .any(|ext| ext == "nip-cl"));
    let unknown = crate::nip11::nip11_document(&f.state, "unknown.example").await;
    assert!(!unknown
        .supported_extensions
        .unwrap()
        .iter()
        .any(|ext| ext == "nip-cl"));
    let channel = Uuid::new_v4();
    f.create(channel, &["retained"], "open").await;
    let before = f.snapshot(channel, &["retained"]).await;
    config.nip_cl_enabled = false;
    Arc::get_mut(&mut f.state).unwrap().config = Arc::new(config);
    let info = crate::nip11::nip11_document(&f.state, &f.host).await;
    assert!(!info
        .supported_extensions
        .unwrap()
        .iter()
        .any(|ext| ext == "nip-cl"));
    let mutation = f.command(&f.owner, channel, 9002, &[("remove-label", "retained")]);
    f.submit(&f.owner, &mutation, false, "restricted: nip-cl-rejected")
        .await;
    let rename = f.command(&f.owner, channel, 9002, &[("name", "after-disable")]);
    f.submit(&f.owner, &rename, true, "").await;
    let after = f.snapshot(channel, &["retained"]).await;
    assert!(after.created_at > before.created_at);
    assert!(after
        .tags
        .iter()
        .any(|tag| tag.as_slice() == ["name", "after-disable"]));
    let evidence: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE community_id=$1 AND id=$2")
            .bind(f.community.as_uuid())
            .bind(mutation.id.as_bytes().as_slice())
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(evidence, 0);
}
