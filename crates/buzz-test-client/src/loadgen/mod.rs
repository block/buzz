//! Scenario-driven relay load generator.
//!
//! A scenario (TOML, see `perf/LOADGEN.md`) describes the data shape to seed
//! and a population of simulated humans and agents with per-connection rates.
//! `seed` creates the communities,
//! channels, history and deep threads; `run` drives the population through a
//! ramp of user multipliers and reports latency percentiles per operation.
//!
//! Every identity, community host and channel id is derived from the
//! scenario's `run_id`, so independent shards (`--shard i/n`) agree on the
//! fixture without sharing state.

pub mod identity;
pub mod net;
pub mod run;
pub mod scenario;
pub mod seed;
pub mod stats;
