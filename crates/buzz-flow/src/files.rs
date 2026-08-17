//! Flow Studio file metadata (content stored via Buzz media / Blossom).

use serde::{Deserialize, Serialize};

/// File metadata projected from kind 46350–46399 events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlowFileRecord {
    /// File identifier.
    pub file_id: String,
    /// Original filename.
    pub filename: String,
    /// Blossom media URL when uploaded.
    pub media_url: Option<String>,
    /// Monotonic version counter.
    pub version: u32,
    /// Whether the file was soft-deleted.
    pub deleted: bool,
}

/// Bump version on upload; mark deleted on delete event.
pub fn apply_file_event(
    files: &mut Vec<FlowFileRecord>,
    file_id: &str,
    filename: &str,
    media_url: Option<String>,
    deleted: bool,
) {
    if deleted {
        if let Some(existing) = files.iter_mut().find(|f| f.file_id == file_id) {
            existing.deleted = true;
        }
        return;
    }

    if let Some(existing) = files.iter_mut().find(|f| f.file_id == file_id) {
        existing.filename = filename.to_string();
        existing.media_url = media_url;
        existing.version += 1;
        existing.deleted = false;
    } else {
        files.push(FlowFileRecord {
            file_id: file_id.to_string(),
            filename: filename.to_string(),
            media_url,
            version: 1,
            deleted: false,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_then_version() {
        let mut files = Vec::new();
        apply_file_event(
            &mut files,
            "f1",
            "doc.pdf",
            Some("https://relay/media/x".into()),
            false,
        );
        apply_file_event(
            &mut files,
            "f1",
            "doc-v2.pdf",
            Some("https://relay/media/y".into()),
            false,
        );
        assert_eq!(files[0].version, 2);
    }
}
