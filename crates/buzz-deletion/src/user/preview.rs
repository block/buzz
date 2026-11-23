//! Stateless inspection: no lease, stage write, child submission or completion claim.
use super::{
    handlers::Handler,
    worker::RunOptions,
    workflow::{rule, Progress, Step, INITIAL_STAGE},
};
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
    let handler = Handler {
        db: source,
        request,
        options,
        mode: buzz_db::user_deletion::ExecutionMode::ReadOnly,
        current: None,
    };
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
            Some(INITIAL_STAGE)
        } else {
            None
        });
    let community = community.or_else(|| current.as_ref().and_then(|s| s.target_community_id));
    ensure!(
        selected != Some("purge_agents"),
        "agents are included in content/account/profile previews"
    );
    let mut progress = Progress::new(false);
    let mut step = Some(Step {
        name: INITIAL_STAGE,
        community: None,
    });
    let mut matched = false;
    while let Some(current) = step {
        // The terminal report reads persisted results. Stateless previews have none.
        if current.name == "remaining_cleanup" {
            break;
        }
        if selected.is_none_or(|s| s == current.name)
            && community.is_none_or(|c| Some(c) == current.community)
        {
            crate::print_json(&handler.run(current.name, current.community).await?)?;
            matched = true;
        }
        step = progress.apply(&inventory, current.community, rule(current.name)?.next)?;
    }
    ensure!(
        community.is_none() || matched,
        "selected stage does not apply to this community's approved relationship"
    );
    crate::print_json(
        &serde_json::json!({"record":"stateless_preview_complete","workflow_advanced":false,
        "account_erasure_complete":false,"child_submitted":false}),
    )?;
    Ok(0)
}
