//! All user restriction installation and draining belongs to this first stage.
use super::{ExecutionMode, HandlerContext};
use anyhow::{ensure, Result};
use buzz_db::media_deletion::SubjectScope;
use serde_json::{json, Value};

pub(super) fn proposal(subject: &str) -> Value {
    json!({"record":"user_gate_preview","read_only":true,"subject":subject,
        "community":null,"offline_required_for_delete":true,
        "actions":["install user restriction", "verify all serving work has drained"],
        "online_write_fencing_complete":false})
}

pub(super) async fn run(context: &HandlerContext<'_>) -> Result<Value> {
    ensure!(
        context.row.target_community_id.is_none(),
        "user gate must be request-wide"
    );
    let result = match context.mode {
        ExecutionMode::ReadOnly => proposal(context.subject),
        ExecutionMode::Delete => {
            ensure!(
                context.args.offline,
                "user gate requires relay and external workers stopped"
            );
            context
                .db
                .user_deletion_store()
                .gate_user(
                    context.lease,
                    context.row.id,
                    &SubjectScope {
                        human: context.subject.into(),
                        agents: Default::default(),
                    },
                )
                .await?
        }
    };
    crate::print_json(&result)?;
    Ok(result)
}
