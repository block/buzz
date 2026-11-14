//! Classify the bounded selection together; never erase surviving ancestry.
use super::*;
use std::collections::HashMap;

/// Order all references, including edits and legacy events without metadata.
pub(super) fn children_first(
    selected: Vec<Event>,
    metadata: &HashMap<String, Value>,
) -> Result<Vec<Event>> {
    let mut by_id: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, event) in selected.iter().enumerate() {
        by_id.entry(hex::encode(&event.id)).or_default().push(i);
    }
    let mut parents = vec![HashSet::new(); selected.len()];
    let mut children = vec![0usize; selected.len()];
    for (i, event) in selected.iter().enumerate() {
        let mut refs = Vec::new();
        if let Some(tags) = event.tags.as_array() {
            for tag in tags {
                if tag[0] == "e" {
                    if let Some(id) = tag[1].as_str() {
                        refs.push(id.to_ascii_lowercase());
                    }
                }
            }
        }
        if let Some(meta) = metadata.get(&hex::encode(&event.id)) {
            for field in ["parent_event_id", "root_event_id"] {
                if let Some(id) = meta[field].as_str().and_then(|id| id.strip_prefix("\\x")) {
                    refs.push(id.to_owned());
                }
            }
        }
        for id in refs {
            for &parent in by_id.get(&id).into_iter().flatten() {
                if i != parent && parents[i].insert(parent) {
                    children[parent] += 1;
                }
            }
        }
    }
    let mut ready = std::collections::BinaryHeap::new();
    for (i, count) in children.iter().enumerate() {
        if *count == 0 {
            ready.push(std::cmp::Reverse(i));
        }
    }
    let count = selected.len();
    let mut selected: Vec<Option<Event>> = selected.into_iter().map(Some).collect();
    let mut ordered = Vec::with_capacity(count);
    while let Some(std::cmp::Reverse(i)) = ready.pop() {
        if let Some(event) = selected[i].take() {
            ordered.push(event);
        }
        for &parent in &parents[i] {
            children[parent] -= 1;
            if children[parent] == 0 {
                ready.push(std::cmp::Reverse(parent));
            }
        }
    }
    if ordered.len() != count {
        return Err(invalid("cyclic selected references; nothing deleted"));
    }
    Ok(ordered)
}

pub(super) async fn needed(
    conn: &mut PgConnection,
    community: Uuid,
    selected: &[Event],
    authors: &[Vec<u8>],
) -> Result<HashSet<String>> {
    if selected.is_empty() {
        return Ok(HashSet::new());
    }
    let keys: Vec<Value> = selected
        .iter()
        .map(|e| {
            json!({
                "id": hex::encode(&e.id), "created_at": e.created_at
            })
        })
        .collect();
    let ids: Vec<String> = sqlx::query_scalar(include_str!("retention.sql"))
        .bind(community)
        .bind(json!(keys))
        .bind(authors)
        .fetch_all(conn)
        .await?;
    Ok(ids.into_iter().collect())
}

pub(super) async fn metadata(
    conn: &mut PgConnection,
    community: Uuid,
    selected: &[Event],
) -> Result<HashMap<String, Value>> {
    let ids: Vec<Vec<u8>> = selected.iter().map(|e| e.id.clone()).collect();
    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT encode(event_id,'hex'),to_jsonb(tm) FROM thread_metadata tm
         WHERE community_id=$1 AND event_id=ANY($2::bytea[])",
    )
    .bind(community)
    .bind(ids)
    .fetch_all(conn)
    .await?;
    Ok(rows.into_iter().collect())
}
