//! Send an @mention event to a Buzz channel targeting a specific npub.
//! Usage: mention <channel_uuid> <target_npub> <message>

use buzz_core::nostr_identity::{parse_public_key_compat, public_key_to_npub};
use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, Keys, Kind, Tag};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // rustls needs a CryptoProvider even for plain ws:// connections.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage: mention <channel_uuid> <target_npub> <message>");
        std::process::exit(1);
    }
    let channel_id = &args[1];
    let target_pubkey = parse_public_key_compat(&args[2])?.0.to_hex();
    let message = args[3..].join(" ");

    let url = std::env::var("BUZZ_RELAY_URL").unwrap_or_else(|_| "ws://localhost:3000".into());
    let keys = Keys::generate();
    println!("Sender pubkey: {}", public_key_to_npub(&keys.public_key())?);

    let mut client = BuzzTestClient::connect(&url, &keys).await?;

    let h_tag = Tag::parse(["h", channel_id])?;
    let p_tag = Tag::parse(["p", &target_pubkey])?;
    let event = EventBuilder::new(Kind::Custom(9), message)
        .tags([h_tag, p_tag])
        .sign_with_keys(&keys)?;

    let ok = client.send_event(event).await?;
    if ok.accepted {
        println!("✅ @mention sent: {}", ok.event_id);
    } else {
        eprintln!("❌ Rejected: {}", ok.message);
    }
    Ok(())
}
