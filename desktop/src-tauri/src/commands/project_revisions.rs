use std::collections::HashSet;

use tauri::State;

use crate::{app_state::AppState, relay::query_relay};

const MAX_PROJECT_REVISION_HEAD_COORDINATES: usize = 100;
const KIND_PROJECT_REVISION: u32 = 47_001;

fn valid_project_coordinate(coordinate: &str) -> bool {
    let mut parts = coordinate.splitn(3, ':');
    matches!(parts.next(), Some("30621"))
        && parts.next().is_some_and(|owner| {
            owner.len() == 64
                && owner
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        })
        && parts
            .next()
            .is_some_and(|slug| !slug.is_empty() && slug.len() <= 1_024)
}

/// Fetch the current actor-signed related-channel revision for each Project.
#[tauri::command]
pub async fn get_project_revision_heads(
    state: State<'_, AppState>,
    coordinates: Vec<String>,
) -> Result<Vec<nostr::Event>, String> {
    if coordinates.is_empty() {
        return Ok(Vec::new());
    }
    if coordinates.len() > MAX_PROJECT_REVISION_HEAD_COORDINATES {
        return Err("too many Project coordinates".into());
    }
    let mut unique = HashSet::with_capacity(coordinates.len());
    for coordinate in &coordinates {
        if !valid_project_coordinate(coordinate) {
            return Err("invalid Project coordinate".into());
        }
        if !unique.insert(coordinate) {
            return Err("duplicate Project coordinate".into());
        }
    }
    query_relay(
        &state,
        &[serde_json::json!({
            "kinds": [KIND_PROJECT_REVISION],
            "#a": coordinates,
            "project_revision_heads": true,
        })],
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::valid_project_coordinate;

    #[test]
    fn validates_canonical_project_coordinates() {
        assert!(valid_project_coordinate(&format!(
            "30621:{}:buzz",
            "a".repeat(64)
        )));
        assert!(!valid_project_coordinate(&format!(
            "30621:{}:buzz",
            "A".repeat(64)
        )));
        assert!(!valid_project_coordinate(&format!(
            "30617:{}:buzz",
            "a".repeat(64)
        )));
        assert!(!valid_project_coordinate(&format!(
            "30621:{}:",
            "a".repeat(64)
        )));
    }
}
