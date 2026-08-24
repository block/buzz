use std::collections::HashSet;

use tauri::State;

use crate::{app_state::AppState, relay::query_relay};

// The relay clamps a single filter to this many events. Keep exact-ID reads in
// chunks so a large workflow list cannot silently lose late presentations.
const EVENT_QUERY_CHUNK_SIZE: usize = 1_000;

const GET_EVENT_KINDS: [u32; 15] = [
    0,
    1,
    3,
    5,
    7,
    9,
    30078,
    40002,
    40003,
    40008,
    40099,
    40100,
    45001,
    45003,
    buzz_core_pkg::kind::KIND_HUDDLE_STARTED,
];

#[tauri::command]
pub async fn get_event(event_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let events = query_relay(
        &state,
        &[serde_json::json!({
            "ids": [event_id],
            "kinds": GET_EVENT_KINDS,
            "limit": 1
        })],
    )
    .await?;

    let event = events
        .first()
        .ok_or_else(|| "event not found".to_string())?;
    serde_json::to_string(event).map_err(|error| format!("serialize event: {error}"))
}

/// Resolve many exact event IDs in relay-sized chunks. Callers still validate
/// event kind, channel scope, and requested ID before using presentation data.
fn normalized_event_id_chunks(event_ids: Vec<String>) -> Vec<Vec<String>> {
    let mut seen_ids = HashSet::new();
    let event_ids = event_ids
        .into_iter()
        .map(|event_id| event_id.trim().to_ascii_lowercase())
        .filter(|event_id| event_id.len() == 64 && event_id.chars().all(|c| c.is_ascii_hexdigit()))
        .filter(|event_id| seen_ids.insert(event_id.clone()))
        .collect::<Vec<_>>();
    event_ids
        .chunks(EVENT_QUERY_CHUNK_SIZE)
        .map(<[String]>::to_vec)
        .collect()
}

#[tauri::command]
pub async fn get_events(
    event_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let event_id_chunks = normalized_event_id_chunks(event_ids);
    if event_id_chunks.is_empty() {
        return Ok(Vec::new());
    }

    let mut events_by_id = std::collections::HashMap::new();
    for event_ids in event_id_chunks {
        let events = query_relay(
            &state,
            &[serde_json::json!({
                "ids": event_ids,
                "kinds": GET_EVENT_KINDS,
                "limit": event_ids.len()
            })],
        )
        .await?;
        for event in events {
            events_by_id.entry(event.id).or_insert(event);
        }
    }

    events_by_id
        .into_values()
        .map(|event| {
            serde_json::to_value(event).map_err(|error| format!("serialize event: {error}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_exact_relay_ceiling_in_one_chunk() {
        let chunks = normalized_event_id_chunks(
            (0..EVENT_QUERY_CHUNK_SIZE)
                .map(|index| format!("{index:064x}"))
                .collect(),
        );

        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), EVENT_QUERY_CHUNK_SIZE);
    }

    #[test]
    fn normalizes_deduplicates_and_keeps_ids_beyond_relay_ceiling() {
        let last_id = format!("{:064x}", EVENT_QUERY_CHUNK_SIZE);
        let mut event_ids = (0..=EVENT_QUERY_CHUNK_SIZE)
            .map(|index| format!("{index:064X}"))
            .collect::<Vec<_>>();
        event_ids.extend(["not-an-event-id".to_string(), format!(" {last_id} ")]);

        let chunks = normalized_event_id_chunks(event_ids);

        assert_eq!(chunks.iter().map(Vec::len).sum::<usize>(), 1_001);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), EVENT_QUERY_CHUNK_SIZE);
        assert_eq!(chunks[1], [last_id]);
    }
}
