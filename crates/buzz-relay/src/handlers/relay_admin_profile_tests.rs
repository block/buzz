use super::*;
use nostr::{EventBuilder, Keys, Kind, Tag};

#[test]
fn name_validation_bounds_unicode_and_rejects_control_characters() {
    assert_eq!(
        normalize_community_name("  ATL BitLab  ").unwrap(),
        "ATL BitLab"
    );
    assert!(normalize_community_name(&"é".repeat(128)).is_ok());
    for name in ["", "   ", "line\nbreak", "tab\tname", &"é".repeat(129)] {
        assert!(normalize_community_name(name).is_err(), "{name:?}");
    }
}

async fn submit(
    state: &Arc<AppState>,
    tenant: &TenantContext,
    keys: &Keys,
    tags: &[&[&str]],
) -> Result<(), RelayAdminError> {
    let event = EventBuilder::new(Kind::Custom(RELAY_ADMIN_SET_WORKSPACE_PROFILE as u16), "")
        .tags(
            tags.iter()
                .map(|tag| Tag::parse(tag.iter().copied()).unwrap()),
        )
        .sign_with_keys(keys)
        .unwrap();
    handle_relay_admin_event(tenant, state, &event).await
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn names_are_authorized_atomic_idempotent_and_host_scoped() {
    let host = format!("name-{}.example", uuid::Uuid::new_v4().simple());
    let (state, tenant) = super::postgres_tests::workspace_profile_test_state(&host, false).await;
    let other_host = format!("other-{host}");
    let other_record = state
        .db
        .ensure_configured_community(&other_host)
        .await
        .unwrap();
    let other = TenantContext::resolved(other_record.id, &other_host);
    let owner = Keys::generate();
    let admin = Keys::generate();
    let member = Keys::generate();
    let name = &[&["name", "ATL BitLab"][..]][..];
    // The icon's rosterless exception must never grant name authority.
    assert!(submit(&state, &tenant, &member, name).await.is_err());
    for (keys, role) in [(&owner, "owner"), (&admin, "admin"), (&member, "member")] {
        state
            .db
            .add_relay_member(tenant.community(), &keys.public_key().to_hex(), role, None)
            .await
            .unwrap();
    }
    let before = crate::nip11::nip11_document(&state, &host).await;
    assert!(before.community_profile.unwrap().name.is_none());
    assert_eq!(before.name, "Buzz Relay");
    submit(
        &state,
        &tenant,
        &owner,
        &[&["icon", "https://example.com/icon.png"]],
    )
    .await
    .unwrap();
    for _ in 0..2 {
        submit(&state, &tenant, &owner, name).await.unwrap();
    }
    assert_eq!(
        state
            .db
            .get_community_icon(tenant.community())
            .await
            .unwrap()
            .as_deref(),
        Some("https://example.com/icon.png")
    );
    assert!(submit(&state, &tenant, &member, &[&["name", "hijack"]])
        .await
        .is_err());
    assert!(submit(&state, &other, &owner, name).await.is_err());
    assert!(submit(
        &state,
        &tenant,
        &owner,
        &[&["name", "new"], &["icon", "invalid"]]
    )
    .await
    .is_err());
    assert!(
        submit(&state, &tenant, &owner, &[&["name", ""], &["icon", ""]])
            .await
            .is_err()
    );
    assert!(submit(&state, &tenant, &owner, &[&["name"]]).await.is_err());
    assert!(submit(
        &state,
        &tenant,
        &owner,
        &[&["name", "one"], &["name", "two"]]
    )
    .await
    .is_err());
    assert_eq!(
        state
            .db
            .get_community_name(tenant.community())
            .await
            .unwrap()
            .as_deref(),
        Some("ATL BitLab")
    );
    assert!(state
        .db
        .get_community_name(other.community())
        .await
        .unwrap()
        .is_none());
    submit(&state, &tenant, &admin, &[&["name", "  New Name  "]])
        .await
        .unwrap();
    // Old icon clients preserve the new name, including legacy empty clears.
    submit(&state, &tenant, &owner, &[]).await.unwrap();
    assert!(state
        .db
        .get_community_icon(tenant.community())
        .await
        .unwrap()
        .is_none());
    let doc = crate::nip11::nip11_document(&state, &host).await;
    assert_eq!(doc.name, "New Name");
    assert_eq!(
        doc.community_profile.unwrap().name.as_deref(),
        Some("New Name")
    );
    assert_eq!(
        crate::nip11::nip11_document(&state, &other_host).await.name,
        "Buzz Relay"
    );
    let unknown = crate::nip11::nip11_document(&state, "unmapped.example").await;
    assert_eq!(unknown.name, "Buzz Relay");
    assert!(unknown.community_profile.is_none());
    assert_eq!(
        state
            .db
            .get_relay_member(tenant.community(), &owner.public_key().to_hex())
            .await
            .unwrap()
            .unwrap()
            .role,
        "owner"
    );
}
