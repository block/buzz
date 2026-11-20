//! Stateless inspection: no lease, stage write, child submission or completion claim.
use super::{rehearsal::preview_handler, worker::RunOptions};
use anyhow::{ensure, Result};
use buzz_db::{entity_deletion::EntityRequest, user_deletion::UserDeletionInventory, Db};
use uuid::Uuid;

pub(super) async fn run(
    db: &Db,
    request: &EntityRequest,
    options: &RunOptions,
    selected: Option<&str>,
    community: Option<Uuid>,
) -> Result<i32> {
    let inventory = UserDeletionInventory::read(request)?;
    if request.target.starts_with("demo:") {
        crate::print_json(
            &serde_json::json!({"read_only":true,"inventory":request.inventory_manifest}),
        )?;
        return Ok(0);
    }
    if let Some(id) = community {
        ensure!(
            inventory.communities.iter().any(|(key, _)| *key == id),
            "community outside inventory"
        );
        ensure!(
            selected.is_some_and(|s| super::workflow::rule(s).is_ok_and(|r| r.community_scope)),
            "community requires a community-scoped stage"
        );
    }
    let source = options.db.as_ref().unwrap_or(db);
    let current = if options.next_step && selected.is_none() {
        db.entity_deletion_store()
            .inspect(request.id)
            .await?
            .stage_history
            .into_iter()
            .find(|s| s.is_current)
    } else {
        None
    };
    let selected = selected
        .or_else(|| current.as_ref().map(|s| s.stage.as_str()))
        .or(if options.next_step {
            Some("gate_user")
        } else {
            None
        });
    let community = community.or_else(|| current.as_ref().and_then(|s| s.target_community_id));
    for stage in ["gate_user", "purge_media"] {
        if selected.is_none_or(|s| s == stage) {
            crate::print_json(&preview_handler(source, request, stage, None, options).await?)?;
        }
    }
    for (id, attrs) in &inventory.communities {
        if community.is_some_and(|c| c != *id) {
            continue;
        }
        let stages: &[&str] = if attrs["relationship"] == "owner" {
            &["delete_owned_community"]
        } else {
            &["purge_content", "purge_account", "purge_profile"]
        };
        if community == Some(*id) && selected.is_some_and(|s| !stages.contains(&s)) {
            anyhow::bail!(
                "selected stage does not apply to this community's approved relationship"
            );
        }
        for stage in stages {
            if selected.is_none_or(|s| s == *stage) {
                crate::print_json(
                    &preview_handler(source, request, stage, Some(*id), options).await?,
                )?;
            }
        }
    }
    if selected.is_none_or(|s| s == "global_cleanup") {
        crate::print_json(
            &preview_handler(source, request, "global_cleanup", None, options).await?,
        )?;
    }
    ensure!(
        selected != Some("purge_agents"),
        "agents are included in content/account/profile previews"
    );
    crate::print_json(
        &serde_json::json!({"record":"stateless_preview_complete","workflow_advanced":false,
        "account_erasure_complete":false,"child_submitted":false}),
    )?;
    Ok(0)
}
