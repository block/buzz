//! Fold engine core: incrementally maintained, provenance-carrying artifacts
//! over the Buzz event stream.
//!
//! The accumulator maintains small, always-current summaries ("artifacts") of
//! selections of the event stream. Each update is a *fold*: the next artifact
//! is a function of the previous artifact plus only the signals that arrived
//! since the last run — never a re-read of history.
//!
//! Five nouns, one recurrence:
//! - **signal** — a raw relay event, never rewritten; the source of truth.
//! - **selection** — a saveable query over signals (channels, authors, kinds).
//! - **fold** — `artifact' = fold(artifact, new_signals, spec)`.
//! - **artifact** — compact sectioned output where every claim cites the exact
//!   signals it came from; published back to the relay as an immutable event.
//! - (a **lens** renders artifacts into UI; rendering is out of scope here.)
//!
//! This crate is deliberately **pure**: it plans runs, prices them, renders
//! transcripts, validates model output, and splices history — but it performs
//! no relay I/O and holds no storage. Callers (the `buzz` CLI, the desktop
//! app) fetch signals and prior artifacts from the relay, hand them to
//! [`run::plan_run`], invoke a [`runner::FoldRunner`] if the plan is ready,
//! and finish with [`run::complete_run`]. All state lives in signed relay
//! events, so the artifact store is append-only by construction.
//!
//! Honesty invariants the engine enforces (ported from the X-Ray POC and
//! covered by unit tests in each module):
//! - Estimates are computed without any model call, and an unknown model's
//!   cost is an honest `None`, never a guess ([`estimate`]).
//! - Coverage records exactly the signals the model was shown — an oversized
//!   window stalls or truncates honestly; unread signals are never sealed
//!   ([`run`]).
//! - Output that breaks the artifact contract — wrong sections, a citation of
//!   a signal the model was not shown, or no citation at all — is refused and
//!   nothing persists ([`validate`]).
//! - Prior append-section history is retained by the engine, not the model:
//!   history cannot be rewritten regardless of model output ([`validate::splice_append_sections`]).

pub mod artifact;
pub mod error;
pub mod estimate;
pub mod run;
pub mod runner;
pub mod schema;
pub mod selection;
pub mod signal;
pub mod spec;
pub mod transcript;
pub mod validate;

pub use artifact::ArtifactPayload;
pub use error::Error;
pub use run::{complete_run, plan_run, Plan, RunPlan};
pub use runner::{FoldRunner, SubprocessRunner};
pub use selection::Selection;
pub use signal::Signal;
pub use spec::FoldSpec;
