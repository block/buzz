//! Ownership changes and community boundaries through the real media handler.
use super::*;

#[tokio::test]
async fn agent_claims_in_other_communities_protect_blobs_and_are_not_selected() {
    let hashes: Vec<_> = (1..=4).map(|n| format!("{n:064x}")).collect();
    let db = FakeDb {
        targets: vec![(uuid::Uuid::from_u128(1), AGENT.into())],
        ..Default::default()
    };
    let mut media = FakeStorage::default();
    media.upload(&hashes[0], AGENT, 1); // Owned here: delete.
    media.upload(&hashes[1], AGENT, 2); // Owned elsewhere or missing evidence: untouched.
    media.upload(&hashes[2], ALICE, 1);
    media.upload(&hashes[2], AGENT, 2); // Same key outside scope protects shared bytes.
    media.upload(&hashes[3], AGENT, 1);
    media.upload(&hashes[3], AGENT, 2); // Sharing across communities protects agent's bytes too.
    for hash in &hashes {
        media.object(format!("{hash}.png"), ObjectVersionKind::Object);
    }
    let before = media.entries.borrow().clone();
    let preview = run_handler(&db, &media, ALICE, false, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(preview["selected_files"], 3);
    assert_eq!(preview["kept_files"], 2);
    assert_eq!(*media.entries.borrow(), before);
    assert!(db.removed.borrow().is_empty());
    let result = run_handler(&db, &media, ALICE, true, &mut Vec::new())
        .await
        .unwrap();
    assert_eq!(result["deleted_versions"], 2);
    assert!(media
        .deleted
        .borrow()
        .iter()
        .all(|entry| entry.key.contains(&hashes[0])));
    assert_eq!(
        *db.removed.borrow(),
        vec![hashes[0].clone(), hashes[2].clone(), hashes[3].clone()]
    );
}

#[tokio::test]
async fn changed_ownership_aborts_before_mutation_and_retry_rediscovers() {
    let community = uuid::Uuid::from_u128(1);
    // Transfer away/missing row, new ownership, and same-key transfer between communities.
    for changed_after in [1, 2] {
        for changed in [
            vec![],
            vec![(community, AGENT.into()), (community, BOB.into())],
            vec![(uuid::Uuid::from_u128(2), AGENT.into())],
        ] {
            let mut db = FakeDb {
                targets: vec![(community, AGENT.into())],
                changed_targets: Some((changed_after, changed.clone())),
                ..Default::default()
            };
            let hash = "a".repeat(64);
            let mut media = FakeStorage::default();
            media.upload(&hash, AGENT, 1);
            media.object(format!("{hash}.png"), ObjectVersionKind::Object);
            let before = media.entries.borrow().clone();
            let error = run_handler(&db, &media, ALICE, true, &mut Vec::new())
                .await
                .unwrap_err();
            assert!(error.to_string().contains("ownership changed"));
            assert_eq!(media.deleted.borrow().len(), changed_after - 1);
            if changed_after == 1 {
                assert_eq!(*media.entries.borrow(), before);
            }
            assert!(media
                .entries
                .borrow()
                .iter()
                .any(|entry| entry.key.starts_with("_uploads/")));
            assert!(db.removed.borrow().is_empty());
            db.targets = changed;
            db.changed_targets = None;
            let result = run_handler(&db, &media, ALICE, true, &mut Vec::new())
                .await
                .unwrap();
            assert_eq!(
                result["selected_files"],
                if db.targets.contains(&(community, AGENT.into())) {
                    1
                } else {
                    0
                }
            );
        }
    }
}

#[tokio::test]
async fn changed_ownership_aborts_at_uploader_removal_with_or_without_storage_deletes() {
    for shared in [false, true] {
        let hash = "a".repeat(64);
        let db = FakeDb {
            targets: vec![(uuid::Uuid::from_u128(1), AGENT.into())],
            changed_targets: Some((if shared { 1 } else { 3 }, vec![])),
            shared: if shared { vec![hash.clone()] } else { vec![] },
            ..Default::default()
        };
        let mut media = FakeStorage::default();
        media.upload(&hash, AGENT, 1);
        media.object(format!("{hash}.png"), ObjectVersionKind::Object);
        let before = media.entries.borrow().clone();
        let error = run_handler(&db, &media, ALICE, true, &mut Vec::new())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("ownership changed"));
        assert!(db.removed.borrow().is_empty());
        assert_eq!(media.deleted.borrow().len(), if shared { 0 } else { 2 });
        if shared {
            assert_eq!(*media.entries.borrow(), before);
        }
    }
}

#[tokio::test]
async fn empty_inventory_rechecks_new_agents_before_reporting_success() {
    let db = FakeDb {
        changed_targets: Some((1, vec![(uuid::Uuid::from_u128(1), AGENT.into())])),
        ..Default::default()
    };
    let media = FakeStorage::default();
    let error = run_handler(&db, &media, ALICE, true, &mut Vec::new())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("ownership changed"));
    assert!(db.removed.borrow().is_empty());
    assert!(media.deleted.borrow().is_empty());
}
