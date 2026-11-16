use super::*;

#[tokio::test]
async fn inventory_deduplicates_pages_and_orders_attribution_last() {
    let mut inventory = Inventory::new().await.unwrap();
    let hashes: Vec<_> = (0..501).map(|i| format!("{i:064x}")).collect();
    inventory.add_candidates(&hashes).await.unwrap();
    inventory.add_candidates(&hashes).await.unwrap();
    let page = inventory.candidates("").await.unwrap();
    assert_eq!(page.len(), 500);
    assert_eq!(inventory.candidates(&page[499].0).await.unwrap().len(), 1);
    let hash = &hashes[0];
    let entries: Vec<_> = [
        ("_uploads/claim", ObjectVersionKind::Object),
        ("_uploads/claim", ObjectVersionKind::DeleteMarker),
        ("blob", ObjectVersionKind::DeleteMarker),
        ("blob", ObjectVersionKind::Object),
    ]
    .into_iter()
    .map(|(key, kind)| {
        (
            hash.clone(),
            ObjectVersionEntry {
                key: key.into(),
                version_id: "v1".into(),
                kind,
                size: 1,
            },
        )
    })
    .collect();
    inventory.add_versions(&entries).await.unwrap();
    let ordered = inventory.versions(hash, (-1, 0)).await.unwrap();
    assert_eq!(
        ordered.iter().map(|v| v.0).collect::<Vec<_>>(),
        vec![0, 1, 2, 3]
    );
    assert_eq!(ordered[3].2.key, "_uploads/claim");
    // A large file family must cross a page without losing a version or
    // moving its final ownership record ahead of the remaining bytes.
    let many: Vec<_> = (0..501)
        .map(|i| {
            (
                hashes[1].clone(),
                ObjectVersionEntry {
                    key: "blob".into(),
                    version_id: format!("v{i}"),
                    kind: ObjectVersionKind::Object,
                    size: 1,
                },
            )
        })
        .chain(std::iter::once((hashes[1].clone(), entries[0].1.clone())))
        .collect();
    inventory.add_versions(&many).await.unwrap();
    let first = inventory.versions(&hashes[1], (-1, 0)).await.unwrap();
    assert_eq!(first.len(), 500);
    let last = first.last().unwrap();
    let rest = inventory
        .versions(&hashes[1], (last.0, last.1))
        .await
        .unwrap();
    assert_eq!(rest.len(), 2);
    assert_eq!(rest[1].2.key, "_uploads/claim");
    assert!(inventory
        .remember_marker("_uploads/", "key", "v1")
        .await
        .unwrap());
    assert!(inventory
        .remember_marker("_uploads/", "key", "v2")
        .await
        .unwrap());
    assert!(!inventory
        .remember_marker("_uploads/", "key", "v1")
        .await
        .unwrap());
    assert!(inventory.remember_marker("", "key", "v1").await.unwrap());
    inventory.close().await.unwrap();
}

fn object(key: &str) -> ObjectVersionEntry {
    ObjectVersionEntry {
        key: key.into(),
        version_id: "v1".into(),
        kind: ObjectVersionKind::Object,
        size: 1,
    }
}

#[tokio::test]
async fn capacity_covers_every_retained_collection() {
    for bytes_limited in [false, true] {
        for kind in 0..4 {
            let mut inventory = Inventory::with_limits(
                if bytes_limited { 1 } else { usize::MAX },
                if bytes_limited { usize::MAX } else { 0 },
            );
            let result = match kind {
                0 => inventory.add_candidates(&["a".repeat(64)]).await,
                1 => inventory.add_filename("a", "a.png").await,
                2 => {
                    inventory
                        .add_versions(&[("a".into(), object("a.png"))])
                        .await
                }
                _ => inventory
                    .remember_marker("prefix", "key", "v1")
                    .await
                    .map(|_| ()),
            };
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("capacity exceeded"));
            assert!(inventory.candidates.is_empty());
            assert!(inventory.filenames.is_empty());
            assert!(inventory.versions.is_empty());
            assert!(inventory.markers.is_empty());
            assert_eq!((inventory.bytes, inventory.entries), (0, 0));
        }
    }
}

#[tokio::test]
async fn duplicates_at_capacity_preserve_classification_without_allocating() {
    let mut inventory = Inventory::with_limits(usize::MAX, 3);
    let hash = "a".repeat(64);
    inventory
        .add_candidates(std::slice::from_ref(&hash))
        .await
        .unwrap();
    inventory.add_filename(&hash, "a.png").await.unwrap();
    assert!(inventory
        .remember_marker("prefix", "key", "v1")
        .await
        .unwrap());
    let bytes = inventory.bytes;
    inventory
        .add_classified_candidates(&[(hash.clone(), true)])
        .await
        .unwrap();
    inventory
        .add_classified_candidates(&[(hash.clone(), false)])
        .await
        .unwrap();
    inventory
        .add_candidates(std::slice::from_ref(&hash))
        .await
        .unwrap();
    inventory.add_filename(&hash, "a.png").await.unwrap();
    assert!(!inventory
        .remember_marker("prefix", "key", "v1")
        .await
        .unwrap());
    assert_eq!(inventory.candidates("").await.unwrap(), vec![(hash, true)]);
    assert!(inventory.unchecked_candidates("").await.unwrap().is_empty());
    assert_eq!((inventory.bytes, inventory.entries), (bytes, 3));
    assert!(inventory.add_candidates(&["b".repeat(64)]).await.is_err());
}

#[tokio::test]
async fn version_pages_do_not_cross_file_boundaries_and_keep_duplicate_entries() {
    let mut inventory = Inventory::new().await.unwrap();
    inventory
        .add_versions(&[
            ("a".into(), object("a.png")),
            ("a".into(), object("a.png")),
            ("b".into(), object("b.png")),
        ])
        .await
        .unwrap();
    let page = inventory.versions("a", (-1, 0)).await.unwrap();
    assert_eq!(page.len(), 2);
    assert!(page[0].1 < page[1].1);
    assert!(inventory
        .versions("a", (page[1].0, page[1].1))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(inventory.versions("b", (-1, 0)).await.unwrap().len(), 1);
}

#[tokio::test]
async fn fields_and_serialized_versions_are_bounded() {
    let large = "x".repeat(MAX_ENTRY_BYTES + 1);
    let mut inventory = Inventory::new().await.unwrap();
    assert!(inventory
        .add_candidates(std::slice::from_ref(&large))
        .await
        .is_err());
    assert!(inventory.add_filename("a", &large).await.is_err());
    for (prefix, key, version) in [
        (&*large, "k", "v"),
        ("p", &*large, "v"),
        ("p", "k", &*large),
    ] {
        assert!(inventory
            .remember_marker(prefix, key, version)
            .await
            .is_err());
    }
    // Each field fits but its serialized representation exceeds the record cap.
    let mut version = object(&"x".repeat(MAX_ENTRY_BYTES / 2));
    version.version_id = "v".repeat(MAX_ENTRY_BYTES / 2);
    assert!(inventory
        .add_versions(&[("a".into(), version)])
        .await
        .is_err());
    assert_eq!(inventory.entries, 0);
}
