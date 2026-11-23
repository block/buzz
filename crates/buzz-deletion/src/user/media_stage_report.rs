//! Capture the media handler's JSON for a persisted read-only stage.
use super::{run_handler, Database, Storage};
use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;
use std::io::{self, Write};

// Bound the additional allocation for a report that must fit in one journal row.
// Standalone and stateless preview commands can stream larger reports.
const MAX_REPORT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Default)]
struct StageReport(Vec<u8>);

impl Write for StageReport {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_REPORT_BYTES - self.0.len() {
            return Err(io::Error::other(
                "read-only media report exceeds 16 MiB; use media-batches --read-only to stream the full preview",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Deserialize)]
struct Details {
    files: Vec<Value>,
    target_pubkeys: Vec<String>,
}

pub(super) async fn stage_preview(
    db: &impl Database,
    media: &impl Storage,
    subject: &str,
) -> Result<Value> {
    let mut report = StageReport::default();
    let mut result = run_handler(db, media, subject, false, &mut report).await?;
    // Discovery has released its inventory before we decode the report.
    let details: Details = serde_json::from_slice(&report.0)?;
    result["files"] = Value::Array(details.files);
    result["target_pubkeys"] = details.target_pubkeys.into();
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_report_fails_without_appending_a_partial_write() {
        let mut report = StageReport(vec![0; MAX_REPORT_BYTES - 1]);
        report.write_all(b"a").unwrap();
        assert!(report.write_all(b"bc").is_err());
        assert_eq!(report.0.len(), MAX_REPORT_BYTES);
    }
}
