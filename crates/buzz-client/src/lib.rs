//! Reusable, authenticated operations for one Buzz community.
//!
//! A [`BuzzClient`] is bound to one validated community authority, one
//! asynchronous Nostr signer, and an explicit authentication context. The
//! crate owns Buzz protocol policy; process arguments, environment variables,
//! local file acquisition, output formatting, and exit codes remain concerns
//! of application adapters such as `buzz-cli`.
//!
//! # Read/write session
//!
//! ```no_run
//! use std::sync::Arc;
//! use buzz_client::{
//!     AuthContext, BuzzClient, CommunityEndpoint, DeliveryOutcome,
//!     ListChannelsRequest, MessageKind, SendMessageRequest,
//! };
//! use nostr::Keys;
//!
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! let community = CommunityEndpoint::parse("https://buzz.example")?;
//! let signer = Arc::new(Keys::generate());
//! let client = BuzzClient::builder(community, signer)
//!     .auth_context(AuthContext::signer_only())
//!     .build()
//!     .await?;
//!
//! let channels = client
//!     .list_channels(ListChannelsRequest::default())
//!     .await?;
//! if let Some(channel) = channels.first() {
//!     let result = client
//!         .send_message(SendMessageRequest {
//!             channel_id: channel.channel_id,
//!             content: "hello from another client".into(),
//!             kind: MessageKind::Stream,
//!             reply_to: None,
//!             broadcast: false,
//!             attachments: Vec::new(),
//!             mentions: Vec::new(),
//!         })
//!         .await?;
//!     match result.delivery {
//!         DeliveryOutcome::Accepted(delivery) => {
//!             println!("accepted {}", delivery.event_id);
//!         }
//!         DeliveryOutcome::Rejected(delivery) => {
//!             eprintln!("rejected: {}", delivery.reason);
//!         }
//!         DeliveryOutcome::Unknown(delivery) => {
//!             // Keep this event ID for reconciliation. Do not automatically
//!             // create and sign a replacement event.
//!             eprintln!("delivery of {} is unknown", delivery.event_id);
//!         }
//!     }
//! }
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

mod auth;
mod channels;
mod client;
mod config;
mod endpoint;
mod error;
mod messages;
mod transport;

pub use auth::AuthContext;
pub use channels::{ChannelRecord, ChannelScope, ChannelVisibility, ListChannelsRequest};
pub use client::{BuzzClient, BuzzClientBuilder};
pub use config::{ClientConfig, RetryPolicy};
pub use endpoint::{CommunityEndpoint, EndpointError};
pub use error::{
    AcceptedDelivery, ClientError, DeliveryOutcome, DeliveryUnknown, RejectedDelivery,
};
pub use messages::{MessageAttachment, MessageKind, SendMessageRequest, SendMessageResult};
