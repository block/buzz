//! Human-friendly terminal table rendering for the global `--format table`
//! flag.
//!
//! List commands already build a stable "compact" projection — a flat array of
//! JSON objects with the human-scannable subset of fields. `--format table`
//! reuses that exact projection and renders it as a bordered terminal table so
//! the column set stays in lockstep with `--format compact` for free.

use comfy_table::{presets::UTF8_FULL, ContentArrangement, Table};

/// Render `rows` as a terminal table using the explicit, ordered `headers`.
///
/// `headers` is passed in (not derived from the row objects) on purpose: the
/// workspace builds `serde_json` without the `preserve_order` feature, so
/// `Value` objects are `BTreeMap`-backed and iterating their keys would sort
/// alphabetically — turning a declared `id, content, created_at` projection
/// into `content, created_at, id`. The caller owns column order; each list
/// command passes the same field order its compact projection declares.
///
/// Missing keys render as empty cells. An empty `rows` slice renders as
/// `(no results)` so a table request never prints a bare, confusing blank
/// line.
pub fn render_rows(headers: &[&str], rows: &[serde_json::Value]) -> String {
    if rows.is_empty() {
        return "(no results)".to_string();
    }

    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers.iter().copied());

    for row in rows {
        let cells: Vec<String> = headers
            .iter()
            .map(|h| row.get(*h).map(cell_value).unwrap_or_default())
            .collect();
        table.add_row(cells);
    }

    table.to_string()
}

/// Render a single JSON value as a table cell. Strings drop their surrounding
/// quotes; nulls become an empty cell; everything else uses its compact JSON
/// form (numbers, bools, and any nested array/object). All cell text is passed
/// through [`sanitize`] first.
fn cell_value(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => sanitize(s),
        other => sanitize(&other.to_string()),
    }
}

/// Replace terminal control characters (ESC, BEL, CR, and the rest of the C0
/// set, DEL, and the C1 range) with spaces.
///
/// Table cells print raw to the terminal, so attacker-controlled relay content
/// — message bodies, feed items, display names, channel names — could
/// otherwise smuggle ANSI/OSC escape sequences that erase, recolor, or spoof
/// the user's terminal output. The JSON and compact paths are unaffected
/// because `serde_json` already escapes control characters as `\uXXXX`; only
/// the table path decodes them back to raw bytes, so only it needs this.
fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::render_rows;
    use serde_json::json;

    #[test]
    fn empty_rows_render_no_results_placeholder() {
        assert_eq!(render_rows(&["id"], &[]), "(no results)");
    }

    #[test]
    fn columns_follow_declared_header_order_not_alphabetical() {
        // `content` < `created_at` < `id` alphabetically, so a BTreeMap-backed
        // key iteration would reorder them. The explicit header order must win.
        let rows = vec![json!({ "id": "e1", "content": "hi", "created_at": 5 })];
        let out = render_rows(&["id", "content", "created_at"], &rows);
        let header_line = out
            .lines()
            .find(|l| l.contains("content"))
            .expect("header row present");
        let id_pos = header_line.find("id").expect("id header");
        let content_pos = header_line.find("content").expect("content header");
        let created_pos = header_line.find("created_at").expect("created_at header");
        assert!(
            id_pos < content_pos && content_pos < created_pos,
            "declared order id<content<created_at not preserved: {out}"
        );
    }

    #[test]
    fn values_and_headers_appear_in_output() {
        let rows = vec![
            json!({ "id": "e1", "content": "hello" }),
            json!({ "id": "e2", "content": "world" }),
        ];
        let out = render_rows(&["id", "content"], &rows);
        for needle in ["id", "content", "e1", "hello", "e2", "world"] {
            assert!(out.contains(needle), "missing {needle:?} in:\n{out}");
        }
    }

    #[test]
    fn strings_render_without_surrounding_quotes() {
        let rows = vec![json!({ "name": "general" })];
        let out = render_rows(&["name"], &rows);
        assert!(out.contains("general"));
        assert!(!out.contains("\"general\""), "quotes leaked: {out}");
    }

    #[test]
    fn missing_key_becomes_empty_cell_not_a_panic() {
        // Second row omits `content` — the column still exists, and the cell
        // must be blank rather than dropping the column or panicking.
        let rows = vec![
            json!({ "id": "e1", "content": "hi" }),
            json!({ "id": "e2" }),
        ];
        let out = render_rows(&["id", "content"], &rows);
        assert!(out.contains("content"));
        assert!(out.contains("e2"));
    }

    #[test]
    fn non_string_scalars_render_via_json_form() {
        let rows = vec![json!({ "created_at": 1234, "pinned": true })];
        let out = render_rows(&["created_at", "pinned"], &rows);
        assert!(out.contains("1234"), "number missing: {out}");
        assert!(out.contains("true"), "bool missing: {out}");
    }

    #[test]
    fn control_sequences_are_stripped_from_cells() {
        // A relay event whose content embeds an ANSI color escape + BEL must
        // not reach the terminal verbatim from the table path.
        let rows = vec![json!({ "content": "\u{1b}[31mhacked\u{7}\r" })];
        let out = render_rows(&["content"], &rows);
        assert!(!out.contains('\u{1b}'), "ESC leaked: {out:?}");
        assert!(!out.contains('\u{7}'), "BEL leaked: {out:?}");
        assert!(!out.contains('\r'), "CR leaked: {out:?}");
        assert!(out.contains("hacked"), "visible text lost: {out:?}");
    }
}
