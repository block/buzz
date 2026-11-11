//! CLI ownership of one approved media stage; never advances the account workflow.
use super::*;
use buzz_db::{entity_deletion::EntityLease, Db};

pub(super) async fn run(
    db: &Db,
    media: &MediaStorage,
    request_id: Uuid,
    options: StageOptions,
) -> Result<i32> {
    let store = db.entity_deletion_store();
    let request = store.get(request_id).await?;
    if request.entity_type != "user" || request.target != options.human {
        bail!("--request-id must identify an account request for --pubkey");
    }
    let lease = store
        .claim(
            request_id,
            &crate::default_executor_id(),
            crate::DEFAULT_LEASE_DURATION,
        )
        .await?
        .context("request is unapproved, not due for retry, or already leased")?;
    let shutdown = crate::shutdown_token();
    let renewal = async {
        let mut interval = tokio::time::interval(crate::HEARTBEAT_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = store.heartbeat(&lease, crate::DEFAULT_LEASE_DURATION).await {
                return error;
            }
        }
    };
    let result = tokio::select! {
        biased;
        _ = shutdown.cancelled() => Err(anyhow::anyhow!("media deletion interrupted; retry the same --request-id")),
        result = execute(db, media, &lease, options) => result,
        error = renewal => Err(error.into()),
    };
    // No detached heartbeat remains. Failed runs retain their saved targets and
    // progress; the next invocation claims a new generation and resumes.
    let released = store.release(&lease).await;
    match (result, released) {
        (Err(error), Err(release)) => {
            let message = format!("{error:#}; also failed to release media lease: {release}");
            Err(error.context(message))
        }
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(error.into()),
        (Ok(()), Ok(_)) => Ok(0),
    }
}

async fn execute(
    db: &Db,
    media: &MediaStorage,
    lease: &EntityLease,
    options: StageOptions,
) -> Result<()> {
    let store = db.entity_deletion_store();
    let inspection = store.inspect(lease.request_id).await?;
    let stage = inspection
        .stage_history
        .iter()
        .find(|row| row.is_current)
        .context("approved request has no current stage; expected purge_media")?;
    if stage.stage != "purge_media" {
        bail!(
            "current request stage is {}; expected purge_media",
            stage.stage
        );
    }
    if stage.status == "completed" {
        if stage.metadata_json["binding"] != options.binding()?
            || stage.metadata_json["result"]["media_deleted"] != true
        {
            bail!("completed media stage does not match this execution");
        }
        crate::print_json(&json!({"stage_id":stage.id,"mode":"delete","complete":true}))?;
        return Ok(());
    }
    store.start_stage(lease, stage.id).await?;
    run_stage(db, media, lease, stage.id, options).await?;
    store
        .save_stage_success(lease, stage.id, json!({"media_deleted":true}))
        .await?;
    Ok(())
}
