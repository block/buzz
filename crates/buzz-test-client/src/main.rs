//! `buzz-test-cli` — Manual testing CLI for the Buzz relay.
//!
//! # Usage
//!
//! ```text
//! buzz-test-cli [OPTIONS]
//!
//! Options:
//!   --url <URL>        Relay WebSocket URL [default: ws://localhost:3000]
//!   --send <MESSAGE>   Send a text message to a channel
//!   --channel <ID>     Channel ID for send/subscribe
//!   --subscribe        Subscribe to a channel and print events
//!   --kind <KIND>      Event kind [default: 9]
//! ```
//!
//! # Examples
//!
//! Send a message:
//! ```text
//! buzz-test-cli --channel my-channel --send "Hello, Buzz!"
//! ```
//!
//! Subscribe and watch events:
//! ```text
//! buzz-test-cli --channel my-channel --subscribe
//! ```

use std::time::Duration;

use buzz_test_client::{BuzzTestClient, RelayMessage};
use nostr::{Filter, Keys};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "buzz_test_client=debug".to_string())
                .as_str(),
        )
        .init();

    let args: Vec<String> = std::env::args().collect();
    let opts = parse_args(&args);

    let url = opts.url.as_deref().unwrap_or("ws://localhost:3000");
    let channel = opts.channel.as_deref().unwrap_or("default");
    let kind = opts.kind.unwrap_or(9);

    let keys = match std::env::var("BUZZ_PRIVATE_KEY") {
        Ok(sk) => Keys::parse(&sk).expect("invalid BUZZ_PRIVATE_KEY"),
        Err(_) => Keys::generate(),
    };
    println!("Using pubkey: {}", keys.public_key());

    if opts.subscribe {
        run_subscribe(url, &keys, channel, kind).await;
    } else if let Some(ref msg) = opts.send {
        run_send(url, &keys, channel, msg, kind).await;
    } else {
        eprintln!("No action specified. Use --send <MSG> or --subscribe.");
        eprintln!("Run with --help for usage.");
        std::process::exit(1);
    }
}

async fn run_send(url: &str, keys: &Keys, channel: &str, message: &str, kind: u16) {
    println!("Connecting to {url}...");
    let mut client = match BuzzTestClient::connect(url, keys).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect: {e}");
            std::process::exit(1);
        }
    };

    println!("Sending message to channel {channel}...");
    match client.send_text_message(keys, channel, message, kind).await {
        Ok(ok) if ok.accepted => {
            println!("✅ Event accepted: {}", ok.event_id);
        }
        Ok(ok) => {
            eprintln!("❌ Event rejected: {}", ok.message);
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Error sending event: {e}");
            std::process::exit(1);
        }
    }

    let _ = client.disconnect().await;
}

async fn run_subscribe(url: &str, keys: &Keys, channel: &str, kind: u16) {
    println!("Connecting to {url}...");
    let mut client = match BuzzTestClient::connect(url, keys).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect: {e}");
            std::process::exit(1);
        }
    };

    let sub_id = format!("cli-sub-{}", uuid::Uuid::new_v4());
    let filter = Filter::new().kind(nostr::Kind::Custom(kind)).custom_tags(
        nostr::SingleLetterTag::lowercase(nostr::Alphabet::E),
        [channel],
    );

    println!("Subscribing to channel {channel} (kind {kind})...");
    if let Err(e) = client.subscribe(&sub_id, vec![filter]).await {
        eprintln!("Subscribe failed: {e}");
        std::process::exit(1);
    }

    println!("Listening for events (Ctrl+C to stop)...");
    loop {
        match client.recv_event(Duration::from_secs(30)).await {
            Ok(RelayMessage::Event {
                subscription_id: _,
                event,
            }) => {
                println!(
                    "[{}] kind={} pubkey={} content={}",
                    event.created_at,
                    event.kind.as_u16(),
                    event.pubkey,
                    event.content
                );
            }
            Ok(RelayMessage::Eose { .. }) => {
                println!("(end of stored events — waiting for live events)");
            }
            Ok(RelayMessage::Notice { message }) => {
                println!("NOTICE: {message}");
            }
            Ok(RelayMessage::Closed { message, .. }) => {
                eprintln!("Subscription closed by relay: {message}");
                break;
            }
            Ok(_) => {}
            Err(buzz_test_client::TestClientError::Timeout) => {
                // Keep waiting.
            }
            Err(e) => {
                eprintln!("Error: {e}");
                break;
            }
        }
    }

    let _ = client.disconnect().await;
}

struct CliOpts {
    url: Option<String>,
    send: Option<String>,
    channel: Option<String>,
    subscribe: bool,
    kind: Option<u16>,
}

fn parse_args(args: &[String]) -> CliOpts {
    let mut opts = CliOpts {
        url: None,
        send: None,
        channel: None,
        subscribe: false,
        kind: None,
    };

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--url" => {
                i += 1;
                opts.url = args.get(i).cloned();
            }
            "--send" => {
                i += 1;
                opts.send = args.get(i).cloned();
            }
            "--channel" => {
                i += 1;
                opts.channel = args.get(i).cloned();
            }
            "--subscribe" => {
                opts.subscribe = true;
            }
            "--kind" => {
                i += 1;
                opts.kind = args.get(i).and_then(|s| s.parse().ok());
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown argument: {other}");
                std::process::exit(1);
            }
        }
        i += 1;
    }

    opts
}

fn print_help() {
    println!(
        r#"buzz-test-cli — Manual testing CLI for the Buzz relay

USAGE:
    buzz-test-cli [OPTIONS]

OPTIONS:
    --url <URL>        Relay WebSocket URL [default: ws://localhost:3000]
    --send <MESSAGE>   Send a text message to a channel
    --channel <ID>     Channel ID for send/subscribe [default: default]
    --subscribe        Subscribe to a channel and print events
    --kind <KIND>      Event kind [default: 9]
    --help             Print this help message

EXAMPLES:
    # Send a message to a channel
    buzz-test-cli --channel my-channel --send "Hello, Buzz!"

    # Subscribe and watch live events
    buzz-test-cli --channel my-channel --subscribe

    # Use a different relay URL
    buzz-test-cli --url ws://relay.example.com --channel test --subscribe
"#
    );
}
