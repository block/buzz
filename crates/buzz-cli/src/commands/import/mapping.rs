//! Explicit Slack-conversation → existing Buzz-channel crosswalk parsing.
//!
//! Importers often run after an operator has already mirrored a workspace's
//! channel shell. The crosswalk lets history adopt those channels instead of
//! creating duplicates. CSV matches the export/audit artifact shape used by
//! migration tooling; JSON is useful for generated pipelines.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde::Deserialize;
use uuid::Uuid;

use crate::error::CliError;

#[derive(Deserialize)]
struct ChannelMapEntry {
    slack_channel_id: String,
    buzz_channel_id: String,
}

/// Load a channel crosswalk from CSV or a JSON array.
///
/// CSV must contain `slack_channel_id` and `buzz_channel_id` columns (extra
/// columns are ignored). JSON uses an array of objects with those same fields.
pub(super) fn load_channel_map(path: &Path) -> Result<HashMap<String, String>, CliError> {
    let raw = std::fs::read_to_string(path).map_err(|error| {
        CliError::Other(format!(
            "cannot read channel map {}: {error}",
            path.display()
        ))
    })?;
    let entries = if raw.trim_start().starts_with('[') {
        serde_json::from_str::<Vec<ChannelMapEntry>>(&raw).map_err(|error| {
            CliError::Usage(format!(
                "channel map {} is invalid JSON: {error}",
                path.display()
            ))
        })?
    } else {
        parse_csv_entries(&raw, path)?
    };

    let mut result = HashMap::new();
    let mut buzz_ids = HashSet::new();
    for entry in entries {
        let slack_id = entry.slack_channel_id.trim();
        if slack_id.is_empty() {
            return Err(CliError::Usage(format!(
                "channel map {} contains an empty Slack channel id",
                path.display()
            )));
        }
        let buzz_id = Uuid::parse_str(entry.buzz_channel_id.trim())
            .map_err(|error| {
                CliError::Usage(format!(
                    "channel map {} has invalid Buzz UUID for {slack_id}: {error}",
                    path.display()
                ))
            })?
            .to_string();
        if let Some(previous) = result.get(slack_id) {
            let detail = if previous == &buzz_id {
                "appears more than once"
            } else {
                "maps to two Buzz UUIDs"
            };
            return Err(CliError::Usage(format!(
                "channel map {} Slack channel {slack_id} {detail}",
                path.display()
            )));
        }
        if !buzz_ids.insert(buzz_id.clone()) {
            return Err(CliError::Usage(format!(
                "channel map {} assigns Buzz UUID {buzz_id} more than once",
                path.display()
            )));
        }
        result.insert(slack_id.to_string(), buzz_id);
    }
    if result.is_empty() {
        return Err(CliError::Usage(format!(
            "channel map {} contains no mappings",
            path.display()
        )));
    }
    Ok(result)
}

fn parse_csv_entries(raw: &str, path: &Path) -> Result<Vec<ChannelMapEntry>, CliError> {
    let records = parse_csv_records(raw).map_err(|error| {
        CliError::Usage(format!(
            "channel map {} is invalid CSV: {error}",
            path.display()
        ))
    })?;
    let (header, rows) = records
        .split_first()
        .ok_or_else(|| CliError::Usage(format!("channel map {} is empty", path.display())))?;
    let slack_index = header
        .iter()
        .position(|field| field == "slack_channel_id")
        .ok_or_else(|| {
            CliError::Usage(format!(
                "channel map {} is missing slack_channel_id",
                path.display()
            ))
        })?;
    let buzz_index = header
        .iter()
        .position(|field| field == "buzz_channel_id")
        .ok_or_else(|| {
            CliError::Usage(format!(
                "channel map {} is missing buzz_channel_id",
                path.display()
            ))
        })?;
    let expected_columns = header.len();
    rows.iter()
        .enumerate()
        .map(|(index, row)| {
            if row.len() != expected_columns {
                return Err(CliError::Usage(format!(
                    "channel map {} CSV row {} has {} columns; expected {}",
                    path.display(),
                    index + 2,
                    row.len(),
                    expected_columns
                )));
            }
            Ok(ChannelMapEntry {
                slack_channel_id: row[slack_index].clone(),
                buzz_channel_id: row[buzz_index].clone(),
            })
        })
        .collect()
}

/// Small RFC 4180 record parser: handles quoted commas, doubled quotes, CRLF,
/// and embedded newlines without adding another runtime dependency.
fn parse_csv_records(raw: &str) -> Result<Vec<Vec<String>>, &'static str> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut chars = raw.chars().peekable();
    let mut quoted = false;

    while let Some(ch) = chars.next() {
        if quoted {
            if ch == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    field.push('"');
                } else {
                    quoted = false;
                }
            } else {
                field.push(ch);
            }
            continue;
        }

        match ch {
            '"' if field.chars().all(char::is_whitespace) => {
                field.clear();
                quoted = true;
            }
            ',' => record.push(std::mem::take(&mut field)),
            '\n' | '\r' => {
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                record.push(std::mem::take(&mut field));
                if !record.iter().all(String::is_empty) {
                    records.push(std::mem::take(&mut record));
                } else {
                    record.clear();
                }
            }
            _ => field.push(ch),
        }
    }
    if quoted {
        return Err("unterminated quoted field");
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        if !record.iter().all(String::is_empty) {
            records.push(record);
        }
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_parser_handles_generated_crosswalk_shape() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let dir = std::env::temp_dir().join(format!("buzz-channel-map-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("map.csv");
        std::fs::write(
            &path,
            format!(
                "\"slack_channel_id\",\"name\",\"buzz_channel_id\"\r\n\
                 \"C1\",\"general\",\"{id_a}\"\r\n\
                 \"C2\",\"name, with comma\",\"{id_b}\"\r\n"
            ),
        )
        .expect("write map");
        let map = load_channel_map(&path).expect("load map");
        assert_eq!(map["C1"], id_a.to_string());
        assert_eq!(map["C2"], id_b.to_string());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_duplicate_target_uuid() {
        let id = Uuid::new_v4();
        let dir = std::env::temp_dir().join(format!("buzz-channel-map-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("map.json");
        std::fs::write(
            &path,
            serde_json::json!([
                {"slack_channel_id": "C1", "buzz_channel_id": id},
                {"slack_channel_id": "C2", "buzz_channel_id": id},
            ])
            .to_string(),
        )
        .expect("write map");
        assert!(load_channel_map(&path).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_duplicate_slack_id_before_target_reuse_diagnostics() {
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let dir = std::env::temp_dir().join(format!("buzz-channel-map-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("map.json");

        std::fs::write(
            &path,
            serde_json::json!([
                {"slack_channel_id": "C1", "buzz_channel_id": id_a},
                {"slack_channel_id": "C1", "buzz_channel_id": id_a},
            ])
            .to_string(),
        )
        .expect("write exact duplicate");
        let exact = load_channel_map(&path).expect_err("exact duplicate must fail");
        assert!(exact.to_string().contains("appears more than once"));

        std::fs::write(
            &path,
            serde_json::json!([
                {"slack_channel_id": "C1", "buzz_channel_id": id_a},
                {"slack_channel_id": "C1", "buzz_channel_id": id_b},
            ])
            .to_string(),
        )
        .expect("write conflicting duplicate");
        let conflicting = load_channel_map(&path).expect_err("conflicting duplicate must fail");
        assert!(conflicting.to_string().contains("maps to two Buzz UUIDs"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn csv_parser_accepts_leading_space_before_quotes_and_bare_cr_records() {
        let id = Uuid::new_v4();
        let dir = std::env::temp_dir().join(format!("buzz-channel-map-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("map.csv");
        std::fs::write(
            &path,
            format!(
                "  \"slack_channel_id\",  \"name\",  \"buzz_channel_id\"\r\
                   \"C1\",  \"name, with comma\",  \"{id}\"\r"
            ),
        )
        .expect("write map");

        let map = load_channel_map(&path).expect("load map");
        assert_eq!(map["C1"], id.to_string());
        std::fs::remove_dir_all(&dir).ok();
    }
}
