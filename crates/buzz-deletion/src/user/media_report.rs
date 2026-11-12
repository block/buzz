//! One JSON document, streamed from bounded inventory pages even for large files.
use super::{Database, Inventory, Storage};
use anyhow::Result;
use serde_json::{json, Value};
use std::io::Write;

pub(super) async fn execute(
    db: &impl Database,
    media: &impl Storage,
    pubkeys: &[String],
    delete: bool,
    inventory: &mut Inventory,
    output: &mut impl Write,
) -> Result<Value> {
    let mode = if delete { "delete" } else { "read-only" };
    write!(
        output,
        "{{\n  \"mode\": \"{mode}\",\n  \"target_pubkeys\": "
    )?;
    serde_json::to_writer(&mut *output, pubkeys)?;
    write!(output, ",\n  \"files\": [")?;
    let (mut selected, mut kept, mut deleted_versions) = (0usize, 0usize, 0usize);
    let mut after = String::new();
    loop {
        let page = inventory.candidates(&after).await?;
        if page.is_empty() {
            break;
        }
        for (hash, shared) in page {
            if selected > 0 {
                write!(output, ",")?;
            }
            selected += 1;
            let fields = json!({
                "filename": inventory.filename(&hash).await?,
                "original_filename": null,
                "file_hash": hash,
                "decision": if shared { "keep" } else { "delete" },
                "reason": if shared { "Uploaded by someone outside the target set" }
                    else { "All known uploaders belong to the target set" },
                "has_other_uploader": shared,
            });
            // Serialize scalar fields normally, then stream potentially large
            // owner/version arrays without collecting them into a Value.
            let fields = serde_json::to_string_pretty(&fields)?;
            write!(output, "\n{}", fields.trim_end_matches('}').trim_end())?;
            write!(output, ",\n  \"owners\": [")?;
            let mut owner_cursor = String::new();
            let mut first_owner = true;
            loop {
                let owners = inventory.owners(&hash, &owner_cursor).await?;
                if owners.is_empty() {
                    break;
                }
                for owner in owners {
                    if !first_owner {
                        write!(output, ", ")?;
                    }
                    serde_json::to_writer(&mut *output, &owner)?;
                    first_owner = false;
                    owner_cursor = owner;
                }
            }
            write!(output, "],\n  \"versions\": [")?;
            if shared {
                kept += 1;
            }
            let mut handled = 0usize;
            let mut cursor = (-1, 0);
            let mut first_version = true;
            loop {
                let versions = inventory.versions(&hash, cursor).await?;
                if versions.is_empty() {
                    break;
                }
                for (priority, id, version) in versions {
                    if !first_version {
                        write!(output, ",")?;
                    }
                    write!(output, "\n    ")?;
                    serde_json::to_writer(&mut *output, &version)?;
                    first_version = false;
                    if delete && !shared {
                        media.delete(&version).await?;
                        handled += 1;
                    }
                    cursor = (priority, id);
                }
            }
            let removed = if delete {
                db.remove(pubkeys, &hash).await?
            } else {
                0
            };
            write!(output, "\n  ],\n  \"deleted_versions\": {handled},\n  \"removed_uploader_rows\": {removed}\n}}")?;
            deleted_versions += handled;
            after = hash;
        }
    }
    let summary = json!({"mode": mode, "selected_files": selected,
        "kept_files": kept, "deleted_versions": deleted_versions,
        "account_erasure_complete": false});
    write!(output, "\n  ],\n  \"summary\": ")?;
    serde_json::to_writer_pretty(&mut *output, &summary)?;
    writeln!(output, "\n}}")?;
    output.flush()?;
    Ok(summary)
}
