//! Flow Studio table row helpers (event-backed CRUD).

use serde::{Deserialize, Serialize};

/// A table row in the Flow Studio read-model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableRow {
    /// Table identifier.
    pub table_id: String,
    /// Row identifier within the table.
    pub row_id: String,
    /// Row payload as JSON.
    pub row_json: serde_json::Value,
}

/// Apply a create/update/delete event to an in-memory row set (projector logic).
pub fn apply_row_event(
    rows: &mut Vec<TableRow>,
    table_id: &str,
    row_id: &str,
    row_json: Option<serde_json::Value>,
    deleted: bool,
) {
    rows.retain(|r| !(r.table_id == table_id && r.row_id == row_id));
    if !deleted {
        if let Some(json) = row_json {
            rows.push(TableRow {
                table_id: table_id.to_string(),
                row_id: row_id.to_string(),
                row_json: json,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn apply_delete_removes_row() {
        let mut rows = vec![TableRow {
            table_id: "t".into(),
            row_id: "1".into(),
            row_json: json!({"x": 1}),
        }];
        apply_row_event(&mut rows, "t", "1", None, true);
        assert!(rows.is_empty());
    }
}
