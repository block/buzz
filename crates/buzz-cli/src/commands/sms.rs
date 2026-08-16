//! `buzz sms` — SMS bridge routing.
//!
//! `set-route` is a signed command event (kind 9046) submitted via
//! `POST /events`, mirroring the 9040-series moderation commands: the relay
//! validates, authorizes, and executes it directly — it is never stored, so
//! the subscriber's phone number is not republished to channel subscribers or
//! left queryable.
//!
//! This exists because the sms-operator persona could previously only *ask*
//! "which project?" — nothing anywhere persisted the answer, so a sender was
//! asked forever and seeding a route meant hand-written SQL against the
//! relay's database. The persona can now record a caller's choice itself.
//!
//! Authorization is membership of the relay's configured SMS-inbox channel;
//! the relay owns that check.

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::{OutputFormat, SmsCmd};

async fn cmd_set_route(
    client: &BuzzClient,
    phone: &str,
    project: Option<&str>,
) -> Result<(), CliError> {
    let builder = buzz_sdk::build_sms_set_route(phone, project)
        .map_err(|e| CliError::Usage(format!("invalid sms route: {e}")))?;
    let event = client.sign_event(builder)?;
    let resp = client.submit_event(event).await?;
    println!("{}", normalize_write_response(&resp));
    Ok(())
}

pub async fn dispatch(
    cmd: SmsCmd,
    client: &BuzzClient,
    _format: &OutputFormat,
) -> Result<(), CliError> {
    match cmd {
        SmsCmd::SetRoute { phone, project } => {
            cmd_set_route(client, &phone, Some(project.as_str())).await
        }
        SmsCmd::ClearRoute { phone } => cmd_set_route(client, &phone, None).await,
    }
}
