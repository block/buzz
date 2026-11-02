//! Broker for the co-founder customer record (kind 47069, head kind 30679).
//!
//! One relay-signed head per business, keyed by the party id that its prospect,
//! proposals and client share, so a lead that becomes a client keeps one record
//! and its whole history. The relay resolves who the caller is, whether
//! co-founder mode is on and which slots the caller leads, then hands all of it
//! to `buzz_core::company_customers`, which owns every rule. **No fact is ever
//! taken from a caller**: the timeline, the Money panel and the Chats panel are
//! rebuilt from the company's own records by `crate::customer_inputs`. A caller
//! adds a milestone, a copied email, a file reference or a conversion, nothing
//! else.
//!
//! The relay also writes and refreshes records by itself
//! ([`ensure_customer_records`], called from the wake sweep), so a lead found
//! by Discovery and a client created directly both have a record without anyone
//! asking for one.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::Utc;
use nostr::Event;

use buzz_core::company_customers::{
    apply_customer_action, compute_customer_record, customer_d_tag, customer_refresh_is_fresh,
    parse_customer_d_tag, validate_customer_action, validate_customer_head, CustomerAction,
    CustomerChange, CustomerOp, CustomerRecordHead, ManualState,
};
use buzz_core::company_plans::{PlanActor, PlanActorRole, PlanContext};
use buzz_core::kind::{KIND_CUSTOMER_ACTION, KIND_CUSTOMER_HEAD};
use buzz_core::tenant::{CommunityId, TenantContext};
use buzz_core::StoredEvent;
use buzz_db::replaceable::{ParameterizedReplacePrecondition, ParameterizedReplaceStatus};
use buzz_db::EventQuery;

use super::company_cofounder::{
    check_expected_head, command_d_tag, conflict, duplicate_result, forbidden, internal, invalid,
    is_admin, relay_head_event,
};
use super::company_initiative::{cofounder_settings, commit_head, lock_current_head, HeadWrite};
use super::company_plans::{is_managed_agent, led_functions, map_error};
use super::ingest::{IngestAuth, IngestError, IngestResult};
use crate::state::AppState;

/// Most customer heads one read returns.
const MAX_CUSTOMER_HEADS: i64 = 100_000;
/// Most heads one sweep writes for one company; the rest follow on the next sweep.
const MAX_SWEEP_WRITES: u32 = 100;

/// Handles a customer record write (kind 47069).
pub(super) async fn handle(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: Event,
    auth: IngestAuth,
) -> Result<IngestResult, IngestError> {
    if u32::from(event.kind.as_u16()) != KIND_CUSTOMER_ACTION {
        return Err(invalid("customer action has the wrong event kind"));
    }
    let action = serde_json::from_str::<CustomerAction>(&event.content)
        .map_err(|error| invalid(format!("customer action: {error}")))?;
    validate_customer_action(&action)
        .map_err(|error| invalid(format!("customer action: {error}")))?;
    if event.verify().is_err() {
        return Err(invalid("customer action signature is invalid"));
    }
    if event.pubkey.to_hex() != auth.pubkey().to_hex() {
        return Err(forbidden(
            "event signer does not match the authenticated member",
        ));
    }
    if auth.channel_ids().is_some() {
        return Err(forbidden(
            "channel-scoped authentication cannot write customer records",
        ));
    }
    let d_tag = command_d_tag(&event, "customer")?;
    let expected_d_tag = action
        .d_tag()
        .map_err(|error| invalid(format!("customer action: {error}")))?;
    if d_tag != expected_d_tag {
        return Err(invalid("customer command d tag must match its record"));
    }

    apply(tenant, state, &event, auth.pubkey(), action, d_tag, true).await
}

/// Runs one validated customer action as `actor`: role, authorisation, the record rebuild and
/// the atomic commit. `event` is stored as the action. `fenced` makes the action name the head
/// it saw; the Outreach desk's "call booked" passes `false`, because appending a milestone does
/// not depend on what the head looked like.
async fn apply(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: &Event,
    actor: &nostr::PublicKey,
    action: CustomerAction,
    d_tag: String,
    fenced: bool,
) -> Result<IngestResult, IngestError> {
    let community_id = tenant.community();
    let actor_pubkey = actor.to_hex();
    let actor_managed = is_managed_agent(state, community_id, &actor.to_bytes()).await?;

    let mut tx = state
        .db
        .begin_event_write_transaction()
        .await
        .map_err(internal)?;
    buzz_deletion::store(&state.db)
        .guard_transaction(&mut tx, community_id)
        .await
        .map_err(|error| {
            IngestError::Rejected(format!("restricted: community writes are fenced: {error}"))
        })?;
    let role = sqlx::query_scalar::<_, String>(
        "SELECT role FROM relay_members WHERE community_id = $1 AND pubkey = $2 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(&actor_pubkey)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?;
    if role.is_none() && !actor_managed {
        return Err(forbidden("actor is not a member or managed employee"));
    }
    // A managed agent never counts as an owner or admin, whatever its role row says.
    let actor_role = if actor_managed {
        PlanActorRole::ManagedAgent
    } else if role.as_deref().is_some_and(is_admin) {
        PlanActorRole::Admin
    } else {
        PlanActorRole::Member
    };

    let command_id = event.id.to_bytes();
    let already_stored = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM events WHERE community_id = $1 AND id = $2)",
    )
    .bind(community_id.as_uuid())
    .bind(command_id.as_slice())
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    if already_stored {
        return Ok(duplicate_result(event));
    }

    let (current_head_id, current_stored) = lock_current_head(
        state,
        &mut tx,
        community_id,
        KIND_CUSTOMER_HEAD,
        &d_tag,
        &format!("company-customers:{d_tag}"),
    )
    .await?;
    let actor = PlanActor {
        pubkey: actor_pubkey.clone(),
        role: actor_role,
        led_functions: led_functions(state, community_id, &actor_pubkey).await?,
    };
    let now = Utc::now().timestamp();
    let context = PlanContext {
        now,
        cofounder_enabled: cofounder_settings(state, community_id).await?.enabled,
        action_event_id: event.id.to_hex(),
    };

    // Who may write is settled before any record is read, so a stranger learns
    // nothing about which businesses exist.
    buzz_core::company_customers::authorize_customer_action(&action, &actor, &context)
        .map_err(map_error)?;

    // Every write names the head it saw once a head exists (the same fence as numbers).
    if fenced && (current_stored.is_some() || action.expected_head_event_id.is_some()) {
        check_expected_head(
            "customer record",
            action.expected_head_event_id.as_deref(),
            current_head_id.as_deref(),
        )?;
    }
    let current = current_stored.as_ref().map(parse_head).transpose()?;

    let reads = crate::customer_inputs::gather_facts(state, community_id)
        .await
        .map_err(internal)?;
    let Some(source) = reads.source_for(&action.party_id) else {
        return Err(conflict(
            "no customer has that party id: there is no prospect or client record for it",
        ));
    };
    let manual = current
        .as_ref()
        .map(ManualState::from_head)
        .unwrap_or_default();
    let change = apply_customer_action(
        &manual,
        !source.clients.is_empty(),
        &action,
        &actor,
        &context,
    )
    .map_err(map_error)?;
    let next_manual = match change {
        CustomerChange::Unchanged(message) => {
            return Ok(IngestResult {
                event_id: event.id.to_hex(),
                accepted: true,
                message: message.into(),
            });
        }
        CustomerChange::Refresh => {
            if let Some(head) = &current {
                if customer_refresh_is_fresh(head, now) {
                    // Fresh enough: nothing to recompute, nothing stored.
                    return Ok(IngestResult {
                        event_id: event.id.to_hex(),
                        accepted: true,
                        message: "customer record is fresh".into(),
                    });
                }
            }
            manual
        }
        CustomerChange::Changed(next) => next,
    };

    let head = compute_customer_record(
        &source,
        &next_manual,
        now,
        &actor_pubkey,
        Some(&context.action_event_id),
    );
    if action.op == CustomerOp::Refresh {
        if let Some(current) = &current {
            if current.same_content(&head) {
                return Ok(IngestResult {
                    event_id: event.id.to_hex(),
                    accepted: true,
                    message: "customer record is up to date".into(),
                });
            }
        }
    }
    validate_customer_head(&head).map_err(|error| invalid(format!("customer head: {error}")))?;
    let head_json = serde_json::to_string(&head).map_err(internal)?;

    commit_head(
        tenant,
        state,
        tx,
        event,
        &actor_pubkey,
        HeadWrite {
            head_kind: KIND_CUSTOMER_HEAD,
            action_kind: KIND_CUSTOMER_ACTION,
            d_tag: &d_tag,
            head_json,
            current_head_id,
            current_stored,
            label: "customer record",
        },
    )
    .await
}

/// What happened when the Outreach desk asked for a "call booked" milestone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CallBookedOutcome {
    /// The milestone is on the record now (or already was).
    Recorded,
    /// The caller may not write the customer record, so only the desk stage changed.
    NotAllowed,
}

/// Append a `CallBooked` milestone to the customer record of `party_id` on behalf of `actor`,
/// the person who marked the call booked on the desk. The caller has already seen that a record
/// exists. The time is the desk command's own time, so a retry of the same command finds the
/// milestone already there and changes nothing. Only an authorisation refusal is absorbed
/// (`NotAllowed`); every other failure is returned so the command can be retried.
pub(crate) async fn record_call_booked(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    command: &Event,
    actor: &nostr::PublicKey,
    party_id: &str,
) -> Result<CallBookedOutcome, IngestError> {
    let at = i64::try_from(command.created_at.as_secs()).unwrap_or_default();
    let action = CustomerAction {
        schema_version: buzz_core::company_records::COMPANY_RECORD_SCHEMA_VERSION,
        party_id: party_id.to_owned(),
        op: CustomerOp::RecordMilestone,
        milestone_kind: Some(buzz_core::company_customers::MilestoneKind::CallBooked),
        at: Some(at),
        title: None,
        note: Some("Marked as call booked on the Outreach desk".to_owned()),
        file_ref: None,
        from: None,
        subject: None,
        date: None,
        summary: None,
        file_refs: None,
        label: None,
        attachment_id: None,
        expected_head_event_id: None,
    };
    validate_customer_action(&action)
        .map_err(|error| invalid(format!("customer action: {error}")))?;
    let d_tag = action
        .d_tag()
        .map_err(|error| invalid(format!("customer action: {error}")))?;
    let content = serde_json::to_string(&action).map_err(internal)?;
    // Relay-signed, and named by the desk command, so the same command always yields the same
    // action event id: a retry is a duplicate, not a second milestone.
    let origin = nostr::Tag::parse(["e", &command.id.to_hex()]).map_err(internal)?;
    let d = nostr::Tag::parse(["d", d_tag.as_str()]).map_err(internal)?;
    let event = nostr::EventBuilder::new(nostr::Kind::Custom(KIND_CUSTOMER_ACTION as u16), content)
        .tags([d, origin])
        .custom_created_at(command.created_at)
        .sign_with_keys(&state.relay_keypair)
        .map_err(internal)?;
    match apply(tenant, state, &event, actor, action, d_tag, false).await {
        Ok(_) => Ok(CallBookedOutcome::Recorded),
        // A refusal of the caller (not a member, not a lead of Sales, Scout or an admin). A fenced
        // community is a failure to retry, not a refusal.
        Err(IngestError::Rejected(reason))
            if reason.starts_with("restricted:") && !reason.contains("fenced") =>
        {
            Ok(CallBookedOutcome::NotAllowed)
        }
        Err(error) => Err(error),
    }
}

fn parse_head(stored: &StoredEvent) -> Result<CustomerRecordHead, IngestError> {
    let head = serde_json::from_str::<CustomerRecordHead>(&stored.event.content)
        .map_err(|error| internal(format!("stored customer head is invalid: {error}")))?;
    validate_customer_head(&head)
        .map_err(|error| internal(format!("stored customer head: {error}")))?;
    Ok(head)
}

fn d_tag_of(event: &Event) -> Option<String> {
    event
        .tags
        .iter()
        .find(|tag| tag.kind().to_string() == "d")
        .and_then(|tag| tag.content())
        .map(str::to_owned)
}

/// Every customer head of the company, by d-tag. A head that does not name a
/// party is skipped.
async fn customer_heads(
    state: &AppState,
    community_id: CommunityId,
) -> Result<HashMap<String, StoredEvent>, String> {
    let mut query = EventQuery::for_community(community_id);
    query.kinds = Some(vec![KIND_CUSTOMER_HEAD as i32]);
    query.pubkey = Some(state.relay_keypair.public_key().to_bytes().to_vec());
    query.global_only = true;
    query.limit = Some(MAX_CUSTOMER_HEADS);
    query.max_limit = Some(MAX_CUSTOMER_HEADS);
    let rows = state
        .db
        .query_events_for_event_write(&query)
        .await
        .map_err(|error| format!("could not read the customer records: {error}"))?;
    Ok(rows
        .into_iter()
        .filter_map(|stored| {
            let d_tag = d_tag_of(&stored.event)?;
            parse_customer_d_tag(&d_tag).ok()?;
            Some((d_tag, stored))
        })
        .collect())
}

/// Write or refresh the relay's own copy of one record. Returns `false` when
/// another writer changed the record meanwhile; the next sweep tries again.
async fn write_relay_head(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    d_tag: &str,
    head_json: String,
    expected: Option<&StoredEvent>,
) -> Result<bool, String> {
    let community_id = tenant.community();
    let failed = |error: IngestError| format!("could not write the customer record: {error:?}");
    let mut tx = state
        .db
        .begin_event_write_transaction()
        .await
        .map_err(|error| failed(internal(error)))?;
    buzz_deletion::store(&state.db)
        .guard_transaction(&mut tx, community_id)
        .await
        .map_err(|error| format!("community writes are fenced: {error}"))?;
    let (current_head_id, current) = lock_current_head(
        state,
        &mut tx,
        community_id,
        KIND_CUSTOMER_HEAD,
        d_tag,
        &format!("company-customers:{d_tag}"),
    )
    .await
    .map_err(failed)?;
    let expected_id = expected.map(|stored| stored.event.id.to_bytes().to_vec());
    if current_head_id != expected_id {
        // Another writer got there first.
        return Ok(false);
    }
    let head_event = relay_head_event(
        state,
        KIND_CUSTOMER_HEAD,
        d_tag,
        head_json,
        current.as_ref(),
    )
    .map_err(failed)?;
    let precondition = current_head_id.as_deref().map_or(
        ParameterizedReplacePrecondition::CreateOnly,
        ParameterizedReplacePrecondition::ExpectedRevision,
    );
    let replaced = state
        .db
        .replace_parameterized_event_in_transaction(
            &mut tx,
            community_id,
            &head_event,
            d_tag,
            None,
            precondition,
        )
        .await
        .map_err(|error| failed(internal(error)))?;
    if replaced.status != ParameterizedReplaceStatus::Inserted {
        return Ok(false);
    }
    tx.commit().await.map_err(|error| failed(internal(error)))?;
    super::event::dispatch_persistent_event(
        tenant,
        state,
        &replaced.event,
        KIND_CUSTOMER_HEAD,
        &state.relay_keypair.public_key().to_hex(),
        None,
    )
    .await;
    Ok(true)
}

/// Give every business that has a prospect or a client record a customer record,
/// and bring each existing record up to date with the company's records. A
/// record whose content did not change is not rewritten. Returns how many heads
/// this call wrote; at most [`MAX_SWEEP_WRITES`], the rest follow on the next
/// sweep. A record that could not be written is retried by the next sweep.
pub(crate) async fn ensure_customer_records(
    state: &Arc<AppState>,
    community_id: CommunityId,
    now: i64,
) -> Result<u32, String> {
    let reads = crate::customer_inputs::gather_facts(state, community_id).await?;
    let existing = customer_heads(state, community_id).await?;
    let host = state
        .db
        .lookup_community_host(community_id)
        .await
        .map_err(|error| format!("community host lookup failed: {error}"))?
        .ok_or_else(|| "community no longer exists".to_string())?;
    let tenant = TenantContext::resolved(community_id, host);
    let relay_pubkey = state.relay_keypair.public_key().to_hex();

    let mut written = 0_u32;
    let mut failures = Vec::new();
    for party_id in reads.party_ids() {
        if written >= MAX_SWEEP_WRITES {
            break;
        }
        let Some(source) = reads.source_for(&party_id) else {
            continue;
        };
        let d_tag = customer_d_tag(&party_id).map_err(|error| error.to_string())?;
        let current = existing.get(&d_tag);
        let previous = current.and_then(|stored| parse_head(stored).ok());
        let manual = previous
            .as_ref()
            .map(ManualState::from_head)
            .unwrap_or_default();
        let head = compute_customer_record(&source, &manual, now, &relay_pubkey, None);
        if previous
            .as_ref()
            .is_some_and(|previous| previous.same_content(&head))
        {
            continue;
        }
        if let Err(error) = validate_customer_head(&head) {
            failures.push(format!("{party_id}: {error}"));
            continue;
        }
        let head_json = match serde_json::to_string(&head) {
            Ok(json) => json,
            Err(error) => {
                failures.push(format!("{party_id}: {error}"));
                continue;
            }
        };
        match write_relay_head(&tenant, state, &d_tag, head_json, current).await {
            Ok(true) => written += 1,
            Ok(false) => {}
            Err(error) => failures.push(format!("{party_id}: {error}")),
        }
    }
    if failures.is_empty() {
        Ok(written)
    } else {
        Err(format!(
            "{} customer record(s) could not be written, the next sweep retries: {}",
            failures.len(),
            failures.join("; ")
        ))
    }
}

#[cfg(test)]
mod tests {
    // The `postgres_tests` path puts these in the PostgreSQL CI lane, which gives
    // each test its own database.
    mod postgres_tests {
        use buzz_core::business_records::{client_d_tag, prospect_d_tag};
        use buzz_core::business_records::{ClientHead, ProspectHead};
        use buzz_core::company_customers::{CustomerStage, MilestoneKind, TimelineKind};
        use buzz_core::company_records::COMPANY_RECORD_SCHEMA_VERSION;
        use buzz_core::kind::{KIND_CLIENT_HEAD, KIND_PROSPECT_HEAD};

        use super::super::super::company_initiative::load_head;
        use super::super::super::company_initiative::test_support::{
            add_agent, add_member, auth, enable_cofounder, fixture, rejection, Fixture,
        };
        use super::super::*;

        const PARTY: &str = "00000000-0000-0000-0000-000000000064";
        const CLIENT: &str = "00000000-0000-0000-0000-0000000000c8";
        const PROSPECT: &str = "00000000-0000-0000-0000-000000000001";
        const HEX: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

        fn base(op: CustomerOp) -> CustomerAction {
            CustomerAction {
                schema_version: COMPANY_RECORD_SCHEMA_VERSION,
                party_id: PARTY.into(),
                op,
                milestone_kind: None,
                at: None,
                title: None,
                note: None,
                file_ref: None,
                from: None,
                subject: None,
                date: None,
                summary: None,
                file_refs: None,
                label: None,
                attachment_id: None,
                expected_head_event_id: None,
            }
        }

        async fn send(
            fixture: &Fixture,
            keys: &nostr::Keys,
            action: &CustomerAction,
        ) -> Result<IngestResult, IngestError> {
            let event = buzz_sdk::company_customers::build_customer_action(action)
                .expect("build customer action")
                .sign_with_keys(keys)
                .expect("sign customer action");
            handle(&fixture.tenant, &fixture.state, event, auth(keys)).await
        }

        async fn head_of(fixture: &Fixture) -> Option<StoredEvent> {
            load_head(
                &fixture.state,
                fixture.tenant.community(),
                KIND_CUSTOMER_HEAD,
                &format!("customer:{PARTY}"),
            )
            .await
            .expect("query customer head")
        }

        /// Writes a relay-signed record the way the business broker would.
        async fn seed_relay_head(fixture: &Fixture, kind: u32, d_tag: &str, content: String) {
            let community = fixture.tenant.community();
            let event = relay_head_event(&fixture.state, kind, d_tag, content, None)
                .expect("relay head event");
            let mut tx = fixture
                .state
                .db
                .begin_event_write_transaction()
                .await
                .expect("begin");
            fixture
                .state
                .db
                .replace_parameterized_event_in_transaction(
                    &mut tx,
                    community,
                    &event,
                    d_tag,
                    None,
                    ParameterizedReplacePrecondition::CreateOnly,
                )
                .await
                .expect("seed head");
            tx.commit().await.expect("commit");
        }

        async fn seed_client(fixture: &Fixture) {
            let head = ClientHead {
                schema_version: 1,
                client_id: CLIENT.parse().unwrap(),
                party_id: PARTY.parse().unwrap(),
                display_name: "Plane Tree Guesthouse".into(),
                approver_pubkeys: Vec::new(),
                status: "active".into(),
                source_action_event_id: HEX.into(),
            };
            seed_relay_head(
                fixture,
                KIND_CLIENT_HEAD,
                &client_d_tag(CLIENT.parse().unwrap(), "client", CLIENT.parse().unwrap()),
                serde_json::to_string(&head).unwrap(),
            )
            .await;
        }

        async fn seed_prospect(fixture: &Fixture) {
            let head = serde_json::from_value::<ProspectHead>(serde_json::json!({
                "schemaVersion": 1,
                "prospectId": PROSPECT,
                "status": "active",
                "prospect": {
                    "prospectId": PROSPECT,
                    "party": {
                        "partyId": PARTY,
                        "partyType": "organization",
                        "displayName": "Plane Tree Guesthouse",
                        "externalIds": []
                    },
                    "industry": "Hospitality",
                    "vertical": "guesthouses",
                    "origin": "discovery",
                    "website": null,
                    "location": "Parkhurst",
                    "email": null,
                    "phone": null,
                    "evidence": [],
                    "lastVerifiedAt": null,
                    "qualification": "qualified",
                    "saved": true,
                    "stage": "qualified",
                    "lostReason": null
                },
                "activities": [],
                "sourceActionEventId": HEX
            }))
            .expect("prospect head");
            seed_relay_head(
                fixture,
                KIND_PROSPECT_HEAD,
                &prospect_d_tag(
                    *fixture.tenant.community().as_uuid(),
                    PROSPECT.parse().unwrap(),
                ),
                serde_json::to_string(&head).unwrap(),
            )
            .await;
        }

        async fn company() -> (Fixture, nostr::Keys, nostr::Keys) {
            let fixture = fixture().await;
            let owner = nostr::Keys::generate();
            let scout = nostr::Keys::generate();
            add_member(&fixture, &owner, "owner").await;
            add_agent(&fixture, &owner, &scout).await;
            enable_cofounder(&fixture, &owner).await;
            (fixture, owner, scout)
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn a_stranger_and_a_plain_member_cannot_write_a_record() {
            let (fixture, _owner, _scout) = company().await;
            seed_client(&fixture).await;
            // Not a member and not a managed agent at all.
            let stranger = nostr::Keys::generate();
            let message = rejection(send(&fixture, &stranger, &base(CustomerOp::Refresh)).await);
            assert!(message.contains("not a member"), "{message}");
            // A member with no Sales or Customer Success slot.
            let member = nostr::Keys::generate();
            add_member(&fixture, &member, "member").await;
            for op in [CustomerOp::Refresh, CustomerOp::ConvertToClient] {
                let message = rejection(send(&fixture, &member, &base(op)).await);
                assert!(message.contains("only the owner"), "{op:?}: {message}");
            }
            assert!(
                head_of(&fixture).await.is_none(),
                "a refused write stores no record"
            );
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn co_founder_mode_off_stops_writes() {
            let fixture = fixture().await;
            let owner = nostr::Keys::generate();
            add_member(&fixture, &owner, "owner").await;
            seed_client(&fixture).await;
            let message = rejection(send(&fixture, &owner, &base(CustomerOp::Refresh)).await);
            assert!(message.contains("co-founder mode is off"), "{message}");
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn an_unknown_party_has_no_record() {
            let (fixture, _owner, scout) = company().await;
            let message = rejection(send(&fixture, &scout, &base(CustomerOp::Refresh)).await);
            assert!(message.contains("no customer"), "{message}");
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn a_client_created_directly_gets_a_relay_signed_record_and_refresh_is_throttled() {
            let (fixture, _owner, scout) = company().await;
            seed_client(&fixture).await;
            send(&fixture, &scout, &base(CustomerOp::Refresh))
                .await
                .expect("first refresh creates the record");
            let first = head_of(&fixture).await.expect("customer head");
            assert_eq!(first.event.pubkey, fixture.state.relay_keypair.public_key());
            assert!(first.channel_id.is_none());
            let head: CustomerRecordHead = serde_json::from_str(&first.event.content).unwrap();
            assert_eq!(head.stage, CustomerStage::Client);
            assert_eq!(head.lead_id, None);
            assert_eq!(head.party_id, PARTY);
            assert_eq!(head.timeline.len(), 1);
            assert_eq!(head.timeline[0].kind, TimelineKind::Converted);

            // The same action signed in the same second has the same event id and
            // reads as a duplicate, so let the clock move on before the retry.
            tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
            let again = send(
                &fixture,
                &scout,
                &CustomerAction {
                    expected_head_event_id: Some(first.event.id.to_hex()),
                    ..base(CustomerOp::Refresh)
                },
            )
            .await
            .expect("second refresh");
            assert_eq!(again.message, "customer record is fresh");
            assert_eq!(head_of(&fixture).await.unwrap().event.id, first.event.id);
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn a_milestone_is_fenced_by_the_head_and_lands_on_the_timeline() {
            let (fixture, _owner, scout) = company().await;
            seed_client(&fixture).await;
            send(&fixture, &scout, &base(CustomerOp::Refresh))
                .await
                .expect("create the record");
            let first = head_of(&fixture).await.expect("customer head");

            let call = CustomerAction {
                milestone_kind: Some(MilestoneKind::CallBooked),
                at: Some(Utc::now().timestamp() - 60),
                note: Some("Google Calendar, Fri 30 Oct 14:00".into()),
                expected_head_event_id: Some(first.event.id.to_hex()),
                ..base(CustomerOp::RecordMilestone)
            };
            send(&fixture, &scout, &call)
                .await
                .expect("record the call");
            let second = head_of(&fixture).await.expect("updated head");
            assert_ne!(second.event.id, first.event.id);
            let head: CustomerRecordHead = serde_json::from_str(&second.event.content).unwrap();
            assert!(head
                .timeline
                .iter()
                .any(|entry| entry.kind == TimelineKind::CallBooked));
            assert_eq!(head.milestones.len(), 1);

            // The first head is stale now: a different milestone naming it conflicts.
            let stale = CustomerAction {
                note: Some("Another note".into()),
                at: Some(Utc::now().timestamp() - 30),
                ..call
            };
            let message = rejection(send(&fixture, &scout, &stale).await);
            assert!(
                message.contains(&format!(
                    "customer record changed; current head is {}",
                    second.event.id.to_hex()
                )),
                "{message}"
            );
            assert_eq!(head_of(&fixture).await.unwrap().event.id, second.event.id);
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn converting_a_lead_keeps_its_history_and_twice_is_a_no_op() {
            let (fixture, owner, scout) = company().await;
            seed_prospect(&fixture).await;
            send(&fixture, &scout, &base(CustomerOp::Refresh))
                .await
                .expect("create the lead's record");
            let lead = head_of(&fixture).await.expect("lead head");
            let before: CustomerRecordHead = serde_json::from_str(&lead.event.content).unwrap();
            assert_eq!(before.stage, CustomerStage::Lead);

            // Scout is not the Sales lead, so it cannot convert.
            let convert = CustomerAction {
                expected_head_event_id: Some(lead.event.id.to_hex()),
                ..base(CustomerOp::ConvertToClient)
            };
            let message = rejection(send(&fixture, &scout, &convert).await);
            assert!(message.contains("Sales lead"), "{message}");

            send(&fixture, &owner, &convert)
                .await
                .expect("owner converts");
            let converted = head_of(&fixture).await.expect("client head");
            let after: CustomerRecordHead = serde_json::from_str(&converted.event.content).unwrap();
            assert_eq!(after.stage, CustomerStage::Client);
            assert_eq!(after.party_id, before.party_id);
            assert_eq!(after.lead_id, before.lead_id);
            for entry in &before.timeline {
                assert!(after.timeline.contains(entry), "dropped {}", entry.entry_id);
            }

            tokio::time::sleep(std::time::Duration::from_millis(1_100)).await;
            let again = send(
                &fixture,
                &owner,
                &CustomerAction {
                    expected_head_event_id: Some(converted.event.id.to_hex()),
                    ..base(CustomerOp::ConvertToClient)
                },
            )
            .await
            .expect("second conversion");
            assert_eq!(again.message, "already a client");
            assert_eq!(
                head_of(&fixture).await.unwrap().event.id,
                converted.event.id,
                "converting twice writes nothing"
            );
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn the_sweep_writes_each_record_once_and_leaves_unchanged_ones_alone() {
            let (fixture, _owner, _scout) = company().await;
            let community = fixture.tenant.community();
            seed_prospect(&fixture).await;
            let now = Utc::now().timestamp();
            assert_eq!(
                ensure_customer_records(&fixture.state, community, now)
                    .await
                    .unwrap(),
                1
            );
            let first = head_of(&fixture).await.expect("swept record");
            let head: CustomerRecordHead = serde_json::from_str(&first.event.content).unwrap();
            assert_eq!(head.stage, CustomerStage::Lead);
            assert_eq!(
                ensure_customer_records(&fixture.state, community, now + 600)
                    .await
                    .unwrap(),
                0,
                "nothing changed, nothing rewritten"
            );
            assert_eq!(head_of(&fixture).await.unwrap().event.id, first.event.id);

            // The prospect becomes a client: the same record moves on, keeping its lead.
            seed_client(&fixture).await;
            assert_eq!(
                ensure_customer_records(&fixture.state, community, now + 1_200)
                    .await
                    .unwrap(),
                1
            );
            let updated: CustomerRecordHead =
                serde_json::from_str(&head_of(&fixture).await.unwrap().event.content).unwrap();
            assert_eq!(updated.stage, CustomerStage::Client);
            assert_eq!(updated.lead_id, head.lead_id);
            assert!(updated
                .timeline
                .iter()
                .any(|e| e.kind == TimelineKind::Found));
        }

        const DESK_WA: &str = "27821230100";

        async fn desk_update(
            fixture: &Fixture,
            keys: &nostr::Keys,
            content: serde_json::Value,
        ) -> Result<IngestResult, IngestError> {
            let event = nostr::EventBuilder::new(
                nostr::Kind::Custom(buzz_core::kind::KIND_WHATSAPP_CONTACT_UPDATE as u16),
                content.to_string(),
            )
            .sign_with_keys(keys)
            .expect("sign desk command");
            super::super::super::whatsapp_commands::handle(
                &fixture.tenant,
                &fixture.state,
                event,
                auth(keys),
            )
            .await
        }

        async fn record_head(fixture: &Fixture) -> CustomerRecordHead {
            serde_json::from_str(&head_of(fixture).await.expect("record").event.content)
                .expect("customer head")
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn marking_call_booked_on_the_desk_adds_one_milestone_and_the_view_agrees() {
            let (fixture, owner, scout) = company().await;
            seed_prospect(&fixture).await;
            send(&fixture, &scout, &base(CustomerOp::Refresh))
                .await
                .expect("create the lead's record");
            let community = fixture.tenant.community();
            let db = &fixture.state.db;
            db.whatsapp_ensure_contact(community, DESK_WA, Some("Plane Tree"))
                .await
                .expect("contact");
            db.whatsapp_set_lead(community, DESK_WA, PROSPECT)
                .await
                .expect("link the lead");
            let command =
                |stage: &str| serde_json::json!({"v": 1, "waId": DESK_WA, "stage": stage});

            // A plain member changes neither the desk nor the record.
            let member = nostr::Keys::generate();
            add_member(&fixture, &member, "member").await;
            let message = rejection(desk_update(&fixture, &member, command("call_booked")).await);
            assert!(message.contains("only the owner, admins"), "{message}");
            assert!(record_head(&fixture).await.milestones.is_empty());

            // The owner marks the call booked: the desk stage AND the record agree.
            let done = desk_update(&fixture, &owner, command("call_booked"))
                .await
                .expect("owner marks the call booked");
            assert!(
                done.message
                    .contains("call booked added to the customer record"),
                "{}",
                done.message
            );
            let head = record_head(&fixture).await;
            assert_eq!(head.milestones.len(), 1);
            assert_eq!(head.milestones[0].kind, MilestoneKind::CallBooked);
            assert_eq!(head.milestones[0].by_pubkey, owner.public_key().to_hex());
            assert!(head
                .timeline
                .iter()
                .any(|entry| entry.kind == TimelineKind::CallBooked));

            // Marking it again is the same call: still one milestone.
            let again = desk_update(&fixture, &owner, command("call_booked"))
                .await
                .expect("again");
            assert!(
                !again.message.contains("added to the customer record"),
                "{}",
                again.message
            );
            assert_eq!(record_head(&fixture).await.milestones.len(), 1);

            let view = crate::api::whatsapp::thread_view(
                &fixture.state,
                community,
                DESK_WA,
                50,
                true,
                Utc::now(),
            )
            .await
            .expect("thread view");
            assert_eq!(view["contact"]["stage"], "call_booked");
            assert_eq!(view["contact"]["businessName"], "Plane Tree Guesthouse");
        }

        struct SqlCounter(std::sync::Arc<std::sync::atomic::AtomicUsize>);

        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for SqlCounter {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _ctx: tracing_subscriber::layer::Context<'_, S>,
            ) {
                if event.metadata().target().starts_with("sqlx::query") {
                    self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
        }

        #[tokio::test]
        #[ignore = "requires Postgres"]
        async fn the_contacts_query_costs_the_same_for_one_contact_and_a_hundred() {
            use tracing_subscriber::prelude::*;
            let (fixture, _owner, scout) = company().await;
            seed_prospect(&fixture).await;
            send(&fixture, &scout, &base(CustomerOp::Refresh))
                .await
                .expect("create the lead's record");
            let community = fixture.tenant.community();
            let db = &fixture.state.db;
            for index in 0..100u32 {
                let wa = format!("2782123{index:04}");
                db.whatsapp_ensure_contact(community, &wa, Some("Thabo"))
                    .await
                    .expect("contact");
                // One lead is real; the rest name leads that do not exist.
                let lead = if index == 0 {
                    PROSPECT.to_owned()
                } else {
                    uuid::Uuid::new_v4().to_string()
                };
                db.whatsapp_set_lead(community, &wa, &lead)
                    .await
                    .expect("link");
            }
            let all = db
                .whatsapp_list_contacts(community, 200)
                .await
                .expect("contacts");
            assert_eq!(all.len(), 100);
            let linked = all
                .iter()
                .position(|contact| contact.lead_id.as_deref() == Some(PROSPECT))
                .expect("the real lead is listed");

            let counter = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let _guard = tracing::subscriber::set_default(
                tracing_subscriber::registry().with(SqlCounter(counter.clone())),
            );
            let measure = |contacts: Vec<buzz_db::whatsapp::WhatsappContact>| {
                let fixture = &fixture;
                let counter = counter.clone();
                async move {
                    let before = counter.load(std::sync::atomic::Ordering::Relaxed);
                    let view = crate::api::whatsapp::contacts_view(
                        &fixture.state,
                        fixture.tenant.community(),
                        &contacts,
                        Utc::now(),
                    )
                    .await
                    .expect("contacts view");
                    let queries = counter.load(std::sync::atomic::Ordering::Relaxed) - before;
                    (queries, view)
                }
            };
            let (one, view_one) = measure(vec![all[linked].clone()]).await;
            let (hundred, view_all) = measure(all.clone()).await;
            assert!(one > 0, "the counter sees the queries");
            // GUARD: the view gathers its inputs in a fixed number of queries.
            assert_eq!(
                one, hundred,
                "100 contacts must not cost more queries than 1 ({one} vs {hundred})"
            );
            assert!(one <= 12, "a bounded number of queries: {one}");
            assert_eq!(view_one["contacts"].as_array().map(Vec::len), Some(1));
            assert_eq!(view_all["contacts"].as_array().map(Vec::len), Some(100));
            let linked_view = view_all["contacts"]
                .as_array()
                .expect("array")
                .iter()
                .find(|contact| contact["waId"] == all[linked].wa_id.as_str())
                .expect("linked contact");
            assert_eq!(linked_view["businessName"], "Plane Tree Guesthouse");
        }
    }
}
