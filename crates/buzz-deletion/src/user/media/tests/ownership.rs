//! Community boundaries through the real media handler.
use super::*;

#[tokio::test]
async fn agent_claims_in_other_communities_protect_blobs_and_are_not_selected() {
    let hashes: Vec<_> = (1..=4).map(|n| format!("{n:064x}")).collect();
    let db = FakeDb {
        targets: vec![(uuid::Uuid::from_u128(1), AGENT.into())],
        candidates: vec![hashes[0].clone(), hashes[3].clone()],
        evidence: vec![
            MediaUploadEvidence {
                sha256: hashes[0].clone(),
                community_id: uuid::Uuid::from_u128(1),
                pubkey: AGENT.into(),
                source: "audit_log".into(),
            },
            MediaUploadEvidence {
                sha256: hashes[3].clone(),
                community_id: uuid::Uuid::from_u128(2),
                pubkey: ALICE.into(),
                source: "media_uploaders".into(),
            },
        ],
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
    let mut output = Vec::new();
    let preview = run_handler(&db, &media, ALICE, false, &mut output)
        .await
        .unwrap();
    let stage = stage_report::stage_preview(&db, &media, ALICE)
        .await
        .unwrap();
    let streamed: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(stage["files"], streamed["files"]);
    assert_eq!(stage["target_pubkeys"], streamed["target_pubkeys"]);
    for (field, value) in preview.as_object().unwrap() {
        assert_eq!(&stage[field], value);
    }
    assert_eq!(preview["selected_files"], 3);
    assert_eq!(preview["kept_files"], 2);
    let files = stage["files"].as_array().unwrap();
    assert_eq!(files[0]["file_hash"], hashes[0]);
    assert_eq!(files[0]["attribution_status"], "attributed");
    assert_eq!(
        files[0]["target_uploaders"],
        json!([
            {"identity": "agent", "pubkey": AGENT, "community_id": uuid::Uuid::from_u128(1), "source": "audit_log"},
            {"identity": "agent", "pubkey": AGENT, "community_id": uuid::Uuid::from_u128(1), "source": "storage_upload_record"}
        ])
    );
    assert_eq!(files[1]["file_hash"], hashes[2]);
    assert_eq!(files[1]["attribution_status"], "shared");
    assert_eq!(files[1]["has_other_uploader"], true);
    assert_eq!(
        files[1]["target_uploaders"],
        json!([{"identity": "human", "pubkey": ALICE, "community_id": uuid::Uuid::from_u128(1), "source": "storage_upload_record"}])
    );
    assert_eq!(files[2]["file_hash"], hashes[3]);
    assert_eq!(files[2]["attribution_status"], "shared");
    assert_eq!(
        files[2]["target_uploaders"],
        json!([
            {"identity": "agent", "pubkey": AGENT, "community_id": uuid::Uuid::from_u128(1), "source": "storage_upload_record"},
            {"identity": "human", "pubkey": ALICE, "community_id": uuid::Uuid::from_u128(2), "source": "media_uploaders"}
        ])
    );
    assert_eq!(*media.entries.borrow(), before);
    assert!(db.removed.borrow().is_empty());
    output.clear();
    let result = run_handler(&db, &media, ALICE, true, &mut output)
        .await
        .unwrap();
    let destructive: Value = serde_json::from_slice(&output).unwrap();
    for (preview, applied) in files.iter().zip(destructive["files"].as_array().unwrap()) {
        assert_eq!(preview["decision"], applied["decision"]);
        assert!(applied.get("target_uploaders").is_none());
        assert!(applied.get("attribution_status").is_none());
    }
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
