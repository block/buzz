//! Capacity failures must stop the real handler before any external mutation.
use super::*;

#[tokio::test]
async fn late_capacity_failure_never_deletes_an_already_discovered_file() {
    for (max_bytes, max_entries) in [(usize::MAX, 5), (2000, usize::MAX)] {
        let first = "a".repeat(64);
        let last = "b".repeat(64);
        let db = FakeDb {
            candidates: vec![first.clone(), last.clone()],
            ..Default::default()
        };
        let mut media = FakeStorage::default();
        media.object(format!("{first}.png"), ObjectVersionKind::Object);
        for i in 0..10 {
            media.object(format!("{last}.{i}.png"), ObjectVersionKind::Object);
        }
        let before = media.entries.borrow().clone();
        let mut output = Vec::new();
        let error = run_handler_with_inventory(
            &db,
            &media,
            ALICE,
            true,
            &mut output,
            Inventory::with_limits(max_bytes, max_entries),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("capacity exceeded"));
        // The first complete file fits, but discovery of the second exhausts
        // capacity. No streaming deletion of that first file is allowed.
        assert!(media.passes.borrow().contains(&format!("{last}.")));
        assert!(media.deleted.borrow().is_empty());
        assert!(db.removed.borrow().is_empty());
        assert_eq!(*media.entries.borrow(), before);
        assert!(output.is_empty());
    }
}
