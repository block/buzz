//! Shared discovery and classification for all media entry points.
use super::*;
use buzz_db::{
    media_deletion::{MediaScan, SubjectScope},
    Db,
};

pub(super) enum Journal<'a> {
    None,
    Preview(&'a MediaDeletionStore, &'a MediaRun),
    Execution(&'a MediaDeletionStore, &'a MediaRun, &'a SubjectScope),
}

/// Resume collected evidence or collect it once, then classify that inventory.
/// Only journal/checkpoint operations depend on execution mode.
pub(super) async fn prepare(
    db: Option<&Db>,
    media: &MediaStorage,
    options: &StageOptions,
    journal: Journal<'_>,
) -> Result<MediaScan> {
    let saved = match &journal {
        Journal::None => None,
        Journal::Preview(store, run) | Journal::Execution(store, run, _) => store.load(run).await?,
    };
    let mut scan = if let Some(files) = saved {
        let (Journal::Preview(store, run) | Journal::Execution(store, run, _)) = &journal else {
            bail!("saved inventory requires a journal");
        };
        let receipt = store.receipt(run).await?;
        MediaScan {
            files,
            scope: serde_json::from_value(receipt["scans"]["scope"].clone())?,
            scanned: serde_json::from_value(receipt["scans"]["tables"].clone())?,
            unresolved: serde_json::from_value(receipt["scans"]["unresolved"].clone())?,
            database_snapshot_at: receipt["scans"]["database_snapshot_at"]
                .as_str()
                .context("missing discovery snapshot time")?
                .to_owned(),
        }
    } else {
        let mut scan = stage::scan(
            db.context("source connection required for fresh discovery")?,
            options,
        )
        .await?;
        storage::supplement(&mut scan, media, &options.storage_namespace).await?;
        match &journal {
            Journal::None => {}
            Journal::Preview(store, run) | Journal::Execution(store, run, _) => {
                store
                    .publish(
                        run,
                        &scan.files,
                        json!({
                            "tables":scan.scanned, "scope":scan.scope,
                            "unresolved":scan.unresolved,
                            "database_snapshot_at":scan.database_snapshot_at,
                        }),
                    )
                    .await?;
            }
        }
        scan
    };
    if let Journal::Execution(store, run, expected_scope) = &journal {
        if serde_json::to_value(&scan.scope)? != serde_json::to_value(expected_scope)? {
            bail!("saved subject scope changed");
        }
        store.validate_removal_scope(run).await?;
    }
    for file in scan.files.iter_mut().filter(|f| f.status == "unknown") {
        let decided = match decision::decide(media, file.clone()).await {
            Ok(decided) => decided,
            Err(error) => {
                let mut pending = file.clone();
                pending.last_error = Some(format!("{error:#}"));
                pending
            }
        };
        match &journal {
            Journal::None => {}
            Journal::Preview(store, run) => store.save_preview_decision(run, &decided).await?,
            Journal::Execution(store, run, _) => {
                store.save_execution_decision(run, &decided).await?
            }
        }
        *file = decided;
    }
    Ok(scan)
}
