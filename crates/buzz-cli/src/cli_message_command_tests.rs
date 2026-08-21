use clap::Parser;

use super::Cli;

#[test]
fn messages_thread_accepts_link_or_explicit_identifiers() {
    let channel = "123e4567-e89b-12d3-a456-426614174000";
    let event = "a".repeat(64);
    let link = format!("buzz://message?channel={channel}&id={event}");

    assert!(Cli::try_parse_from(["buzz", "messages", "thread", "--link", link.as_str(),]).is_ok());
    assert!(Cli::try_parse_from([
        "buzz",
        "messages",
        "thread",
        "--channel",
        channel,
        "--event",
        event.as_str(),
    ])
    .is_ok());
}

#[test]
fn messages_raw_requires_an_event_id_argument() {
    let event = "a".repeat(64);
    assert!(Cli::try_parse_from(["buzz", "messages", "raw", "--event", event.as_str()]).is_ok());
    assert!(Cli::try_parse_from(["buzz", "messages", "raw"]).is_err());
}
