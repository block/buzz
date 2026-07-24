//! Human-friendly terminal table rendering for the global `--format table`
//! flag.
//!
//! List commands already build a stable "compact" projection — a flat array of
//! JSON objects with the human-scannable subset of fields. `--format table`
//! reuses that exact projection and renders it as a bordered terminal table so
//! the column set stays in lockstep with `--format compact` for free.

use comfy_table::{presets::UTF8_FULL, ContentArrangement, Table};

/// Render a slice of flat JSON objects as a terminal table.
///
/// Columns are the union of the objects' keys in first-seen order (each list
/// command feeds a deterministic compact projection, so header order is
/// stable). Missing keys render as empty cells. An empty slice renders as
/// `(no results)` so a table request never prints a bare, confusing blank
/// line. A non-object payload has no columns to project and falls back to
/// compact JSON so nothing is silently dropped.
pub fn render_rows(rows: &[serde_json::Value]) -> String {
    if rows.is_empty() {
        return "(no results)".to_string();
    }

    let mut headers: Vec<String> = Vec::new();
    for row in rows {
        if let Some(obj) = row.as_object() {
            for key in obj.keys() {
                if !headers.iter().any(|h| h == key) {
                    headers.push(key.clone());
                }
            }
        }
    }

    if headers.is_empty() {
        return serde_json::to_string(rows).unwrap_or_default();
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers.clone());

    for row in rows {
        let cells: Vec<String> = headers
            .iter()
            .map(|h| row.get(h).map(cell_value).unwrap_or_default())
            .collect();
        table.add_row(cells);
    }

    table.to_string()
}

/// Render a single JSON value as a table cell. Strings drop their surrounding
/// quotes; nulls become an empty cell; everything else uses its compact JSON
/// form (numbers, bools, and any nested array/object).
fn cell_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::render_rows;
    use serde_json::json;

    #[test]
    fn empty_rows_render_no_results_placeholder() {
        assert_eq!(render_rows(&[]), "(no results)");
    }

    #[test]
    fn headers_follow_first_seen_key_order() {
        let rows = vec![json!({ "channel_id": "abc", "name": "general" })];
        let out = render_rows(&rows);
        let header_line = out
            .lines()
            .find(|l| l.contains("channel_id"))
            .expect("header row present");
        let id_pos = header_line.find("channel_id").expect("channel_id header");
        let name_pos = header_line.find("name").expect("name header");
        assert!(id_pos < name_pos, "channel_id must precede name: {out}");
    }

    #[test]
    fn values_and_headers_appear_in_output() {
        let rows = vec![
            json!({ "id": "e1", "content": "hello" }),
            json!({ "id": "e2", "content": "world" }),
        ];
        let out = render_rows(&rows);
        for needle in ["id", "content", "e1", "hello", "e2", "world"] {
            assert!(out.contains(needle), "missing {needle:?} in:\n{out}");
        }
    }

    #[test]
    fn strings_render_without_surrounding_quotes() {
        let rows = vec![json!({ "name": "general" })];
        let out = render_rows(&rows);
        assert!(out.contains("general"));
        assert!(!out.contains("\"general\""), "quotes leaked: {out}");
    }

    #[test]
    fn missing_key_becomes_empty_cell_not_a_panic() {
        // Second row omits `content` — the union header set still has it, and
        // the cell must be blank rather than dropping the column or panicking.
        let rows = vec![
            json!({ "id": "e1", "content": "hi" }),
            json!({ "id": "e2" }),
        ];
        let out = render_rows(&rows);
        assert!(out.contains("content"));
        assert!(out.contains("e2"));
    }

    #[test]
    fn non_string_scalars_render_via_json_form() {
        let rows = vec![json!({ "created_at": 1234, "pinned": true })];
        let out = render_rows(&rows);
        assert!(out.contains("1234"), "number missing: {out}");
        assert!(out.contains("true"), "bool missing: {out}");
    }
}
