#![deny(unsafe_code)]
//! 9 IDE and AI coding agent transcript parsers.

pub mod agy_cli;
pub mod antigravity;
pub mod claude_code;
pub mod codex;
pub mod cursor;
pub mod goose;
pub mod kimi;
pub mod opencode;
pub mod zcode;

pub use agy_cli::AgyCliRecallPlugin;
pub use antigravity::AntigravityRecallPlugin;
pub use claude_code::ClaudeCodeRecallPlugin;
pub use codex::CodexRecallPlugin;
pub use cursor::CursorRecallPlugin;
pub use goose::GooseRecallPlugin;
pub use kimi::KimiRecallPlugin;
pub use opencode::OpenCodeRecallPlugin;
pub use zcode::ZCodeRecallPlugin;
