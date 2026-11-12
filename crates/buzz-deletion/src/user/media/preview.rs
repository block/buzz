//! A fresh, non-persisting observation for workflow previews.
use super::*;
use buzz_db::Db;
use serde_json::Value;

/// Observe and classify media without writing source data, a journal, fences,
/// reservations, or workflow progress. The caller prints the returned inventory
/// and treats `decision_complete=false` as unresolved, never as authorization.
/// Execution must independently discover and protect its own inventory.
pub async fn preview(db: &Db, media: &MediaStorage, options: &StageOptions) -> Result<Value> {
    if options.mode != Mode::PrintOnly {
        bail!("media preview requires print-only mode");
    }
    let mut scan = stage::scan(db, options).await?;
    storage::supplement(&mut scan, media, &options.storage_namespace).await?;
    let mut files = Vec::with_capacity(scan.files.len());
    for file in scan.files {
        files.push(match decision::decide(media, file.clone()).await {
            Ok(decided) => decided,
            Err(error) => {
                let mut pending = file;
                pending.last_error = Some(format!("{error:#}"));
                pending
            }
        });
    }
    let unknown = files.iter().filter(|file| file.status == "unknown").count();
    Ok(json!({
        "mode": "print-only", "execution_authorized": false,
        "discovery_complete": true, "decision_complete": unknown == 0,
        "candidate_count": files.len(), "unknown_count": unknown,
        "discovery": {"tables": scan.scanned, "scope": scan.scope,
            "unresolved": scan.unresolved, "database_snapshot_at": scan.database_snapshot_at},
        "files": files,
    }))
}
