//! Agent-signed NIP-34 issue lifecycle status commands.
//!
//! P4E: once an issue has a NIP-BW issue-state history (enrolled), its
//! lifecycle belongs exclusively to that state machine
//! (`crates/buzz-core/src/bw/semantics.rs`) — never this legacy 1630-1633
//! mirror. `sign_project_issue_status` refuses for such an issue before it
//! ever builds or signs the legacy event, the same "no separate, weaker
//! gate" discipline `project_bw_write.rs` applies to its own record types.

use super::project_bw::load as load_bw_input;
use super::project_git_workflow::{
    normalize_event_id, project_owner_identity, validate_repo_address,
};
use crate::app_state::AppState;
use crate::bw_projection;
use crate::relay::submit_signed_event_with_keys;
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde::Deserialize;
use tauri::{AppHandle, State};

/// Repository-scoped metadata for an agent-signed issue lifecycle status.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectIssueStatusInput {
    target_owner: String,
    repo_address: String,
    issue_id: String,
    issue_author: String,
    status: String,
    created_at: u64,
}

/// Build a NIP-34 issue status event (kinds 1630/1631/1632/1633).
fn build_issue_status_event(
    keys: &Keys,
    repo_address: &str,
    issue_id: &str,
    issue_author: &str,
    status: &str,
    created_at: u64,
) -> Result<String, String> {
    let owner = keys.public_key().to_hex();
    validate_repo_address(repo_address, &owner)?;
    let issue_id =
        normalize_event_id(issue_id).ok_or_else(|| "Invalid issue event ID.".to_string())?;
    let issue_author =
        normalize_event_id(issue_author).ok_or_else(|| "Invalid issue author.".to_string())?;
    let kind = match status {
        "open" => Kind::Custom(1630),
        "resolved" => Kind::Custom(1631),
        "closed" => Kind::Custom(1632),
        "draft" => Kind::Custom(1633),
        _ => return Err("Invalid issue lifecycle status.".to_string()),
    };
    let mut raw_tags = vec![
        vec!["e", issue_id.as_str(), "", "root"],
        vec!["a", repo_address],
        vec!["p", owner.as_str()],
    ];
    if issue_author != owner {
        raw_tags.push(vec!["p", issue_author.as_str()]);
    }
    let tags = raw_tags
        .into_iter()
        .map(Tag::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("build issue status tags: {error}"))?;
    EventBuilder::new(kind, "")
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at.max(Timestamp::now().as_secs())))
        .sign_with_keys(keys)
        .map(|event| event.as_json())
        .map_err(|error| format!("sign issue status: {error}"))
}

/// Refuse a legacy 1630-1633 lifecycle change for an issue whose NIP-BW
/// history already carries an `issue-state` chain (i.e. it is enrolled).
/// A repository the BW bridge cannot even load (not BW-addressable, or a
/// transient fetch failure) is treated as not-yet-determined rather than
/// blocked: this legacy command must not go globally unavailable for
/// ordinary non-BW projects whenever that read fails. It is deliberately
/// not fail-open on the *enrolled* branch itself — once Core's own
/// projection shows a state for this issue, this command refuses
/// unconditionally, with no override.
async fn reject_if_bw_enrolled(
    state: &AppState,
    repo_address: &str,
    issue_id: &str,
) -> Result<(), String> {
    let bw_input = match load_bw_input(state, repo_address).await {
        Ok(input) => input,
        Err(_) => return Ok(()),
    };
    let (consumer, _) = bw_projection::replay(&bw_input);
    if !consumer.projection()["issues"][issue_id].is_null() {
        return Err(
            "This issue is enrolled in NIP-BW workflow tracking; legacy issue-status \
             changes are disabled for it. Use the BW record commands instead."
                .to_string(),
        );
    }
    Ok(())
}

/// Sign and submit an issue lifecycle status as the repository owner.
#[tauri::command]
pub async fn sign_project_issue_status(
    input: ProjectIssueStatusInput,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let target_owner = input.target_owner.trim().to_ascii_lowercase();
    if normalize_event_id(&target_owner).is_none() {
        return Err("Invalid target repository owner.".to_string());
    }
    let issue_id =
        normalize_event_id(&input.issue_id).ok_or_else(|| "Invalid issue event ID.".to_string())?;
    reject_if_bw_enrolled(&state, &input.repo_address, &issue_id).await?;
    let identity = project_owner_identity(&app, &state, &target_owner)?;
    let event = Event::from_json(build_issue_status_event(
        &identity.keys,
        &input.repo_address,
        &input.issue_id,
        &input.issue_author,
        &input.status,
        input.created_at,
    )?)
    .map_err(|error| format!("parse signed issue status: {error}"))?;
    submit_signed_event_with_keys(&event, &state, &identity.keys, identity.auth_tag.as_deref())
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::build_issue_status_event;
    use nostr::{Event, JsonUtil, Keys, Timestamp};

    #[test]
    fn issue_status_carries_repo_coordinate_and_recipients() {
        let keys = Keys::generate();
        let owner = keys.public_key().to_hex();
        let repo_address = format!("30617:{owner}:buzz");
        let author = "b".repeat(64);
        let event = Event::from_json(
            build_issue_status_event(
                &keys,
                &repo_address,
                &"d".repeat(64),
                &author,
                "resolved",
                Timestamp::now().as_secs(),
            )
            .unwrap(),
        )
        .unwrap();

        assert_eq!(event.pubkey, keys.public_key());
        assert_eq!(event.kind.as_u16(), 1631);
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["a", repo_address.as_str()]));
        assert!(event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["p", author.as_str()]));
        assert!(!event
            .tags
            .iter()
            .any(|tag| tag.as_slice() == ["p", owner.as_str()]));
        assert!(event.verify().is_ok());
    }

    #[test]
    fn issue_status_maps_each_lifecycle_state_to_its_kind() {
        let keys = Keys::generate();
        let owner = keys.public_key().to_hex();
        let repo_address = format!("30617:{owner}:buzz");
        for (status, kind) in [
            ("open", 1630u16),
            ("resolved", 1631),
            ("closed", 1632),
            ("draft", 1633),
        ] {
            let event = Event::from_json(
                build_issue_status_event(
                    &keys,
                    &repo_address,
                    &"d".repeat(64),
                    &"b".repeat(64),
                    status,
                    Timestamp::now().as_secs(),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(event.kind.as_u16(), kind, "status {status}");
        }
    }

    #[test]
    fn issue_status_rejects_unknown_state_and_foreign_repo() {
        let keys = Keys::generate();
        let owner = keys.public_key().to_hex();

        assert!(build_issue_status_event(
            &keys,
            &format!("30617:{owner}:buzz"),
            &"d".repeat(64),
            &"b".repeat(64),
            "in-progress",
            Timestamp::now().as_secs(),
        )
        .is_err());
        assert!(build_issue_status_event(
            &keys,
            &format!("30617:{}:buzz", "a".repeat(64)),
            &"d".repeat(64),
            &"b".repeat(64),
            "closed",
            Timestamp::now().as_secs(),
        )
        .is_err());
    }
}
