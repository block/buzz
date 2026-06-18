#![deny(unsafe_code)]
#![warn(missing_docs)]
//! Tamper-evident hash-chain audit log. Each entry chains to the previous via
//! SHA-256. Single-writer via Postgres `pg_advisory_lock`. AUTH events (kind 22242)
//! are rejected — they carry bearer tokens.

/// Audit action types recorded in the log.
pub mod action;
/// Audit log entry types (stored and input).
pub mod entry;
/// Error types for audit operations.
pub mod error;
/// SHA-256 hash computation for audit entries.
pub mod hash;
/// SQL schema for the audit log table.
pub mod schema;
/// Audit log service — append and verify entries.
pub mod service;

pub use action::AuditAction;
pub use entry::{AuditEntry, NewAuditEntry};
pub use error::AuditError;
pub use hash::{compute_hash, GENESIS_HASH};
pub use schema::AUDIT_SCHEMA_SQL;
pub use service::AuditService;
