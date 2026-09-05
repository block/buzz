use super::*;

#[test]
fn config_reads_resolve_follower_and_relay_only_without_creating_lifecycle() {
    let state = crate::app_state::build_app_state();
    state
        .managed_agent_authority_ready
        .store(true, std::sync::atomic::Ordering::Release);
    let pubkey = "bc".repeat(32);
    let mut payload = test_relay_payload(&pubkey);
    let mut overlay = PrivateConfigOverlay::default();
    overlay.insert(payload.clone()).unwrap();
    let disk = overlay.materialize_relay_only_record(&pubkey, &[]).unwrap();
    payload.config.model = Some("new model".into());
    payload
        .config
        .env_vars
        .insert("API_KEY".into(), "new credential".into());
    state
        .private_managed_agent_overlay
        .lock()
        .unwrap()
        .insert(payload)
        .unwrap();
    let local = vec![disk.clone()];
    for records in [&local[..], &[][..]] {
        let resolved = resolved_record_for_read(&state, records, &pubkey).unwrap();
        assert_eq!(resolved.model.as_deref(), Some("new model"));
        assert_eq!(
            resolved.env_vars.get("API_KEY").map(String::as_str),
            Some("new credential")
        );
    }
    assert_eq!(
        local,
        vec![disk],
        "reads cannot persist overlay into migration seed"
    );
    assert!(resolved_record_for_read(&state, &[], &"cd".repeat(32))
        .unwrap_err()
        .contains("not found"));
    state
        .managed_agent_authority_ready
        .store(false, std::sync::atomic::Ordering::Release);
    assert!(resolved_record_for_read(&state, &local, &pubkey)
        .unwrap_err()
        .contains("authority is unavailable"));
}
