//! `buzz threads title …` — shared thread titles. The convention lives in
//! [`buzz_sdk::thread_title`] so the CLI and desktop write identical artifacts.

use buzz_sdk::thread_title::{
    build_thread_title_revision, is_thread_title_head_conflict, normalize_thread_title,
    parse_thread_title, plan_thread_title, select_thread_title_head, thread_title_current_filter,
    thread_title_history_filter, thread_title_id, thread_title_list_filter, ThreadTitleHead,
    MAX_THREAD_TITLE_WRITE_ATTEMPTS,
};
use uuid::Uuid;

use crate::client::{normalize_write_response, BuzzClient};
use crate::error::CliError;
use crate::validate::{parse_uuid, validate_hex64};

fn parse_events(raw: &str) -> Result<Vec<nostr::Event>, CliError> {
    serde_json::from_str(raw)
        .map_err(|e| CliError::Other(format!("failed to parse artifact query response: {e}")))
}

/// Fetch the current head, including a deleted one (which the `current` view omits).
async fn fetch_head(client: &BuzzClient, root: &str) -> Result<Option<ThreadTitleHead>, CliError> {
    let current = parse_events(&client.query(&thread_title_current_filter(root)).await?)?;
    if let Some(event) = current.first() {
        return Ok(Some(ThreadTitleHead::from_event(event)));
    }
    let history = parse_events(&client.query(&thread_title_history_filter(root)).await?)?;
    Ok(select_thread_title_head(&history))
}

/// Resolve any message in a thread to the channel-checked thread root.
async fn resolve_root(client: &BuzzClient, channel: Uuid, event: &str) -> Result<String, CliError> {
    validate_hex64(event)?;
    let event = event.to_ascii_lowercase();
    let selected = super::messages::fetch_event(client, &event).await?;
    super::messages::resolve_thread_target(channel, &event, None, &selected)
}

fn is_head_conflict(err: &CliError) -> bool {
    matches!(err, CliError::Relay { status: 409, body } if is_thread_title_head_conflict(body))
}

/// `--if-unset` writes only a thread's first title. Any existing head, including
/// a cleared one, means someone already decided, so it is left alone.
fn skip_existing_title(if_unset: bool, head: Option<&ThreadTitleHead>) -> bool {
    if_unset && head.is_some()
}

async fn write_title(
    client: &BuzzClient,
    channel: &str,
    event: &str,
    desired: Option<&str>,
    if_unset: bool,
) -> Result<(), CliError> {
    let channel = parse_uuid(channel)?;
    let root = resolve_root(client, channel, event).await?;
    let id = thread_title_id(&root).to_string();
    for attempt in 1..=MAX_THREAD_TITLE_WRITE_ATTEMPTS {
        let head = fetch_head(client, &root).await?;
        if skip_existing_title(if_unset, head.as_ref()) {
            println!(
                "{}",
                serde_json::json!({
                    "event_id": head.map(|h| h.event_id),
                    "accepted": true,
                    "message": "already titled",
                    "artifact": id,
                    "root": root,
                })
            );
            return Ok(());
        }
        let plan = plan_thread_title(head.as_ref(), desired);
        let Some(builder) = build_thread_title_revision(channel, &root, &plan, desired)
            .map_err(|e| CliError::Other(e.to_string()))?
        else {
            println!(
                "{}",
                serde_json::json!({
                    "event_id": head.map(|h| h.event_id),
                    "accepted": true,
                    "message": "unchanged",
                    "artifact": id,
                    "root": root,
                })
            );
            return Ok(());
        };
        let signed = client.sign_event(builder)?;
        match client.submit_event(signed).await {
            Ok(resp) => {
                let mut out: serde_json::Value =
                    serde_json::from_str(&normalize_write_response(&resp))
                        .unwrap_or_else(|_| serde_json::json!({"raw": resp}));
                out["artifact"] = id.into();
                out["root"] = root.into();
                out["op"] = plan.op().into();
                println!("{out}");
                return Ok(());
            }
            Err(e) if is_head_conflict(&e) && attempt < MAX_THREAD_TITLE_WRITE_ATTEMPTS => continue,
            Err(e) if is_head_conflict(&e) => {
                return Err(CliError::Conflict(format!(
                    "thread title for {root} kept changing during {MAX_THREAD_TITLE_WRITE_ATTEMPTS} attempts; re-run to apply"
                )))
            }
            Err(e) => return Err(e),
        }
    }
    unreachable!("loop returns on its last attempt")
}

async fn cmd_get(client: &BuzzClient, channel: &str, event: &str) -> Result<(), CliError> {
    let channel = parse_uuid(channel)?;
    let root = resolve_root(client, channel, event).await?;
    let current = parse_events(&client.query(&thread_title_current_filter(&root)).await?)?;
    let out = match current.first().and_then(parse_thread_title) {
        Some(title) => serde_json::to_value(title)
            .map_err(|e| CliError::Other(format!("failed to serialize title: {e}")))?,
        None => serde_json::json!({
            "title": null,
            "channel": channel.to_string(),
            "root": root,
            "artifact": thread_title_id(&root).to_string(),
        }),
    };
    println!("{out}");
    Ok(())
}

async fn cmd_list(client: &BuzzClient, channel: Option<&str>, limit: u32) -> Result<(), CliError> {
    let channel = channel.map(parse_uuid).transpose()?;
    let events = parse_events(
        &client
            .query(&thread_title_list_filter(channel, limit))
            .await?,
    )?;
    let titles: Vec<_> = events.iter().filter_map(parse_thread_title).collect();
    println!(
        "{}",
        serde_json::to_string(&titles)
            .map_err(|e| CliError::Other(format!("failed to serialize titles: {e}")))?
    );
    Ok(())
}

pub async fn dispatch(cmd: crate::ThreadsCmd, client: &BuzzClient) -> Result<(), CliError> {
    use crate::{ThreadTitleCmd, ThreadsCmd};
    match cmd {
        ThreadsCmd::Title(sub) => match sub {
            ThreadTitleCmd::Set {
                channel,
                event,
                title,
                if_unset,
            } => {
                let title = normalize_thread_title(&title).map_err(|e| {
                    CliError::Usage(format!("{e} (use `threads title clear` to remove a title)"))
                })?;
                write_title(client, &channel, &event, Some(&title), if_unset).await
            }
            ThreadTitleCmd::Clear { channel, event } => {
                write_title(client, &channel, &event, None, false).await
            }
            ThreadTitleCmd::Get { channel, event } => cmd_get(client, &channel, &event).await,
            ThreadTitleCmd::List { channel, limit } => {
                cmd_list(client, channel.as_deref(), limit).await
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_409_head_conflicts_retry() {
        let relay = |status, body: &str| CliError::Relay {
            status,
            body: body.into(),
        };
        assert!(is_head_conflict(&relay(
            409,
            "conflict: artifact head changed"
        )));
        assert!(is_head_conflict(&relay(
            409,
            "conflict: artifact identity is taken"
        )));
        assert!(!is_head_conflict(&relay(
            409,
            "conflict: artifact home changed"
        )));
        assert!(!is_head_conflict(&relay(400, "artifact head changed")));
    }

    #[test]
    fn if_unset_skips_live_and_cleared_titles_but_not_untitled_threads() {
        let head = |deleted| ThreadTitleHead {
            event_id: "e".repeat(64),
            deleted,
            title: (!deleted).then(|| "Existing".into()),
        };
        assert!(!skip_existing_title(true, None));
        assert!(skip_existing_title(true, Some(&head(false))));
        assert!(skip_existing_title(true, Some(&head(true))));
        assert!(!skip_existing_title(false, Some(&head(false))));
    }
}
