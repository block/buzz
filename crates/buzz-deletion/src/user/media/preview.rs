//! A fresh, non-persisting observation for workflow previews.
use super::*;
use buzz_db::Db;
use serde_json::Value;

/// Observe and classify media without writing source data, a journal or
/// workflow progress. The caller prints the returned inventory
/// and treats `decision_complete=false` as unresolved, never as authorization.
/// Execution must independently discover and journal its own inventory.
pub async fn preview(db: &Db, media: &MediaStorage, options: &StageOptions) -> Result<Value> {
    if options.mode != Mode::PrintOnly {
        bail!("media preview requires print-only mode");
    }
    let scan = pipeline::prepare(Some(db), media, options, pipeline::Journal::None).await?;
    let files = scan.files;
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
