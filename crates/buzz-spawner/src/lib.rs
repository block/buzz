#![deny(unsafe_code)]
#![warn(missing_docs)]
//! `buzz-spawner` — reconciles Nostr agent specs into running agent containers.
//!
//! # What it is
//!
//! A standalone daemon that runs beside a Buzz relay and gives self-hosters
//! first-party server-side agents. Without it, every agent in Buzz is spawned by
//! the desktop app: agents die when the laptop sleeps, and mobile and web cannot
//! create one at all.
//!
//! It is deliberately *not* part of `buzz-relay`. The relay's only subprocess
//! today is `git`, and it goes out of its way to disable repo hooks; arbitrary
//! container execution does not belong in a public-facing WebSocket server. This
//! follows the existing sidecar-crate precedent (`buzz-pair-relay`,
//! `buzz-push-gateway`).
//!
//! # How it works
//!
//! The daemon is an ordinary relay client with its own Nostr identity. It holds
//! no database connection and no privileged relay access.
//!
//! ```text
//! owner   ──kind:30178 spec───────────────────────────►  spawner
//! spawner ──kind:24201 request (agent pubkey + nonce)─►  owner
//! owner   ──kind:24201 response (NIP-OA auth tag)─────►  spawner
//! spawner ──kind:30179 status: running────────────────►  owner
//!                        │
//!                        └──► Docker: one buzz-acp container per agent
//! ```
//!
//! Desired state is [`buzz_sdk::spawner::SpawnerAgentSpec`] events; actual state
//! is Docker containers labelled `com.buzz.agent`. [`reconcile::plan`] diffs the
//! two as a pure function, and [`daemon::Daemon`] applies the result.
//!
//! # Why the handshake exists
//!
//! Each agent's secret key is minted on this host and never transmitted. But a
//! NIP-OA auth tag binds one specific agent pubkey and is signed with the
//! *owner's* secret key, so the spawner cannot self-attest and the owner cannot
//! pre-authorize a pubkey that does not exist yet. See [`attestation`].

pub mod attestation;
pub mod config;
pub mod container;
pub mod daemon;
pub mod env;
pub mod reconcile;
pub mod relay;
pub mod store;
