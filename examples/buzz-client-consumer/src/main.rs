use std::sync::Arc;

use buzz_client::{
    BuzzClient, ChannelScope, CommunityEndpoint, DeliveryOutcome, ListChannelsRequest,
    MessageKind, SendMessageRequest,
};
use nostr::Keys;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = CommunityEndpoint::parse("http://127.0.0.1:3000")?;
    let signer = Arc::new(Keys::generate());
    let client = BuzzClient::builder(endpoint, signer).build().await?;

    let channels = client
        .list_channels(ListChannelsRequest {
            scope: ChannelScope::Member,
            ..ListChannelsRequest::default()
        })
        .await?;
    if let Some(channel) = channels.first() {
        let sent = client
            .send_message(SendMessageRequest {
                channel_id: channel.channel_id,
                content: "hello from an independent buzz-client consumer".into(),
                kind: MessageKind::Stream,
                reply_to: None,
                broadcast: false,
                attachments: Vec::new(),
                mentions: Vec::new(),
            })
            .await?;
        match sent.delivery {
            DeliveryOutcome::Accepted(accepted) => {
                println!("accepted {}", accepted.event_id);
            }
            DeliveryOutcome::Rejected(rejected) => {
                println!("rejected {}: {}", rejected.event_id, rejected.reason);
            }
            DeliveryOutcome::Unknown(unknown) => {
                println!(
                    "delivery unknown for {}; do not re-sign automatically: {}",
                    unknown.event_id, unknown.reason
                );
            }
        }
    }
    Ok(())
}
