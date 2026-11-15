//! Stored state behind `GET /buzz/v1/me/activity`: what each conversation and
//! thread last did, and what each person has read of it.
//!
//! Two shared tables hold one row per conversation ([`channel_heads`]) and one
//! per thread with a reply (`thread_heads`). Comparing a row's arrival time
//! with the person's read position (`personal_read_frontiers`) gives unread,
//! so a list read is a join and never a per-channel scan.
//!
//! The event log is the truth and a shared row is a cache of it that proves
//! its own freshness: `seen_at` is the newest stored event the row accounts
//! for, and a read asks the log, with one index probe per row, whether
//! anything newer exists. A row that is behind is rebuilt before it is
//! returned. So no writer has to be reliable for a read to be right:
//!
//! - **Ingest.** [`stage`] and [`seen`] record a stored event on its
//!   [`crate::AdmittedTx`]. Once that commits, [`submit`] hands the result to
//!   a background writer that coalesces per channel and writes a moment
//!   later. No message transaction holds or waits for a shared row.
//! - **Fill.** A read that meets a row that is missing, behind, or whose
//!   newest message is deleted merges the stored history into it.
//!
//! The same background write records what a message does to people's own
//! rows: it moves the author's read position, and it marks the thread as one
//! the parent's author and the people tagged are part of.
//!
//! Rows are display state. They never grant access: readers join current
//! membership, and callers recheck it before release.
//!
//! [`channel_heads`]: self

mod fill;
mod model;
mod read;
mod writer;

pub use fill::Warmed;
pub use model::*;
#[cfg(test)]
pub(crate) use writer::flush;
pub use writer::flush_pending;
pub(crate) use writer::{seen, stage, submit, Staged};

#[cfg(test)]
mod postgres_tests;
