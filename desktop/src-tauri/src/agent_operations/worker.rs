use std::time::Duration;

use chrono::{Duration as ChronoDuration, SecondsFormat, Utc};
use nostr::{Event, Keys};
use tauri::{AppHandle, Manager};
use tokio::sync::OwnedMutexGuard;

use crate::{
    app_state::AppState,
    archive::store::{owner_metric_subscription_created_at, read_agent_metric_window},
    commands::{
        current_managed_agent_summaries, find_channel_message_by_content_key_in_scope,
        find_channel_message_by_marker_in_scope, send_managed_agent_channel_message_internal,
        send_owner_marked_channel_message,
    },
    managed_agents::nest_dir,
    relay::{relay_http_base_url, relay_ws_url_with_override},
};

use super::{
    activity::aggregate_activity,
    calendar, channel_member_pubkeys,
    liveness::{observe, render_pending_batch},
    operations_lock, storage,
    types::{ConfirmedDigest, DigestDelivery, ScopedOperations},
    value_inbox::scan_value_inbox,
};

fn active_scope(state: &AppState) -> Result<(String, String), String> {
    let owner = state.signing_keys()?.public_key().to_hex();
    let relay = buzz_core_pkg::relay::normalize_relay_url(&relay_ws_url_with_override(state))
        .map_err(|error| format!("invalid active relay: {error}"))?;
    Ok((owner, relay))
}

struct BoundScope {
    _guard: OwnedMutexGuard<()>,
    api_base_url: String,
    owner_keys: Keys,
}

async fn lock_active_scope(
    state: &AppState,
    expected_owner: &str,
    expected_relay: &str,
) -> Result<BoundScope, String> {
    let guard = state.workspace_apply_lock.clone().lock_owned().await;
    let (owner, relay) = active_scope(state)?;
    if !owner.eq_ignore_ascii_case(expected_owner) || relay != expected_relay {
        return Err("operations scope changed during scan".to_string());
    }
    let owner_keys = state.signing_keys()?;
    if !owner_keys
        .public_key()
        .to_hex()
        .eq_ignore_ascii_case(expected_owner)
    {
        return Err("operations owner changed during scan".to_string());
    }
    Ok(BoundScope {
        _guard: guard,
        api_base_url: relay_http_base_url(expected_relay),
        owner_keys,
    })
}

async fn load_enabled_scope(
    app: &AppHandle,
    state: &AppState,
) -> Result<Option<ScopedOperations>, String> {
    let (owner, relay) = active_scope(state)?;
    let _guard = operations_lock().lock().await;
    let store = storage::load(app)?;
    Ok(storage::current_scope(&store, &owner, &relay)
        .filter(|scope| scope.config.enabled)
        .cloned())
}

async fn members_are_valid(
    state: &AppState,
    owner: &str,
    relay: &str,
    assistant: &str,
    channel: &str,
) -> Result<(), String> {
    let scope = lock_active_scope(state, owner, relay).await?;
    let members =
        channel_member_pubkeys(state, &scope.api_base_url, &scope.owner_keys, channel).await?;
    if !members
        .iter()
        .any(|pubkey| pubkey.eq_ignore_ascii_case(owner))
        || !members
            .iter()
            .any(|pubkey| pubkey.eq_ignore_ascii_case(assistant))
    {
        return Err("configured operations channel membership is no longer valid".to_string());
    }
    Ok(())
}

async fn find_marker_in_scope(
    state: &AppState,
    owner: &str,
    relay: &str,
    author: Option<&str>,
    channel: &str,
    marker: &str,
) -> Result<Option<Event>, String> {
    let scope = lock_active_scope(state, owner, relay).await?;
    find_channel_message_by_marker_in_scope(
        state,
        author,
        channel,
        marker,
        &scope.api_base_url,
        &scope.owner_keys,
    )
    .await
}

async fn find_content_key_in_scope(
    state: &AppState,
    owner: &str,
    relay: &str,
    author: &str,
    channel: &str,
    content_key: &str,
) -> Result<Option<Event>, String> {
    let scope = lock_active_scope(state, owner, relay).await?;
    find_channel_message_by_content_key_in_scope(
        state,
        author,
        channel,
        content_key,
        &scope.api_base_url,
        &scope.owner_keys,
    )
    .await
}

async fn update_delivery<F>(
    app: &AppHandle,
    owner: &str,
    relay: &str,
    channel: &str,
    assistant: &str,
    update: F,
) -> Result<(), String>
where
    F: FnOnce(&mut super::types::ScopeDeliveryState),
{
    let _guard = operations_lock().lock().await;
    let mut store = storage::load(app)?;
    let Some(scope) = storage::current_scope_mut(&mut store, owner, relay) else {
        return Ok(());
    };
    if !scope.config.enabled
        || scope.config.channel_id.as_deref() != Some(channel)
        || scope.config.assistant_pubkey.as_deref() != Some(assistant)
    {
        return Ok(());
    }
    update(&mut scope.delivery);
    storage::save(app, &mut store)
}

fn build_wake_prompt(
    date: chrono::NaiveDate,
    boundary: chrono::DateTime<Utc>,
    last_digest: Option<&ConfirmedDigest>,
    value_inbox_line: &str,
    activity: &str,
) -> String {
    let window_start = boundary - ChronoDuration::hours(24);
    let last = last_digest
        .map(|digest| {
            let timestamp = chrono::DateTime::from_timestamp(digest.event_created_at, 0)
                .unwrap_or_default()
                .to_rfc3339_opts(SecondsFormat::Secs, true);
            format!("{} at {timestamp}", digest.event_id)
        })
        .unwrap_or_else(|| "None".to_string());
    format!(
        "Operations assistant, post the daily operations digest for Asia/Manila date {date}.\n\
Digest key: buzz-ops-digest:v1:{date}\n\
Scheduled boundary UTC: {}\n\
Agent activity window: [{}, {})\n\
Last confirmed digest: {last}\n\n\
Before posting, search this channel for a message authored by your exact identity containing the digest key. If found, post nothing. Otherwise post exactly one top-level digest containing the key and these sections in order:\n\
1. In progress\n\
2. Blocked on Mohammad\n\
3. Merged since last digest\n\
4. Next actions\n\
{value_inbox_line}\n\
5. Agent activity, last 24h\n\n\
Use Linear for contract, queue, and blocker state; GitHub for merges, SHAs, history, and checks; Buzz for assignments and milestones; the Value Inbox briefs described below for the YouTube line; and the local activity snapshot below for turns, estimated cost, and errors. A complete empty section is None. Inaccessible evidence is Unavailable. Partial or invalid evidence is Incomplete. Do not turn missing evidence into zero and do not let stale canvas or chat text override a newer authoritative source.\n\n\
For the YouTube line, scan only `RESEARCH/VIDEO_<videoId>_<slug>.md` files present at digest time. Include a distinct video when the brief's frontmatter `created` date is from Monday through the digest's Manila date, inclusive. Read its category only from the fenced automation JSON `disposition`, which must be exactly Apply, Retain, or Discard, and require JSON `source.video_id` to match `<videoId>` in the filename. N is all valid distinct videos, including Discard; X is Apply; Y is Retain. The video's publication date never controls inclusion. If the complete scan has no matching briefs, print zeros. If a file is unreadable/malformed, a date is missing/invalid, a source ID mismatches, a disposition is unknown, or duplicate briefs conflict, prefix the line with `Incomplete; known` and report only defensible lower bounds. Duplicate briefs with the same video ID and disposition count once.\n\n\
Local activity snapshot:\n{activity}",
        boundary.to_rfc3339_opts(SecondsFormat::Secs, true),
        window_start.to_rfc3339_opts(SecondsFormat::Secs, true),
        boundary.to_rfc3339_opts(SecondsFormat::Secs, true),
    )
}

async fn scan(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let Some(scope) = load_enabled_scope(app, &state).await? else {
        return Ok(());
    };
    let owner = scope.owner_pubkey.clone();
    let relay = scope.relay_url.clone();
    let channel = scope
        .config
        .channel_id
        .clone()
        .ok_or("enabled operations config has no channel")?;
    let assistant = scope
        .config
        .assistant_pubkey
        .clone()
        .ok_or("enabled operations config has no assistant")?;
    let now = Utc::now();
    let now_secs = now.timestamp();

    let agents = current_managed_agent_summaries(app.clone()).await?;
    update_delivery(app, &owner, &relay, &channel, &assistant, |delivery| {
        let _ = observe(delivery, &agents, &channel, now_secs);
    })
    .await?;

    let subscription = state
        .archive_db
        .with_conn({
            let owner = owner.clone();
            let relay = relay.clone();
            move |connection| owner_metric_subscription_created_at(connection, &owner, &relay)
        })
        .await;
    update_delivery(
        app,
        &owner,
        &relay,
        &channel,
        &assistant,
        |delivery| match subscription {
            Ok(Some(_)) => {
                if delivery.metric_coverage_since.is_none() {
                    delivery.metric_coverage_since = Some(now_secs);
                }
            }
            _ => delivery.metric_coverage_since = None,
        },
    )
    .await?;

    let refreshed = load_enabled_scope(app, &state)
        .await?
        .ok_or("operations scope changed")?;
    for pending in refreshed
        .delivery
        .alert_batches
        .iter()
        .filter(|batch| batch.event_id.is_none())
        .cloned()
        .collect::<Vec<_>>()
    {
        members_are_valid(&state, &owner, &relay, &assistant, &channel).await?;
        let existing = find_marker_in_scope(
            &state,
            &owner,
            &relay,
            Some(&assistant),
            &channel,
            &pending.marker,
        )
        .await?;
        let event_id = if let Some(event) = existing {
            event.id.to_hex()
        } else {
            let content = render_pending_batch(&refreshed.delivery, &pending, &agents);
            let scope = lock_active_scope(&state, &owner, &relay).await?;
            send_managed_agent_channel_message_internal(
                &assistant,
                &channel,
                &content,
                Some(&pending.marker),
                Some("agent"),
                Vec::new(),
                None,
                Vec::new(),
                Some((&scope.api_base_url, &scope.owner_keys)),
                app,
                &state,
            )
            .await?
            .event_id
        };
        update_delivery(app, &owner, &relay, &channel, &assistant, |delivery| {
            if let Some(batch) = delivery
                .alert_batches
                .iter_mut()
                .find(|batch| batch.marker == pending.marker)
            {
                batch.event_id = Some(event_id);
            }
        })
        .await?;
    }

    let Some((date, boundary)) = calendar::eligible_boundary(now) else {
        return Ok(());
    };
    let date_string = date.to_string();
    let digest_key = format!("buzz-ops-digest:v1:{date_string}");
    if let Some(event) =
        find_content_key_in_scope(&state, &owner, &relay, &assistant, &channel, &digest_key).await?
    {
        let confirmed = ConfirmedDigest {
            date: date_string.clone(),
            event_id: event.id.to_hex(),
            event_created_at: event.created_at.as_secs() as i64,
        };
        update_delivery(app, &owner, &relay, &channel, &assistant, |delivery| {
            delivery.confirmed_digest = Some(confirmed);
        })
        .await?;
        return Ok(());
    }

    let refreshed = load_enabled_scope(app, &state)
        .await?
        .ok_or("operations scope changed")?;
    if refreshed
        .delivery
        .digest_wakes
        .iter()
        .any(|wake| wake.date == date_string && wake.event_id.is_some())
    {
        return Ok(());
    }
    let marker = format!("buzz:ops-digest-wake:v1:{date_string}");
    if !refreshed
        .delivery
        .digest_wakes
        .iter()
        .any(|wake| wake.date == date_string)
    {
        update_delivery(app, &owner, &relay, &channel, &assistant, |delivery| {
            delivery.digest_wakes.push(DigestDelivery {
                date: date_string.clone(),
                marker: marker.clone(),
                event_id: None,
                event_created_at: None,
            });
        })
        .await?;
    }
    members_are_valid(&state, &owner, &relay, &assistant, &channel).await?;
    if let Some(existing) =
        find_marker_in_scope(&state, &owner, &relay, Some(&owner), &channel, &marker).await?
    {
        let event_id = existing.id.to_hex();
        let created_at = existing.created_at.as_secs() as i64;
        update_delivery(app, &owner, &relay, &channel, &assistant, |delivery| {
            if let Some(wake) = delivery
                .digest_wakes
                .iter_mut()
                .find(|wake| wake.date == date_string)
            {
                wake.event_id = Some(event_id);
                wake.event_created_at = Some(created_at);
            }
        })
        .await?;
        return Ok(());
    }

    let metric_rows = state
        .archive_db
        .with_conn({
            let owner = owner.clone();
            let relay = relay.clone();
            move |connection| {
                read_agent_metric_window(
                    connection,
                    &owner,
                    &relay,
                    (boundary - ChronoDuration::hours(24)).timestamp(),
                    boundary.timestamp(),
                )
            }
        })
        .await;
    let continuous_coverage = refreshed
        .delivery
        .metric_coverage_since
        .is_some_and(|since| since <= (boundary - ChronoDuration::hours(24)).timestamp());
    let activity = match metric_rows {
        Ok(rows) => aggregate_activity(rows, &agents, continuous_coverage),
        Err(_) => {
            update_delivery(app, &owner, &relay, &channel, &assistant, |delivery| {
                delivery.metric_coverage_since = None;
            })
            .await?;
            "Coverage: Unavailable".to_string()
        }
    };
    let value_line = nest_dir()
        .map(|nest| scan_value_inbox(&nest.join("RESEARCH"), date).line())
        .unwrap_or_else(|| {
            "YouTube Value Inbox: Incomplete; known videos this week 0, applied 0, retained 0"
                .to_string()
        });
    let prompt = build_wake_prompt(
        date,
        boundary,
        refreshed.delivery.confirmed_digest.as_ref(),
        &value_line,
        &activity,
    );
    let scope = lock_active_scope(&state, &owner, &relay).await?;
    let sent = send_owner_marked_channel_message(
        &state,
        &channel,
        &prompt,
        &assistant,
        &marker,
        &scope.api_base_url,
        &scope.owner_keys,
    )
    .await?;
    update_delivery(app, &owner, &relay, &channel, &assistant, |delivery| {
        if let Some(wake) = delivery
            .digest_wakes
            .iter_mut()
            .find(|wake| wake.date == date_string)
        {
            wake.event_id = Some(sent.event_id);
            wake.event_created_at = Some(sent.created_at);
        }
    })
    .await
}

pub(crate) fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(30));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if let Err(error) = scan(&app).await {
                eprintln!("buzz-desktop: operations automation scan failed: {error}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syn79_wake_prompt_has_exact_sections_and_value_inbox_placement() {
        let date = chrono::NaiveDate::from_ymd_opt(2026, 9, 2).unwrap();
        let boundary = calendar::boundary_for_date(date);
        let prompt = build_wake_prompt(
            date,
            boundary,
            None,
            "YouTube Value Inbox: videos this week 2, applied 1, retained 0",
            "Coverage: Incomplete",
        );
        let next_actions = prompt.find("4. Next actions").unwrap();
        let inbox = prompt
            .find("YouTube Value Inbox: videos this week 2")
            .unwrap();
        let activity = prompt.find("5. Agent activity, last 24h").unwrap();
        assert!(next_actions < inbox && inbox < activity);
        assert!(
            prompt.contains("Agent activity window: [2026-09-01T01:00:00Z, 2026-09-02T01:00:00Z)")
        );
    }
}
