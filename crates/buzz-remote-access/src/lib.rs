//! PWA bridge slice B (docs/pwa-bridge-slice-b-c-design.md).
//!
//! * `tailscale`: detect-only status + a `serve` lifecycle that touches only the one https port this app owns.
//! * `challenge`: one-time, connection-bound challenges signed by the member key (via the user's remote signer).
//! * `server`: loopback-only WebSocket listener; nothing is accepted before a valid signed challenge.
//!
//! The listener never binds anything but 127.0.0.1 and never runs `tailscale funnel`.

pub mod challenge;
pub mod pty;
pub mod server;
pub mod tailscale;

pub use challenge::{Challenge, ChallengeBook, ChallengeError, Purpose};
pub use pty::{PtyBackend, TerminalMode};
pub use server::{RemoteServer, ServerConfig, TerminalBackend, TerminalLaunch};
pub use tailscale::{ServeManager, SystemTailscale, TailscaleCli, TailscaleStatus};
