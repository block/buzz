//! The community landing channel: the one open stream every person who
//! enters the community is joined to first, and where its welcome message
//! shows.
//!
//! The designation lives on the `communities` row (see
//! [`buzz_db::landing`]); NIP-11 `landing_channel` is its authoritative read
//! and the `["landing"]` / `["welcome", text]` tags on the channel's
//! kind:39000 are a projection. A relay owner or admin moves it with a
//! kind:9002 `landing` tag. With no designation, the relay adopts or creates
//! `general` here, lazily: at provisioning, at boot for the deployment
//! community, and on every human admission. A NULL designation is the retry
//! record.

use std::sync::Arc;

use nostr::Event;
use tracing::{debug, info, warn};
use uuid::Uuid;

use buzz_core::kind::KIND_MEMBER_ADDED_NOTIFICATION;
use buzz_core::tenant::TenantContext;
use buzz_db::landing::LandingAdoption;

use super::side_effects::{self, SelfJoinOutcome};
use crate::nip_fi_gate::SessionAdmissionGate;
use crate::state::AppState;

/// Refusal for a kind:9002 `landing` or `welcome` tag from an actor who is
/// not a relay owner or admin.
pub(crate) const NOT_AUTHORIZED: &str =
    "actor not authorized: landing and welcome require a relay owner or admin";

/// Validates a kind:9002's `landing` and `welcome` tags. Returns whether the
/// event carries either; the caller then requires a relay owner or admin and
/// a stream channel.
///
/// `["landing", "true"]` designates the channel; there is no way to turn the
/// designation off, only to move it. `["welcome", text]` sets the welcome
/// message and `["welcome", ""]` restores the default. Each may appear once.
pub(crate) fn validate_tags(event: &Event) -> anyhow::Result<bool> {
    let tags = |name: &str| {
        event
            .tags
            .iter()
            .filter(move |tag| tag.as_slice().first().map(String::as_str) == Some(name))
            .map(|tag| tag.as_slice().get(1).map(String::as_str))
            .collect::<Vec<_>>()
    };
    let (landing, welcome) = (tags("landing"), tags("welcome"));
    if landing.len() > 1 || welcome.len() > 1 {
        anyhow::bail!("kind:9002 may include at most one landing and one welcome tag");
    }
    if let Some(value) = landing.first() {
        match value {
            Some("true") => {}
            Some("false") => {
                anyhow::bail!(
                    "landing cannot be turned off; make another channel the landing channel"
                )
            }
            Some(other) => anyhow::bail!("invalid landing value: {other} (must be \"true\")"),
            None => anyhow::bail!("landing tag must have a value"),
        }
    }
    if let Some(value) = welcome.first() {
        match value {
            None => anyhow::bail!(
                "welcome tag must have a value (text, or empty to restore the default)"
            ),
            Some("") => {}
            Some(text) => {
                if let Some(error) = buzz_db::landing::welcome_message_error(text) {
                    anyhow::bail!("{error}");
                }
            }
        }
    }
    Ok(!landing.is_empty() || !welcome.is_empty())
}

/// Makes sure the community has a landing channel (see
/// [`buzz_db::Db::ensure_landing_channel`]) and announces a new designation:
/// the channel's kind:39000 with its `landing` tag, and for a channel the
/// relay created, the owner's membership notice. Returns the landing
/// channel, `None` when the community has none and cannot get one yet.
///
/// The designation is committed before any announcement, so a failed
/// announcement is logged and does not fail the call.
pub(crate) async fn ensure_landing_channel(
    tenant: &TenantContext,
    state: &Arc<AppState>,
) -> anyhow::Result<Option<Uuid>> {
    let adoption = state.db.ensure_landing_channel(tenant.community()).await?;
    if let Some(outcome) = adoption.outcome() {
        metrics::counter!("buzz_landing_adoption_total", "outcome" => outcome).increment(1);
    }
    match adoption {
        LandingAdoption::Existing(_) => {}
        LandingAdoption::Reused(channel_id) => {
            info!(channel = %channel_id, "adopted general as the landing channel");
            state.invalidate_all_accessible_channels(tenant);
            announce_designation(tenant, state, channel_id).await;
        }
        LandingAdoption::Created {
            channel_id,
            steward,
        } => {
            info!(channel = %channel_id, steward = %hex::encode(steward), "created general as the landing channel");
            state.invalidate_membership(tenant, channel_id, &steward);
            state.invalidate_all_accessible_channels(tenant);
            announce_designation(tenant, state, channel_id).await;
            if let Err(e) = side_effects::emit_membership_notification(
                tenant,
                state,
                channel_id,
                &steward,
                state.relay_keypair.public_key().as_bytes(),
                KIND_MEMBER_ADDED_NOTIFICATION,
            )
            .await
            {
                warn!(channel = %channel_id, error = %e, "landing channel owner notification failed");
            }
        }
        LandingAdoption::SkippedNoSteward => {
            debug!("no landing channel: no eligible general and no owner or admin to create one");
        }
        LandingAdoption::SkippedNameTaken => {
            debug!("no landing channel: general is taken by a channel that cannot be the landing channel");
        }
    }
    Ok(adoption.channel_id())
}

async fn announce_designation(tenant: &TenantContext, state: &Arc<AppState>, channel_id: Uuid) {
    if let Err(e) = side_effects::emit_group_discovery_events(tenant, state, channel_id).await {
        warn!(channel = %channel_id, error = %e, "landing channel discovery emission failed");
    }
}

/// Best-effort [`ensure_landing_channel`] for a caller that has nothing to
/// do with the result: provisioning and boot. A failure is logged; the NULL
/// designation retries at the next human admission.
pub async fn converge_landing_channel(tenant: &TenantContext, state: &Arc<AppState>) {
    if let Err(e) = ensure_landing_channel(tenant, state).await {
        metrics::counter!("buzz_landing_adoption_failures_total").increment(1);
        warn!(community = %tenant.host(), error = %e, "landing channel adoption failed");
    }
}

/// The landing join at a roster write (invite claim, kind:9030), as a
/// kind:9021 self-join: an earlier landing membership is restored, as the
/// default channels' are. Returns whether the default channels should
/// follow: `false` only when the join failed in a way a retry could get
/// past, so the person keeps no entry evidence and their next AUTH joins
/// the landing channel and then the defaults
/// ([`admission_joins_on_auth`]). A refusal (a ban, or a designation left
/// unusable) is final, so the defaults follow it.
pub(crate) async fn join_landing_on_roster_admission(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    pubkey: &[u8],
) -> bool {
    let member = hex::encode(pubkey);
    let landing = match ensure_landing_channel(tenant, state).await {
        Ok(Some(landing)) => landing,
        Ok(None) => return true,
        Err(e) => {
            metrics::counter!("buzz_landing_join_failures_total").increment(1);
            warn!(%member, error = %e, "landing join skipped: adopting the landing channel failed");
            return false;
        }
    };
    match side_effects::join_channel_as_self(tenant, state, landing, pubkey).await {
        Ok(SelfJoinOutcome::Joined) => {
            metrics::counter!("buzz_landing_joins_total").increment(1);
            info!(channel = %landing, %member, "joined the landing channel");
            true
        }
        Ok(SelfJoinOutcome::AlreadyMember) => true,
        Err(e) if side_effects::is_self_join_refusal(&e) => {
            metrics::counter!("buzz_landing_join_refused_total").increment(1);
            info!(channel = %landing, %member, error = %e, "landing join refused");
            true
        }
        Err(e) => {
            metrics::counter!("buzz_landing_join_failures_total").increment(1);
            warn!(channel = %landing, %member, error = %e, "landing join failed; default channels wait for the next admission");
            false
        }
    }
}

/// The joins after a person's WebSocket admission, in both community modes.
/// Agents are not joined, so the caller does not spawn this for one.
///
/// 1. Make sure the community has a landing channel.
/// 2. A person with no entry evidence
///    ([`buzz_db::Db::has_entered_community`]) is joined to it. In roster
///    mode the evidence counts from their roster row's `created_at`, so a
///    re-admission after kind:9031 is a first entry again; without a roster
///    any history counts.
/// 3. In roster mode, the admission then finishes the durable
///    default-channel work its roster write left pending, if any
///    ([`side_effects::auto_join_default_channels`]). Without a roster, the
///    default channels they have never been in follow on every admission
///    ([`side_effects::join_never_joined_channels`]).
///
/// Every write holds one of the socket's effect permits, so nothing is
/// joined after the session ends. A leave that commits first stands:
/// `put_first_member_since` writes nothing over a row removed since
/// admission. Failures are logged and counted.
pub(crate) async fn admission_joins_on_auth(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    pubkey: &[u8],
    gate: &Arc<SessionAdmissionGate>,
) {
    admission_joins(tenant, state, pubkey, gate).await;
    #[cfg(test)]
    crate::nip_fi_test_hooks::after_auto_join(tenant.community()).await;
}

async fn admission_joins(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    pubkey: &[u8],
    gate: &Arc<SessionAdmissionGate>,
) {
    let member = hex::encode(pubkey);
    let landing = match ensure_landing_channel(tenant, state).await {
        Ok(landing) => landing,
        Err(e) => {
            metrics::counter!("buzz_landing_join_failures_total").increment(1);
            warn!(%member, error = %e, "admission joins skipped: adopting the landing channel failed");
            return;
        }
    };
    let roster = state.config.require_relay_membership;
    let admitted_at = if roster {
        match state.db.get_relay_member(tenant.community(), &member).await {
            Ok(Some(row)) => Some(row.created_at),
            // Removed from the roster since admission: the session is
            // being closed, so it joins nothing.
            Ok(None) => return,
            Err(e) => {
                metrics::counter!("buzz_landing_join_failures_total").increment(1);
                warn!(%member, error = %e, "admission joins skipped: reading the roster row failed");
                return;
            }
        }
    } else {
        None
    };
    let entered = match state
        .db
        .has_entered_community(tenant.community(), pubkey, admitted_at)
        .await
    {
        Ok(entered) => entered,
        Err(e) => {
            metrics::counter!("buzz_landing_join_failures_total").increment(1);
            warn!(%member, error = %e, "admission joins skipped: reading entry evidence failed");
            return;
        }
    };
    if !entered {
        if let Some(landing) = landing {
            #[cfg(test)]
            crate::nip_fi_test_hooks::before_auto_join_channel(tenant.community()).await;
            let Ok(_permit) = gate.acquire_effect().await else {
                info!(channel = %landing, %member, "landing join stopped: the session ended");
                return;
            };
            match state
                .db
                .put_first_member_since(tenant.community(), landing, pubkey, admitted_at)
                .await
            {
                Ok(None) => {}
                Ok(Some(_)) => {
                    metrics::counter!("buzz_landing_joins_total").increment(1);
                    info!(channel = %landing, %member, "joined the landing channel on admission");
                    if let Err(e) =
                        side_effects::announce_self_join(tenant, state, landing, pubkey).await
                    {
                        warn!(channel = %landing, %member, error = %e, "landing join announcement failed");
                    }
                }
                Err(
                    e @ (buzz_db::DbError::AccessDenied(_) | buzz_db::DbError::ChannelNotFound(_)),
                ) => {
                    metrics::counter!("buzz_landing_join_refused_total").increment(1);
                    info!(channel = %landing, %member, error = %e, "landing join refused");
                }
                Err(e) => {
                    metrics::counter!("buzz_landing_join_failures_total").increment(1);
                    warn!(channel = %landing, %member, error = %e, "landing join on admission failed; default channels wait for the next admission");
                    return;
                }
            }
        }
    }
    if roster {
        side_effects::auto_join_default_channels(tenant, state, &member, Some(gate)).await;
    } else {
        side_effects::join_never_joined_channels(tenant, state, pubkey, gate).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{EventBuilder, Keys, Kind, Tag};

    fn edit(tags: &[&[&str]]) -> Event {
        let tags = tags
            .iter()
            .map(|tag| Tag::parse(tag.iter().copied()).expect("tag"));
        EventBuilder::new(Kind::Custom(9002), "")
            .tags(tags)
            .sign_with_keys(&Keys::generate())
            .expect("sign")
    }

    /// The `landing`/`welcome` rules of §3.3, with the exact receipt text a
    /// client shows.
    #[test]
    fn landing_and_welcome_tag_rules() {
        let long = "a".repeat(buzz_db::landing::WELCOME_MAX_CHARS + 1);
        let cases: &[(&[&[&str]], Result<bool, &str>)] = &[
            (&[&["h", "x"], &["name", "n"]], Ok(false)),
            (&[&["landing", "true"]], Ok(true)),
            (&[&["landing", "true"], &["welcome", ""]], Ok(true)),
            (
                &[&["landing", "true"], &["welcome", "Hi!\nWelcome."]],
                Ok(true),
            ),
            (&[&["welcome", "Hi"]], Ok(true)),
            (
                &[&["landing", "false"]],
                Err("landing cannot be turned off; make another channel the landing channel"),
            ),
            (
                &[&["landing", "yes"]],
                Err("invalid landing value: yes (must be \"true\")"),
            ),
            (&[&["landing"]], Err("landing tag must have a value")),
            (
                &[&["welcome"]],
                Err("welcome tag must have a value (text, or empty to restore the default)"),
            ),
            (
                &[&["landing", "true"], &["landing", "true"]],
                Err("kind:9002 may include at most one landing and one welcome tag"),
            ),
            (
                &[&["welcome", "a"], &["welcome", "b"]],
                Err("kind:9002 may include at most one landing and one welcome tag"),
            ),
            (
                &[&["welcome", "   "]],
                Err(buzz_db::landing::welcome_message_error("   ").expect("blank refused")),
            ),
            (
                &[&["welcome", long.as_str()]],
                Err(buzz_db::landing::welcome_message_error(&long).expect("long refused")),
            ),
            (
                &[&["welcome", "tab\there"]],
                Err(buzz_db::landing::welcome_message_error("tab\there").expect("control refused")),
            ),
        ];
        for (tags, expected) in cases {
            let got = validate_tags(&edit(tags)).map_err(|e| e.to_string());
            assert_eq!(got, expected.map_err(str::to_string), "tags {tags:?}");
        }
    }
}
