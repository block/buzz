//! Parser and loader for Buzz persona pack files (`.persona.md`).
//!
//! A persona pack bundles one or more agent personas — each a YAML frontmatter
//! header plus a markdown system prompt — together with a `.plugin/plugin.json`
//! manifest, optional shared MCP config, and pack-level behavioral defaults.
//!
//! This crate provides parsing, loading, merge/precedence resolution, ACP-ready
//! projection, and validation of persona packs.

#![warn(missing_docs)]

/// Pack manifest types and `.plugin/plugin.json` parser.
pub mod manifest;

/// Precedence resolution for persona behavioral config.
pub mod merge;

/// Pack directory loader.
pub mod pack;

/// Core persona types and `.persona.md` parser.
pub mod persona;

/// Pack resolution producing fully resolved, ACP-ready output.
pub mod resolve;

/// Pack validation (`buzz pack validate`).
pub mod validate;
