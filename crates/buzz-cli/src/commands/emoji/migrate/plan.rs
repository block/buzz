//! The plan file and the dry-run arithmetic.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::names::PlanItem;

/// Plan file format version.
pub const PLAN_VERSION: u32 = 1;

/// What to do with a name that already has a kind 30180 entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum OnConflict {
    Skip,
    Overwrite,
}

/// `plan.json`: the exact work `run` will do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub version: u32,
    /// SHA-256 of the input (the file, or the directory listing).
    pub input_sha256: String,
    pub on_conflict: OnConflict,
    pub items: Vec<PlanItem>,
}

/// SHA-256 of `bytes` as lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

impl Plan {
    /// Hash of the plan's canonical bytes. The journal header pins it.
    pub fn sha256(&self) -> Result<String, serde_json::Error> {
        Ok(sha256_hex(&serde_json::to_vec(self)?))
    }
}

/// Run time under the rate flags, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Estimate {
    pub upload_secs: f64,
    pub publish_secs: f64,
    /// Uploads and publishes overlap, so the run takes about the longer one.
    pub total_secs: f64,
}

/// Estimate the run time for `unique_blobs` uploads and `names` publishes.
pub fn estimate(
    unique_blobs: u64,
    names: u64,
    uploads_per_minute: u32,
    events_per_minute: u32,
) -> Estimate {
    let upload_secs = unique_blobs as f64 * 60.0 / f64::from(uploads_per_minute.max(1));
    let publish_secs = names as f64 * 60.0 / f64::from(events_per_minute.max(1));
    Estimate {
        upload_secs,
        publish_secs,
        total_secs: upload_secs.max(publish_secs),
    }
}

/// `1 - unique/total`, or 0 when there is nothing to migrate.
pub fn duplicate_ratio(unique: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        1.0 - unique as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_matches_the_design_numbers() {
        let e = estimate(72_000, 96_000, 30, 240);
        assert_eq!(e.upload_secs / 3600.0, 40.0);
        assert_eq!(e.publish_secs / 3600.0, 6.666666666666667);
        assert_eq!(e.total_secs, e.upload_secs);
        let raised = estimate(72_000, 96_000, 600, 3000);
        assert_eq!(raised.upload_secs, 7200.0);
        assert_eq!(raised.publish_secs, 1920.0);
    }

    #[test]
    fn duplicate_ratio_handles_empty_and_shared_blobs() {
        assert_eq!(duplicate_ratio(0, 0), 0.0);
        assert_eq!(duplicate_ratio(75, 100), 0.25);
        assert_eq!(duplicate_ratio(100, 100), 0.0);
    }

    #[test]
    fn plan_hash_changes_with_any_item_or_policy() {
        let item = |n: &str| PlanItem {
            name: n.into(),
            source: "https://x/a.png".into(),
            alias_of: None,
        };
        let base = Plan {
            version: PLAN_VERSION,
            input_sha256: "in".into(),
            on_conflict: OnConflict::Skip,
            items: vec![item("a")],
        };
        let h = base.sha256().unwrap();
        assert_eq!(h, base.clone().sha256().unwrap());
        let mut other = base.clone();
        other.items.push(item("b"));
        assert_ne!(h, other.sha256().unwrap());
        let mut other = base.clone();
        other.on_conflict = OnConflict::Overwrite;
        assert_ne!(h, other.sha256().unwrap());
        let mut other = base;
        other.input_sha256 = "changed".into();
        assert_ne!(h, other.sha256().unwrap());
    }
}
