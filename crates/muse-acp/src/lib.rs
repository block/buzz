//! `muse-acp`: an ACP agent that relays Buzz channel prompts to Muse.
//!
//! Buzz's harness (`buzz-acp`) spawns this binary and speaks the Agent
//! Client Protocol to it over stdio: `initialize`, `session/new`,
//! `session/prompt`, `session/cancel`. This crate answers those requests by
//! driving the user's Muse web session in Chromium — the same session the
//! user talks to in their browser — so a Buzz channel conversation feels
//! exactly like chatting on muse.ai, with the channel's framed prompt as
//! additional context.
//!
//! The crate is split into layers so the fragile part stays small:
//!
//! * [`acp`] — the newline-delimited JSON-RPC loop and session bookkeeping.
//! * [`driver`] — the transport-neutral turn interface (`Driver` trait).
//! * [`muse_web`] — the [`driver::Driver`] implementation for muse.ai.
//! * [`chrome`] — Chromium launcher and a minimal CDP client.
//! * [`selectors`] — the DOM model of the Muse web app. This is the one
//!   module that must be re-verified whenever the web app changes; it fails
//!   loudly (naming what it looked for) instead of failing silently.
//! * [`auth`] — the persisted-login marker behind `muse-acp auth status`.

pub mod acp;
pub mod auth;
pub mod chrome;
pub mod dm;
pub mod driver;
pub mod handoff;
pub mod mapping;
pub mod muse_web;
pub mod selectors;
pub mod threading;

mod error;

pub use driver::{Driver, TurnOutcome};
pub use error::BridgeError;
