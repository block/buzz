//! Repository-scoped BW reads. Never infer external facts from signed claims.
use crate::{
    app_state::AppState,
    bw_projection::{self, Input},
    relay,
};
use buzz_core_pkg::bw::{parse_json, Evidence, Trust};
use serde_json::{json, value::RawValue, Value};
use std::collections::{BTreeMap, BTreeSet};
use tauri::State;

const PAGE: usize = 500;
const KINDS: [u64; 5] = [30617, 1621, 46100, 1063, 1];

async fn page(state: &AppState, filter: Value) -> Result<Vec<String>, String> {
    let url = format!("{}/query", relay::relay_api_base_url_with_override(state));
    let body = serde_json::to_vec(&vec![filter]).map_err(|e| e.to_string())?;
    let auth = relay::build_nip98_auth_header(&reqwest::Method::POST, &url, &body, state)?;
    let response = state
        .http_client
        .post(&url)
        .header("Authorization", auth)
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("BW history unavailable ({})", response.status()));
    }
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    // Reject lossy JSON before extracting the original event slices.
    parse_json(&bytes).map_err(|_| "Invalid BW relay JSON".to_string())?;
    let rows: Vec<Box<RawValue>> = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(rows.into_iter().map(|r| r.get().to_owned()).collect())
}

fn tag<'a>(event: &'a Value, name: &str) -> Option<&'a str> {
    event["tags"].as_array()?.iter().find(|t| t[0] == name)?[1].as_str()
}

/// Load without trusting generic #a post-filtering as a completeness signal.
/// Kind-only candidate pages are drained, then repository matching is local.
/// A saturated timestamp bucket fails closed rather than silently losing events.
pub(crate) async fn load(state: &AppState, repo: &str) -> Result<Input, String> {
    let parts: Vec<_> = repo.split(':').collect();
    if parts.len() != 3 || parts[0] != "30617" {
        return Err("Invalid BW repository".into());
    }
    super::project_git_workflow::validate_repo_address(repo, parts[1])?;
    let now = nostr::Timestamp::now().as_secs();
    let mut events = BTreeMap::new();
    for kind in KINDS {
        for raw in enumerate(|filter| page(state, filter), kind, now, PAGE).await? {
            let event = parse_json(raw.as_bytes()).map_err(|e| e.to_string())?;
            if (kind == 30617 && event["pubkey"] == parts[1] && tag(&event, "d") == Some(parts[2]))
                || (kind != 30617 && tag(&event, "a") == Some(repo))
            {
                events.insert(raw.clone(), raw);
            }
        }
    }
    // Fetch missing exact references as well (including foreign bindings so the
    // Core can reject them). A missing result remains pending in the consumer.
    let mut requested = BTreeSet::new();
    loop {
        let mut ids = BTreeSet::new();
        for raw in events.values() {
            let event = parse_json(raw.as_bytes()).map_err(|e| e.to_string())?;
            for t in event["tags"].as_array().into_iter().flatten() {
                if matches!(
                    t[0].as_str(),
                    Some(
                        "issue"
                            | "policy"
                            | "previous"
                            | "prior"
                            | "delegation"
                            | "set"
                            | "run"
                            | "e"
                    )
                ) {
                    if let Some(id) = t[1].as_str() {
                        ids.insert(id.to_owned());
                    }
                }
            }
            if event["kind"] == 46100 {
                if let Some(content) = event["content"].as_str() {
                    if let Ok(body) = parse_json(content.as_bytes()) {
                        reference_ids(&body, &mut ids);
                    }
                }
            }
        }
        let ids: Vec<_> = ids
            .into_iter()
            .filter(|id| {
                id.len() == 64
                    && id.bytes().all(|b| b.is_ascii_hexdigit())
                    && requested.insert(id.clone())
            })
            .collect();
        if ids.is_empty() {
            break;
        }
        for chunk in ids.chunks(100) {
            for raw in page(state, json!({"kinds":KINDS,"ids":chunk,"limit":PAGE})).await? {
                events.insert(raw.clone(), raw);
            }
        }
    }
    Ok(Input {
        trust: Trust {
            community: relay::relay_api_base_url_with_override(state),
            repo: repo.into(),
            owner: parts[1].into(),
        },
        external: Evidence {
            git_readbacks: vec![],
            git_ancestry: vec![],
            provider_readbacks: vec![],
            downloads: vec![],
            host_authorization: json!({"allowed":false}),
        },
        now,
        events: events.into_values().collect(),
    })
}

fn reference_ids(value: &Value, ids: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                if matches!(
                    key.as_str(),
                    "assignment"
                        | "update"
                        | "implemented"
                        | "pipeline"
                        | "request"
                        | "run"
                        | "artifact"
                        | "set"
                        | "target"
                        | "issue"
                ) {
                    if let Some(id) = value.as_str() {
                        ids.insert(id.to_owned());
                    }
                }
                reference_ids(value, ids);
            }
        }
        Value::Array(values) => {
            for value in values {
                reference_ids(value, ids);
            }
        }
        _ => {}
    }
}

/// Return a transient Core projection for the selected repository/community.
#[tauri::command]
pub async fn get_project_bw(repo: String, state: State<'_, AppState>) -> Result<Value, String> {
    Ok(bw_projection::snapshot(&load(&state, &repo).await?))
}

// Shared by the live loader and fake-page tests. Candidate-page fullness, not
// post-filtered repository row count, controls cursor advancement.
async fn enumerate<F, Fut>(
    mut fetch: F,
    kind: u64,
    now: u64,
    limit: usize,
) -> Result<Vec<String>, String>
where
    F: FnMut(Value) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<String>, String>>,
{
    let mut until = now;
    let mut result = Vec::new();
    loop {
        let rows = fetch(json!({"kinds":[kind],"until":until,"limit":limit})).await?;
        let full = rows.len() >= limit;
        let mut oldest = until;
        for raw in &rows {
            let event = parse_json(raw.as_bytes()).map_err(|e| e.to_string())?;
            let time = event["created_at"].as_u64().ok_or("Missing event time")?;
            if time > until {
                return Err("BW history returned an out-of-page timestamp".into());
            }
            oldest = oldest.min(time);
        }
        result.extend(rows);
        if result.len() > 100_000 {
            return Err("BW history incomplete: candidate scan limit".into());
        }
        if !full {
            return Ok(result);
        }
        let boundary =
            fetch(json!({"kinds":[kind],"since":oldest,"until":oldest,"limit":limit})).await?;
        if boundary.len() >= limit {
            return Err("BW history incomplete: timestamp page saturated".into());
        }
        result.extend(boundary);
        if oldest == 0 {
            return Ok(result);
        }
        until = oldest - 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn pagination_drains_boundary_and_keeps_original_bytes() {
        let newer = "{ \"created_at\": 3 }".to_owned();
        let older = "{\"created_at\":2}".to_owned();
        let last = "{\"created_at\":1}".to_owned();
        let pages = [
            vec![newer.clone(), older.clone()],
            vec![older.clone()],
            vec![last.clone()],
        ];
        let mut call = 0;
        let result = enumerate(
            |filter| {
                match call {
                    0 => assert_eq!(filter["until"], 3),
                    1 => {
                        assert_eq!(filter["since"], 2);
                        assert_eq!(filter["until"], 2);
                    }
                    2 => assert_eq!(filter["until"], 1),
                    _ => panic!("unexpected page"),
                }
                let rows = pages[call].clone();
                call += 1;
                std::future::ready(Ok(rows))
            },
            46100,
            3,
            2,
        )
        .await
        .expect("pages");
        assert_eq!(result, vec![newer, older.clone(), older, last]);
    }
    #[tokio::test]
    async fn saturated_second_is_incomplete_not_a_truncated_success() {
        let result = enumerate(
            |_| std::future::ready(Ok(vec!["{\"created_at\":2}".into(); 2])),
            46100,
            3,
            2,
        )
        .await;
        assert!(result.expect_err("saturated").contains("saturated"));
    }
}
