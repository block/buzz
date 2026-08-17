//! Authenticated download support for non-image files attached to Buzz messages.

use crate::shell::SharedState;
use crate::view_image::fetch_relay_attachment;
use rmcp::{model::CallToolResult, model::Content, ErrorData};
use schemars::JsonSchema;
use serde::Deserialize;
use std::path::{Path, PathBuf};

const MAX_FILENAME_BYTES: usize = 180;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DownloadAttachmentParams {
    /// Buzz relay `/media/` URL from the message attachment link.
    pub url: String,
    /// Original attachment filename shown in the message. When omitted, the
    /// final URL path segment is used.
    #[serde(default)]
    pub filename: Option<String>,
}

pub async fn run(
    state: &SharedState,
    p: DownloadAttachmentParams,
) -> Result<CallToolResult, ErrorData> {
    let bytes = fetch_relay_attachment(p.url.trim()).await?;
    let filename = attachment_filename(&p)?;
    let directory = state.session_dir.path().join("attachments");
    std::fs::create_dir_all(&directory).map_err(|e| {
        ErrorData::internal_error(
            format!(
                "cannot create attachment directory {}: {e}",
                directory.display()
            ),
            None,
        )
    })?;
    let destination = unique_destination(&directory, &filename);
    let mut temporary = tempfile::NamedTempFile::new_in(&directory).map_err(|e| {
        ErrorData::internal_error(
            format!("cannot create temporary attachment file: {e}"),
            None,
        )
    })?;
    use std::io::Write;
    temporary.write_all(&bytes).map_err(|e| {
        ErrorData::internal_error(format!("cannot write downloaded attachment: {e}"), None)
    })?;
    temporary.persist(&destination).map_err(|e| {
        ErrorData::internal_error(
            format!(
                "cannot persist downloaded attachment {}: {}",
                destination.display(),
                e.error
            ),
            None,
        )
    })?;

    Ok(CallToolResult::success(vec![Content::text(format!(
        "Downloaded Buzz attachment ({} bytes) to {}",
        bytes.len(),
        destination.display()
    ))]))
}

fn attachment_filename(p: &DownloadAttachmentParams) -> Result<String, ErrorData> {
    let candidate = p
        .filename
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    let from_url;
    let raw = match candidate {
        Some(value) => value,
        None => {
            let parsed = reqwest::Url::parse(p.url.trim()).map_err(|e| {
                ErrorData::invalid_params(format!("invalid attachment URL: {} ({e})", p.url), None)
            })?;
            from_url = parsed
                .path_segments()
                .and_then(|mut segments| segments.next_back())
                .unwrap_or("attachment.bin")
                .to_string();
            &from_url
        }
    };
    Ok(sanitize_filename(raw))
}

fn sanitize_filename(value: &str) -> String {
    let basename = Path::new(value.trim())
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment.bin");
    let mut clean = String::with_capacity(basename.len().min(MAX_FILENAME_BYTES));
    for ch in basename.chars() {
        let replacement = if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            ch
        } else {
            '_'
        };
        if clean.len() + replacement.len_utf8() > MAX_FILENAME_BYTES {
            break;
        }
        clean.push(replacement);
    }
    let clean = clean.trim_matches('.');
    if clean.is_empty() {
        "attachment.bin".to_string()
    } else {
        clean.to_string()
    }
}

fn unique_destination(directory: &Path, filename: &str) -> PathBuf {
    let initial = directory.join(filename);
    if !initial.exists() {
        return initial;
    }
    let path = Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("attachment");
    let extension = path.extension().and_then(|value| value.to_str());
    for suffix in 1..=9999 {
        let candidate = match extension {
            Some(extension) => directory.join(format!("{stem}-{suffix}.{extension}")),
            None => directory.join(format!("{stem}-{suffix}")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    directory.join(format!("attachment-{}.bin", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_is_reduced_to_a_safe_basename() {
        assert_eq!(
            sanitize_filename(r"..\papers/cysteine mediated?.pdf"),
            "cysteine_mediated_.pdf"
        );
        assert_eq!(sanitize_filename("../.."), "attachment.bin");
    }

    #[test]
    fn duplicate_names_get_a_stable_suffix() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("paper.pdf"), b"one").unwrap();
        assert_eq!(
            unique_destination(directory.path(), "paper.pdf"),
            directory.path().join("paper-1.pdf")
        );
    }
}
