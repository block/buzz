//! Wire shapes for the terminal frame protocol, shared by the Tauri IPC
//! channel and the remote (websocket) listener so both emit identical JSON.

use serde::{Deserialize, Serialize};

use crate::damage::Style;
use crate::transport::Publication;
use crate::Viewport;

pub type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WireViewport {
    pub generation: u64,
    pub columns: usize,
    pub screen_lines: usize,
}

impl From<Viewport> for WireViewport {
    fn from(value: Viewport) -> Self {
        Self {
            generation: value.generation,
            columns: value.columns,
            screen_lines: value.screen_lines,
        }
    }
}

impl From<WireViewport> for Viewport {
    fn from(value: WireViewport) -> Self {
        Self {
            generation: value.generation,
            columns: value.columns,
            screen_lines: value.screen_lines,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachResponse {
    pub session_id: String,
    pub subscription_id: String,
    pub viewport: WireViewport,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireStyle {
    pub fg: u32,
    pub bg: u32,
    pub flags: u16,
}

impl From<Style> for WireStyle {
    fn from(value: Style) -> Self {
        Self {
            fg: value.fg,
            bg: value.bg,
            flags: value.flags,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireCluster {
    pub column: usize,
    pub text: String,
    pub width: u8,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireSpan {
    pub style: WireStyle,
    pub clusters: Vec<WireCluster>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireRow {
    pub line: usize,
    pub wrapped: bool,
    pub spans: Vec<WireSpan>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireCursor {
    pub line: usize,
    pub column: usize,
    pub visible: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameMessage {
    pub subscription_id: String,
    pub sequence: u64,
    pub rows: Vec<WireRow>,
    pub cursor: WireCursor,
    pub full: bool,
    pub viewport: WireViewport,
    pub bracketed_paste: bool,
    pub focus_reporting: bool,
    pub mouse_reporting: bool,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
pub enum TerminalMessage {
    Frame(FrameMessage),
    Title(String),
    ResetTitle,
    Bell,
    Exit,
}

pub fn wire_publication(publication: Publication) -> Result<FrameMessage> {
    let frame = publication.frame;
    let rows = frame
        .rows
        .into_iter()
        .map(|row| {
            let spans = row
                .spans
                .into_iter()
                .map(|span| {
                    if !span.counts_are_consistent() {
                        return Err(
                            "terminal engine emitted an inconsistent cluster count".to_string()
                        );
                    }
                    let clusters = if span.cluster_count == 1 {
                        vec![WireCluster {
                            column: span.column,
                            text: span.text,
                            width: span.width,
                        }]
                    } else {
                        span.text
                            .chars()
                            .enumerate()
                            .map(|(index, ch)| WireCluster {
                                column: span.column + index * usize::from(span.width),
                                text: ch.to_string(),
                                width: span.width,
                            })
                            .collect()
                    };
                    Ok(WireSpan {
                        style: span.style.into(),
                        clusters,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(WireRow {
                line: row.line,
                wrapped: row.wrapped,
                spans,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(FrameMessage {
        subscription_id: publication.subscription_id.to_string(),
        sequence: publication.sequence,
        rows,
        cursor: WireCursor {
            line: frame.cursor.line,
            column: frame.cursor.column,
            visible: frame.cursor.visible,
        },
        full: frame.full,
        viewport: frame.viewport.into(),
        bracketed_paste: false,
        focus_reporting: false,
        mouse_reporting: false,
    })
}

