//! Fold engine core: incrementally maintained, provenance-carrying artifacts
//! over the Buzz event stream.
//!
//! The accumulator maintains small, always-current summaries ("artifacts") of
//! selections of the event stream. Each update is a *fold*: the next artifact
//! is a function of the previous artifact plus only the signals that arrived
//! since the last run — never a re-read of history.
//!
//! Three nouns, one recurrence (signals — raw relay events, never rewritten —
//! are the substrate underneath all three):
//! - **selection** — a frozen-or-live description of a signal set: who ×
//!   what × when (channels, authors, kinds, and the selection's own window).
//!   A pinned `until_exclusive` freezes it; an open end means "and whatever
//!   comes next".
//! - **fold** — name + selection + model + instructions: a factory for
//!   artifacts. `artifact' = fold(artifact, new_signals, spec)`. A frozen
//!   selection makes the fold run until covered, then done forever; a live
//!   one is never done.
//! - **artifact** — the model's response, verbatim and free-form, persisted
//!   locally as an immutable, append-only version chain whose provenance
//!   (exactly which signals went in, and when) is engine-computed. Publishing
//!   an artifact back into a channel makes it a signal again — later folds
//!   can select it, so composition needs no extra machinery.
//!
//! The engine modules are deliberately **pure**: they plan runs, price them,
//! and render transcripts — but perform no relay I/O and hold no storage.
//! Callers (the `buzz` CLI, the [`daemon`]) fetch signals and prior
//! artifacts, hand them to [`run::plan_run`], invoke a [`runner::FoldRunner`]
//! if the plan is ready, and finish with [`run::complete_run`].
//!
//! The optional `daemon` feature (default; engine consumers opt out with
//! `default-features = false`) adds the standalone headless app: it borrows a
//! person's key, mirrors everything that key can see from the relay into local
//! SQLite, holds the live subscription, and exposes connection/backfill status
//! plus the fold machinery (selections, folds, runs, artifacts) over a
//! localhost HTTP API for an external UI. `cargo run -p buzz-accumulator`.
//!
//! Honesty invariants the engine enforces (ported from the X-Ray POC and
//! covered by unit tests in each module):
//! - Estimates are computed without any model call, and an unknown model's
//!   window fit is an honest `None`, never a guess ([`estimate`]).
//! - Coverage records exactly the signals the model was shown — an oversized
//!   window stalls or truncates honestly; unread signals are never sealed
//!   ([`run`]).
//! - Provenance is engine-owned, never read from model output: `shown_ids`
//!   and the coverage window come from the plan. The output itself is
//!   free-form — `[event:<id>]` citations in it are reader-verifiable links,
//!   not a validated contract ([`artifact`]).

pub mod artifact;
#[cfg(feature = "daemon")]
pub mod daemon;
pub mod error;
pub mod estimate;
pub mod run;
pub mod runner;
pub mod selection;
pub mod signal;
pub mod spec;
pub mod transcript;

pub use artifact::ArtifactPayload;
pub use error::Error;
pub use run::{complete_run, plan_run, Plan, RunPlan};
pub use runner::{FoldRunner, SubprocessRunner};
pub use selection::Selection;
pub use signal::Signal;
pub use spec::FoldSpec;
