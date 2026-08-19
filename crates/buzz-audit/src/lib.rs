#![deny(unsafe_code)]
#![warn(missing_docs)]
//! Tamper-evident, **per-community** hash-chain audit log.
//!
//! Each community owns an independent chain: rows are keyed `(community_id, seq)`,
//! `seq` is monotonic *within a community*, and each entry chains to the previous
//! entry *of the same community* via SHA-256. The `community_id` is folded into the
//! hash, so a row lifted out of one community's chain can never verify inside
//! another's — chain identity carries the tenant. This is the audit half of the
//! non-interference floor (`auditHeads[c]` in `MultiTenantRelay.tla`): an audit
//! observation reveals only its own community's head.
//!
//! Writes for a given community are serialized by a **per-community** Postgres
//! advisory lock, so the chain stays consistent across relay processes without one
//! global lock serializing (and timing-coupling) every tenant.
//!
//! The `audit_log` table is owned by the consolidated `0001` migration — this crate
//! is pure chain logic and ships no DDL.

/// Audit action types recorded in the log.
pub mod action;
/// Audit log entry types (stored and input).
pub mod entry;
/// Error types for audit operations.
pub mod error;
/// SHA-256 hash computation for audit entries.
pub mod hash;
/// Audit log service — append and verify entries.
pub mod service;

pub use action::AuditAction;
pub use entry::{AuditEntry, NewAuditEntry};
pub use error::AuditError;
pub use hash::{compute_hash, GENESIS_HASH};
pub use service::AuditService;
