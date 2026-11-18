//! Community boundaries through the real media handler.
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
