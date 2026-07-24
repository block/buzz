//! `buzz-migrate` — the operator claim-service for zero-touch, no-takeover
//! Slack→Buzz identity migration.
//!
//! # The problem it solves
//!
//! History imported by `buzz import slack` is bot-signed and attributed only by
//! display name. To render each person's imported history under their real Buzz
//! profile, a **two-party binding** must exist for `slack:<id>`:
//!
//! 1. an owner/admin **attestation** (kind `KIND_IMPORT_IDENTITY_BINDING`), and
//! 2. the subject's self-signed **claim** (kind `KIND_IMPORT_IDENTITY_CLAIM`).
//!
//! Doing (1) by hand (`buzz import bind`) is O(N) manual work for a large team,
//! and manual matching is exactly where account-takeover mistakes creep in.
//! This service automates (1): it proves *which* Slack user a person is, then
//! publishes the attestation for them — so the operator does zero per-person
//! work and no one can seize another person's history.
//!
//! # Two proof channels
//!
//! - **Email magic-link** ([`token`], [`roster`]): the operator's export knows
//!   each user's email; a single-use, short-TTL token mailed to that address
//!   proves control of it, hence of the Slack user. See [`token`] for the exact
//!   threat model (why the token carries no pubkey).
//! - **Sign in with Slack (OIDC)**: the person authenticates to Slack live; the
//!   service reads back the verified user id. (Built on top of this core in the
//!   service layer.)
//!
//! Either way the outcome is one call to [`attest::publish_attestation`], and
//! the person's own client publishes the matching claim. Everything the service
//! signs is public-key-only and NIP-33-revocable.

pub mod attest;
pub mod roster;
pub mod token;
