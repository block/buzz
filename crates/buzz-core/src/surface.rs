//! Surface Cards — versioned, data-only UI spec rendered as a native card.
//!
//! A surface event's content is a small JSON document (`SurfaceSpec v1`):
//! `version`, `fallbackText`, optional `title`, and an ordered list of
//! display nodes (heading, text, badge, keyValue, statGrid, table, progress).
//! The author controls content, never layout or styling — the spec carries no
//! markup, links, or scripts. Live updates reuse the existing edit kind
//! (`KIND_STREAM_MESSAGE_EDIT`) with full-spec replacement.
//!
//! The relay is the strict gate: [`parse_and_validate`] rejects unknown
//! fields, unknown node types, unknown tones, non-scalar cell values, and any
//! structural-limit violation. Clients parse tolerantly (drop invalid nodes,
//! fall back to `fallbackText`) for historical/foreign events — that
//! tolerance lives client-side, not here.
//!
//! Producers (SDK/CLI) normalize known field aliases via
//! [`normalize_spec_aliases`] and serialize canonically *before* signing, so
//! stored specs are canonical. See block/buzz#2480 for the design discussion.

use serde::{Deserialize, Serialize};

/// Maximum canonical JSON content size in bytes (32 KiB).
///
/// The relay WebSocket frame limit is 64 KiB for the whole event envelope
/// (tags + sig included), so a 64 KiB content cap could never fit — the diff
/// kind uses 60 KiB for the same reason. Surfaces are far smaller; 32 KiB
/// bounds worst-case render work while leaving generous headroom for
/// data-heavy tables.
pub const SURFACE_MAX_CONTENT_BYTES: usize = 32 * 1024;
/// Maximum number of nodes in a spec.
pub const SURFACE_MAX_NODES: usize = 32;
/// Maximum `title` length in characters.
pub const SURFACE_MAX_TITLE_CHARS: usize = 256;
/// Maximum `fallbackText` length in characters.
pub const SURFACE_MAX_FALLBACK_CHARS: usize = 512;
/// Maximum `text` node body length in characters.
pub const SURFACE_MAX_TEXT_CHARS: usize = 4096;
/// Maximum length of any label, value, cell, or delta in characters.
pub const SURFACE_MAX_SCALAR_CHARS: usize = 512;
/// Maximum `keyValue.items` / `statGrid.stats` entries per node.
pub const SURFACE_MAX_ITEMS: usize = 32;
/// Maximum table columns.
pub const SURFACE_MAX_TABLE_COLUMNS: usize = 12;
/// Maximum table rows.
pub const SURFACE_MAX_TABLE_ROWS: usize = 100;
/// The only supported spec version.
pub const SURFACE_SPEC_VERSION: u32 = 1;

/// Validation failure with a field-specific message.
///
/// The message names the offending field (e.g. `nodes[3].table: 14 columns
/// exceeds max 12`) so producers — human or agent — can fix the payload
/// without a relay round-trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceError(String);

impl SurfaceError {
    fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl std::fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SurfaceError {}

/// Semantic tone applied to badges, key-value entries, and stats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceTone {
    /// Neutral (the default when omitted).
    #[default]
    Default,
    /// Positive / healthy.
    Success,
    /// Needs attention.
    Warning,
    /// Failing / destructive.
    Danger,
    /// Informational.
    Info,
}

/// A scalar cell value: a string or a finite JSON number.
///
/// Booleans, objects, arrays, and null are rejected by construction — the
/// untagged enum only admits these two shapes. `serde_json::Number` preserves
/// the author's integer/decimal representation through canonicalization.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SurfaceValue {
    /// Text value.
    Text(String),
    /// Finite numeric value.
    Number(serde_json::Number),
}

impl SurfaceValue {
    /// Character length of the value as displayed.
    fn display_len(&self) -> usize {
        match self {
            Self::Text(s) => s.chars().count(),
            Self::Number(n) => n.to_string().chars().count(),
        }
    }

    fn check_finite(&self, field: &str) -> Result<(), SurfaceError> {
        if let Self::Number(n) = self {
            match n.as_f64() {
                Some(v) if v.is_finite() => {}
                _ => return Err(SurfaceError::new(format!("{field}: number must be finite"))),
            }
        }
        Ok(())
    }
}

/// One `keyValue` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceKeyValueItem {
    /// Entry label.
    pub label: String,
    /// Entry value.
    pub value: SurfaceValue,
    /// Semantic tone (defaults to neutral).
    #[serde(default, skip_serializing_if = "is_default_tone")]
    pub tone: SurfaceTone,
}

/// One `statGrid` entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceStatItem {
    /// Stat label.
    pub label: String,
    /// Stat value.
    pub value: SurfaceValue,
    /// Optional delta shown under the value (e.g. `"+8%"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delta: Option<SurfaceValue>,
    /// Semantic tone (defaults to neutral).
    #[serde(default, skip_serializing_if = "is_default_tone")]
    pub tone: SurfaceTone,
}

fn is_default_tone(tone: &SurfaceTone) -> bool {
    *tone == SurfaceTone::Default
}

/// A single display node. Rendered in order; v1 nodes are display-only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", deny_unknown_fields)]
pub enum SurfaceNode {
    /// Section heading.
    #[serde(rename = "heading")]
    Heading {
        /// Heading text.
        text: String,
    },
    /// Free-form paragraph (plain text — never markdown).
    #[serde(rename = "text")]
    Text {
        /// Paragraph body.
        text: String,
    },
    /// Small status pill.
    #[serde(rename = "badge")]
    Badge {
        /// Badge text.
        text: String,
        /// Semantic tone (defaults to neutral).
        #[serde(default, skip_serializing_if = "is_default_tone")]
        tone: SurfaceTone,
    },
    /// Label/value list.
    #[serde(rename = "keyValue")]
    KeyValue {
        /// Entries, rendered in order.
        items: Vec<SurfaceKeyValueItem>,
    },
    /// Grid of stat tiles.
    #[serde(rename = "statGrid")]
    StatGrid {
        /// Stats, rendered in order.
        stats: Vec<SurfaceStatItem>,
    },
    /// Semantic table.
    #[serde(rename = "table")]
    Table {
        /// Column headers. Every row must have exactly this many cells.
        columns: Vec<String>,
        /// Row cells.
        rows: Vec<Vec<SurfaceValue>>,
    },
    /// Progress bar. Clients clamp `value` to 0–100.
    #[serde(rename = "progress")]
    Progress {
        /// Optional bar label.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        /// Progress value; must be a finite number.
        value: serde_json::Number,
    },
}

/// A `SurfaceSpec v1` document — the content of a surface event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceSpecV1 {
    /// Spec version; must be [`SURFACE_SPEC_VERSION`].
    pub version: u32,
    /// Plain-text summary shown by non-rendering clients and all failure
    /// paths. Required, non-empty.
    #[serde(rename = "fallbackText")]
    pub fallback_text: String,
    /// Optional small uppercase label above the card body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Display nodes, rendered in order (1–32).
    pub nodes: Vec<SurfaceNode>,
}

impl SurfaceSpecV1 {
    /// Validate the spec against every structural limit in the v1 protocol.
    pub fn validate(&self) -> Result<(), SurfaceError> {
        if self.version != SURFACE_SPEC_VERSION {
            return Err(SurfaceError::new(format!(
                "version: must be {SURFACE_SPEC_VERSION} (got {})",
                self.version
            )));
        }
        check_text_field(
            &self.fallback_text,
            SURFACE_MAX_FALLBACK_CHARS,
            "fallbackText",
        )?;
        if let Some(title) = &self.title {
            check_text_field(title, SURFACE_MAX_TITLE_CHARS, "title")?;
        }
        if self.nodes.is_empty() {
            return Err(SurfaceError::new("nodes: at least one node is required"));
        }
        if self.nodes.len() > SURFACE_MAX_NODES {
            return Err(SurfaceError::new(format!(
                "nodes: {} nodes exceeds max {SURFACE_MAX_NODES}",
                self.nodes.len()
            )));
        }
        for (i, node) in self.nodes.iter().enumerate() {
            validate_node(node, i)?;
        }
        Ok(())
    }

    /// Serialize to canonical JSON (stable field order, no insignificant
    /// whitespace) and enforce [`SURFACE_MAX_CONTENT_BYTES`].
    pub fn canonical_json(&self) -> Result<String, SurfaceError> {
        let json = serde_json::to_string(self)
            .map_err(|e| SurfaceError::new(format!("serialize: {e}")))?;
        if json.len() > SURFACE_MAX_CONTENT_BYTES {
            return Err(SurfaceError::new(format!(
                "content: {} bytes exceeds max {SURFACE_MAX_CONTENT_BYTES}",
                json.len()
            )));
        }
        Ok(json)
    }
}

fn check_text_field(value: &str, max_chars: usize, field: &str) -> Result<(), SurfaceError> {
    if value.trim().is_empty() {
        return Err(SurfaceError::new(format!("{field}: must not be empty")));
    }
    let len = value.chars().count();
    if len > max_chars {
        return Err(SurfaceError::new(format!(
            "{field}: {len} chars exceeds max {max_chars}"
        )));
    }
    Ok(())
}

fn check_value(value: &SurfaceValue, field: &str) -> Result<(), SurfaceError> {
    value.check_finite(field)?;
    let len = value.display_len();
    if len > SURFACE_MAX_SCALAR_CHARS {
        return Err(SurfaceError::new(format!(
            "{field}: {len} chars exceeds max {SURFACE_MAX_SCALAR_CHARS}"
        )));
    }
    Ok(())
}

fn validate_node(node: &SurfaceNode, i: usize) -> Result<(), SurfaceError> {
    match node {
        SurfaceNode::Heading { text } => check_text_field(
            text,
            SURFACE_MAX_SCALAR_CHARS,
            &format!("nodes[{i}].heading.text"),
        ),
        SurfaceNode::Text { text } => check_text_field(
            text,
            SURFACE_MAX_TEXT_CHARS,
            &format!("nodes[{i}].text.text"),
        ),
        SurfaceNode::Badge { text, .. } => check_text_field(
            text,
            SURFACE_MAX_SCALAR_CHARS,
            &format!("nodes[{i}].badge.text"),
        ),
        SurfaceNode::KeyValue { items } => {
            if items.is_empty() {
                return Err(SurfaceError::new(format!(
                    "nodes[{i}].keyValue: at least one item is required"
                )));
            }
            if items.len() > SURFACE_MAX_ITEMS {
                return Err(SurfaceError::new(format!(
                    "nodes[{i}].keyValue: {} items exceeds max {SURFACE_MAX_ITEMS}",
                    items.len()
                )));
            }
            for (j, item) in items.iter().enumerate() {
                check_text_field(
                    &item.label,
                    SURFACE_MAX_SCALAR_CHARS,
                    &format!("nodes[{i}].keyValue.items[{j}].label"),
                )?;
                check_value(
                    &item.value,
                    &format!("nodes[{i}].keyValue.items[{j}].value"),
                )?;
            }
            Ok(())
        }
        SurfaceNode::StatGrid { stats } => {
            if stats.is_empty() {
                return Err(SurfaceError::new(format!(
                    "nodes[{i}].statGrid: at least one stat is required"
                )));
            }
            if stats.len() > SURFACE_MAX_ITEMS {
                return Err(SurfaceError::new(format!(
                    "nodes[{i}].statGrid: {} stats exceeds max {SURFACE_MAX_ITEMS}",
                    stats.len()
                )));
            }
            for (j, stat) in stats.iter().enumerate() {
                check_text_field(
                    &stat.label,
                    SURFACE_MAX_SCALAR_CHARS,
                    &format!("nodes[{i}].statGrid.stats[{j}].label"),
                )?;
                check_value(
                    &stat.value,
                    &format!("nodes[{i}].statGrid.stats[{j}].value"),
                )?;
                if let Some(delta) = &stat.delta {
                    check_value(delta, &format!("nodes[{i}].statGrid.stats[{j}].delta"))?;
                }
            }
            Ok(())
        }
        SurfaceNode::Table { columns, rows } => {
            if columns.is_empty() {
                return Err(SurfaceError::new(format!(
                    "nodes[{i}].table: at least one column is required"
                )));
            }
            if columns.len() > SURFACE_MAX_TABLE_COLUMNS {
                return Err(SurfaceError::new(format!(
                    "nodes[{i}].table: {} columns exceeds max {SURFACE_MAX_TABLE_COLUMNS}",
                    columns.len()
                )));
            }
            if rows.len() > SURFACE_MAX_TABLE_ROWS {
                return Err(SurfaceError::new(format!(
                    "nodes[{i}].table: {} rows exceeds max {SURFACE_MAX_TABLE_ROWS}",
                    rows.len()
                )));
            }
            for (j, col) in columns.iter().enumerate() {
                check_text_field(
                    col,
                    SURFACE_MAX_SCALAR_CHARS,
                    &format!("nodes[{i}].table.columns[{j}]"),
                )?;
            }
            for (r, row) in rows.iter().enumerate() {
                if row.len() != columns.len() {
                    return Err(SurfaceError::new(format!(
                        "nodes[{i}].table.rows[{r}]: {} cells does not match {} columns",
                        row.len(),
                        columns.len()
                    )));
                }
                for (c, cell) in row.iter().enumerate() {
                    check_value(cell, &format!("nodes[{i}].table.rows[{r}][{c}]"))?;
                }
            }
            Ok(())
        }
        SurfaceNode::Progress { label, value } => {
            if let Some(label) = label {
                check_text_field(
                    label,
                    SURFACE_MAX_SCALAR_CHARS,
                    &format!("nodes[{i}].progress.label"),
                )?;
            }
            match value.as_f64() {
                Some(v) if v.is_finite() => Ok(()),
                _ => Err(SurfaceError::new(format!(
                    "nodes[{i}].progress.value: number must be finite"
                ))),
            }
        }
    }
}

/// Parse and strictly validate surface event content.
///
/// This is the relay's ingest gate: size cap first, then strict schema parse
/// (unknown fields, unknown node types, unknown tones, and non-scalar cells
/// all reject), then every structural limit.
pub fn parse_and_validate(content: &str) -> Result<SurfaceSpecV1, SurfaceError> {
    if content.len() > SURFACE_MAX_CONTENT_BYTES {
        return Err(SurfaceError::new(format!(
            "content: {} bytes exceeds max {SURFACE_MAX_CONTENT_BYTES}",
            content.len()
        )));
    }
    let spec: SurfaceSpecV1 = serde_json::from_str(content)
        .map_err(|e| SurfaceError::new(format!("invalid surface spec JSON: {e}")))?;
    spec.validate()?;
    Ok(spec)
}

/// Normalize known field aliases observed from real models, in place.
///
/// Producer-side only (SDK/CLI) — run *before* parsing/signing so stored
/// specs are canonical. The relay stays strict and rejects aliases.
///
/// Current aliases: `table.fields` and `table.headers` → `table.columns`.
pub fn normalize_spec_aliases(value: &mut serde_json::Value) {
    let Some(nodes) = value.get_mut("nodes").and_then(|n| n.as_array_mut()) else {
        return;
    };
    for node in nodes {
        let Some(obj) = node.as_object_mut() else {
            continue;
        };
        let is_table = obj.get("type").and_then(|t| t.as_str()) == Some("table");
        if !is_table || obj.contains_key("columns") {
            continue;
        }
        for alias in ["fields", "headers"] {
            if let Some(cols) = obj.remove(alias) {
                obj.insert("columns".to_string(), cols);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_spec_json() -> String {
        r#"{
            "version": 1,
            "fallbackText": "Deploy v2.4.1: 2/2 pods running, rollout 100%",
            "title": "Deployment — api-gateway",
            "nodes": [
                {"type": "badge", "text": "HEALTHY", "tone": "success"},
                {"type": "heading", "text": "Pods"},
                {"type": "text", "text": "All pods running."},
                {"type": "keyValue", "items": [{"label": "Version", "value": "v2.4.1", "tone": "info"}]},
                {"type": "statGrid", "stats": [{"label": "Pods", "value": 2, "delta": "+1", "tone": "success"}, {"label": "Errors", "value": 0}]},
                {"type": "table", "columns": ["Pod", "Status"], "rows": [["web-7d9f", "Running"], ["web-a1c2", 3]]},
                {"type": "progress", "label": "Rollout", "value": 100}
            ]
        }"#
        .to_string()
    }

    #[test]
    fn parses_and_validates_all_node_types() {
        let spec = parse_and_validate(&demo_spec_json()).expect("valid spec");
        assert_eq!(spec.version, 1);
        assert_eq!(spec.nodes.len(), 7);
        assert_eq!(spec.title.as_deref(), Some("Deployment — api-gateway"));
    }

    #[test]
    fn canonical_json_round_trips_and_is_stable() {
        let spec = parse_and_validate(&demo_spec_json()).expect("valid spec");
        let canonical = spec.canonical_json().expect("canonical");
        let reparsed = parse_and_validate(&canonical).expect("canonical parses");
        assert_eq!(spec, reparsed);
        assert_eq!(canonical, reparsed.canonical_json().expect("stable"));
        // Integer representation is preserved (2 stays 2, not 2.0).
        assert!(canonical.contains(r#""value":2,"#));
    }

    #[test]
    fn rejects_wrong_version() {
        let content = r#"{"version":2,"fallbackText":"x","nodes":[{"type":"text","text":"y"}]}"#;
        let err = parse_and_validate(content).expect_err("version 2 rejected");
        assert!(err.to_string().contains("version"), "{err}");
    }

    #[test]
    fn rejects_unknown_node_type_and_unknown_fields() {
        let unknown_node =
            r#"{"version":1,"fallbackText":"x","nodes":[{"type":"iframe","src":"https://x"}]}"#;
        assert!(parse_and_validate(unknown_node).is_err());

        let unknown_field = r#"{"version":1,"fallbackText":"x","onClick":"alert(1)","nodes":[{"type":"text","text":"y"}]}"#;
        assert!(parse_and_validate(unknown_field).is_err());
    }

    #[test]
    fn rejects_unknown_tone() {
        let content = r#"{"version":1,"fallbackText":"x","nodes":[{"type":"badge","text":"B","tone":"sparkly"}]}"#;
        assert!(parse_and_validate(content).is_err());
    }

    #[test]
    fn rejects_non_scalar_cells() {
        for cell in ["true", "null", "{}", "[1]"] {
            let content = format!(
                r#"{{"version":1,"fallbackText":"x","nodes":[{{"type":"table","columns":["A"],"rows":[[{cell}]]}}]}}"#
            );
            assert!(
                parse_and_validate(&content).is_err(),
                "cell {cell} must be rejected"
            );
        }
    }

    #[test]
    fn rejects_ragged_table_rows() {
        let content = r#"{"version":1,"fallbackText":"x","nodes":[{"type":"table","columns":["A","B"],"rows":[["only-one"]]}]}"#;
        let err = parse_and_validate(content).expect_err("ragged row rejected");
        assert!(err.to_string().contains("does not match"), "{err}");
    }

    #[test]
    fn enforces_node_count_boundaries() {
        let node = r#"{"type":"text","text":"y"}"#;
        let build = |n: usize| {
            format!(
                r#"{{"version":1,"fallbackText":"x","nodes":[{}]}}"#,
                vec![node; n].join(",")
            )
        };
        assert!(parse_and_validate(&build(SURFACE_MAX_NODES)).is_ok());
        assert!(parse_and_validate(&build(SURFACE_MAX_NODES + 1)).is_err());
        assert!(parse_and_validate(r#"{"version":1,"fallbackText":"x","nodes":[]}"#).is_err());
    }

    #[test]
    fn enforces_table_shape_boundaries() {
        let cols_ok: Vec<String> = (0..SURFACE_MAX_TABLE_COLUMNS)
            .map(|i| format!("\"c{i}\""))
            .collect();
        let content = format!(
            r#"{{"version":1,"fallbackText":"x","nodes":[{{"type":"table","columns":[{}],"rows":[]}}]}}"#,
            cols_ok.join(",")
        );
        assert!(parse_and_validate(&content).is_ok());

        let cols_bad: Vec<String> = (0..=SURFACE_MAX_TABLE_COLUMNS)
            .map(|i| format!("\"c{i}\""))
            .collect();
        let content = format!(
            r#"{{"version":1,"fallbackText":"x","nodes":[{{"type":"table","columns":[{}],"rows":[]}}]}}"#,
            cols_bad.join(",")
        );
        let err = parse_and_validate(&content).expect_err("13 columns rejected");
        assert!(err.to_string().contains("exceeds max 12"), "{err}");

        let row = r#"["v"]"#;
        let rows_bad = vec![row; SURFACE_MAX_TABLE_ROWS + 1].join(",");
        let content = format!(
            r#"{{"version":1,"fallbackText":"x","nodes":[{{"type":"table","columns":["A"],"rows":[{rows_bad}]}}]}}"#
        );
        assert!(parse_and_validate(&content).is_err());
    }

    #[test]
    fn enforces_scalar_and_text_lengths() {
        let long = "x".repeat(SURFACE_MAX_SCALAR_CHARS + 1);
        let content = format!(
            r#"{{"version":1,"fallbackText":"x","nodes":[{{"type":"badge","text":"{long}"}}]}}"#
        );
        assert!(parse_and_validate(&content).is_err());

        let long_fallback = "x".repeat(SURFACE_MAX_FALLBACK_CHARS + 1);
        let content = format!(
            r#"{{"version":1,"fallbackText":"{long_fallback}","nodes":[{{"type":"text","text":"y"}}]}}"#
        );
        assert!(parse_and_validate(&content).is_err());

        let text_ok = "y".repeat(SURFACE_MAX_TEXT_CHARS);
        let content = format!(
            r#"{{"version":1,"fallbackText":"x","nodes":[{{"type":"text","text":"{text_ok}"}}]}}"#
        );
        assert!(parse_and_validate(&content).is_ok());
    }

    #[test]
    fn enforces_content_byte_cap() {
        // A single text node may hold at most 4096 chars, so the byte cap
        // needs many nodes — build 32 near-max text nodes (~128 KiB total).
        let body = "y".repeat(SURFACE_MAX_TEXT_CHARS);
        let node = format!(r#"{{"type":"text","text":"{body}"}}"#);
        let content = format!(
            r#"{{"version":1,"fallbackText":"x","nodes":[{}]}}"#,
            vec![node.as_str(); SURFACE_MAX_NODES].join(",")
        );
        assert!(content.len() > SURFACE_MAX_CONTENT_BYTES);
        let err = parse_and_validate(&content).expect_err("oversize rejected");
        assert!(err.to_string().contains("exceeds max"), "{err}");
    }

    #[test]
    fn normalizes_table_column_aliases() {
        for alias in ["fields", "headers"] {
            let mut value: serde_json::Value = serde_json::from_str(&format!(
                r#"{{"version":1,"fallbackText":"x","nodes":[{{"type":"table","{alias}":["A"],"rows":[["v"]]}}]}}"#
            ))
            .expect("json");
            normalize_spec_aliases(&mut value);
            let content = value.to_string();
            let spec = parse_and_validate(&content).expect("alias normalized");
            assert!(matches!(spec.nodes[0], SurfaceNode::Table { .. }));
        }
        // Explicit columns win over an alias; the alias would then reject
        // downstream as an unknown field — normalization must not clobber.
        let mut value: serde_json::Value = serde_json::from_str(
            r#"{"version":1,"fallbackText":"x","nodes":[{"type":"table","columns":["A"],"rows":[["v"]]}]}"#,
        )
        .expect("json");
        let before = value.clone();
        normalize_spec_aliases(&mut value);
        assert_eq!(before, value);
    }

    #[test]
    fn tone_default_is_omitted_from_canonical_form() {
        let content = r#"{"version":1,"fallbackText":"x","nodes":[{"type":"badge","text":"B","tone":"default"}]}"#;
        let spec = parse_and_validate(content).expect("valid");
        let canonical = spec.canonical_json().expect("canonical");
        assert!(!canonical.contains("tone"), "{canonical}");
    }

    #[test]
    fn rejects_empty_fallback_and_blank_text() {
        let content = r#"{"version":1,"fallbackText":"  ","nodes":[{"type":"text","text":"y"}]}"#;
        assert!(parse_and_validate(content).is_err());
        let content = r#"{"version":1,"fallbackText":"x","nodes":[{"type":"heading","text":""}]}"#;
        assert!(parse_and_validate(content).is_err());
    }
}
