//! WebSocket Opus audio relay.
//!
//! Clients connect, authenticate via NIP-42, and join an audio room.
//! Binary frames (Opus) are fanned out to all other room members with
//! a 1-byte `peer_index` prefix so receivers know who is speaking.
//!
//! ```text
//! Client A → WS binary → Room::broadcast_frame → Client B, C, ...
//!                                                  (1-byte peer_index prefix)
//! ```

pub mod handler;
pub mod join;
pub mod mesh;
pub mod room;
pub mod wire;

pub use handler::ws_audio_handler;
pub use room::AudioRoomManager;
