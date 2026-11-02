//! Nostr-first broker for the first Colony business-record workflows.
//!
//! Member requests are validated and persisted with their relay-authored heads
//! in one database transaction. Client records always live in private groups.

use std::sync::Arc;

use chrono::{SecondsFormat, Utc};
use nostr::{Event, EventBuilder, Kind, Tag};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use buzz_core::business_records::{
    approval_matches_current_version, business_d_tag, client_d_tag, invoice_head_d_tag,
    invoice_total_minor, invoice_version_d_tag, is_iso_currency_code, money_adjustment_d_tag,
    money_follow_up_d_tag, parse_business_command, payment_d_tag, proposal_version_d_tag,
    prospect_d_tag, validate_business_command_scope, validate_company_work_d_tag,
    validate_company_work_item_action, validate_hex_reference, validate_utc_rfc3339,
    BusinessCommand, ClientAction, ClientHead, CompanyWorkItemAction, CompanyWorkItemActionKind,
    CompanyWorkItemHead, CompanyWorkStatus, CompanyWorkVerification, DeliverablePointer,
    DeliverableVersion, InvoiceHead, InvoiceStatus, InvoiceVersion, InvoiceVersionAction,
    MoneyAdjustment, MoneyAdjustmentType, MoneyFollowUpAction, MoneyFollowUpActionKind,
    MoneyFollowUpHead, MoneyFollowUpStatus, PartyAction, PartyHead, PaymentEvidence,
    ProposalAcceptance, ProposalHead, ProposalVersion, ProspectAction, ProspectActivity,
    ProspectHead, ProspectStage, RecordAction, ServiceAction, ServiceHead, WorkItemAction,
    WorkItemHead, BUSINESS_RECORD_SCHEMA_VERSION,
};
use buzz_core::company_records::{goal_d_tag, GoalHead, GoalStatus};
use buzz_core::kind::*;
use buzz_core::tenant::{CommunityId, TenantContext};
use buzz_core::StoredEvent;
use buzz_datastore_tracing::datastore_span;
use buzz_db::replaceable::{ParameterizedReplacePrecondition, ParameterizedReplaceStatus};
use buzz_db::{DbError, EventQuery};

use crate::state::AppState;

use super::ingest::{IngestAuth, IngestError, IngestResult};

#[cfg(test)]
type PartyValidationTestHook = ([u8; 32], Arc<tokio::sync::Notify>, Arc<tokio::sync::Notify>);

#[cfg(test)]
type ExpectedHeadLockTestHook = (
    [u8; 32],
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
);

#[cfg(test)]
type SecondaryChannelReadTestHook = (Uuid, Arc<std::sync::atomic::AtomicBool>);

#[cfg(test)]
static PARTY_VALIDATION_TEST_HOOK: std::sync::OnceLock<
    std::sync::Mutex<Option<PartyValidationTestHook>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
static EXPECTED_HEAD_LOCK_TEST_HOOK: std::sync::OnceLock<
    std::sync::Mutex<Option<ExpectedHeadLockTestHook>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
static SECONDARY_CHANNEL_READ_TEST_HOOK: std::sync::OnceLock<
    std::sync::Mutex<Option<SecondaryChannelReadTestHook>>,
> = std::sync::OnceLock::new();

struct HeadWrite {
    event: Event,
    channel_id: Uuid,
    d_tag: String,
    expected_event_id: Option<Vec<u8>>,
}

struct ExpectedHead {
    kind: u32,
    d_tag: String,
    event_id: Vec<u8>,
}

struct ConversionClaim {
    conversion_id: Uuid,
    proposal_id: Uuid,
    business_channel_id: Uuid,
    version_event_id: Vec<u8>,
    version_digest: Vec<u8>,
    acceptance_event_id: Vec<u8>,
    receipt_event_id: Vec<u8>,
    accepted_by_pubkey: Vec<u8>,
    client_id: Uuid,
    work_item_id: Uuid,
    draft_invoice_id: Uuid,
}

#[derive(Clone)]
struct ClientChannelProvision {
    channel_id: Uuid,
    name: String,
}

struct PersistBatch {
    command: Event,
    channel_id: Uuid,
    heads: Vec<HeadWrite>,
    appended: Vec<(Event, Uuid)>,
    expected_heads: Vec<ExpectedHead>,
    conversion_claim: Option<ConversionClaim>,
    client_channel_to_create: Option<ClientChannelProvision>,
    business_channel_to_register: Option<Uuid>,
}

#[datastore_span(name = "business_record_command", system = "postgresql")]
/// Validate and persist one member-signed business-record command.
pub async fn handle(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: Event,
    auth: IngestAuth,
) -> Result<IngestResult, IngestError> {
    if event.kind.as_u16() as u32 == KIND_COMPANY_WORK_TRACKING_ACTION {
        return super::company_work_tracking::handle(tenant, state, event, auth).await;
    }
    let (channel_id, d_tag) = command_coordinates(&event)?;
    let kind = event.kind.as_u16() as u32;
    let command = parse_business_command(kind, &event.content)
        .map_err(|error| invalid(format!("business command: {error}")))?;
    validate_business_command_scope(*tenant.community().as_uuid(), channel_id, &d_tag, &command)
        .map_err(|error| invalid(format!("business command scope: {error}")))?;

    require_token_channel_scope(&auth, channel_id)?;

    let _channel = if matches!(&command, BusinessCommand::CompanyWorkItemAction(_)) {
        get_company_work_stream_channel(state, tenant.community(), channel_id).await?
    } else {
        get_private_stream_channel(state, tenant.community(), channel_id).await?
    };
    let actor = auth.pubkey().to_bytes().to_vec();
    let role = state
        .db
        .get_member_role(tenant.community(), channel_id, &actor)
        .await
        .map_err(internal)?
        .ok_or_else(|| forbidden("actor is not a member of the private business channel"))?;

    let command = match command {
        BusinessCommand::CompanyWorkItemAction(action) => {
            return handle_company_work_item_action(
                tenant, state, event, auth, channel_id, d_tag, action,
            )
            .await;
        }
        other => other,
    };

    let community_business_channel =
        buzz_db::business_records::get_business_channel_id(&state.db, tenant.community())
            .await
            .map_err(internal)?;
    let mut business_channel_to_register = None;
    if matches!(
        &command,
        BusinessCommand::PartyAction(_)
            | BusinessCommand::ServiceAction(_)
            | BusinessCommand::ProspectAction(_)
            | BusinessCommand::ProposalVersion(_)
            | BusinessCommand::ProposalAcceptance(_)
    ) {
        match community_business_channel {
            Some(expected_channel_id) if expected_channel_id == channel_id => {}
            Some(_) => {
                return Err(forbidden(
                    "community business commands require the registered business channel",
                ));
            }
            None if initializes_business_channel(&command) => {
                let actor_hex = auth.pubkey().to_hex();
                let community_member = state
                    .db
                    .get_relay_member(tenant.community(), &actor_hex)
                    .await
                    .map_err(internal)?;
                if !community_member
                    .as_ref()
                    .is_some_and(|member| matches!(member.role.as_str(), "owner" | "admin"))
                {
                    return Err(forbidden(
                        "only a community owner or admin can initialize the business channel",
                    ));
                }
                business_channel_to_register = Some(channel_id);
            }
            None => {
                return Err(forbidden(
                    "community business channel has not been initialized",
                ));
            }
        }
    }

    if matches!(
        &command,
        BusinessCommand::PartyAction(_)
            | BusinessCommand::ServiceAction(_)
            | BusinessCommand::ProspectAction(_)
            | BusinessCommand::ClientAction(_)
            | BusinessCommand::WorkItemAction(_)
            | BusinessCommand::ProposalVersion(_)
            | BusinessCommand::DeliverableVersion(_)
            | BusinessCommand::InvoiceVersion(_)
            | BusinessCommand::PaymentEvidence(_)
            | BusinessCommand::MoneyAdjustment(_)
            | BusinessCommand::MoneyFollowUp(_)
    ) {
        match &command {
            BusinessCommand::PartyAction(_) | BusinessCommand::ProposalVersion(_) => {
                require_member_or_admin(&role)?;
            }
            BusinessCommand::ProspectAction(action) => {
                require_prospect_actor(&role, action.action)?;
            }
            BusinessCommand::ServiceAction(_) => require_admin(&role)?,
            BusinessCommand::ClientAction(_) => require_admin(&role)?,
            BusinessCommand::WorkItemAction(action) => {
                validate_work_item_action(&role, action)?;
            }
            _ => {}
        }
        if let Some(replay) = replay_existing_command(state, tenant, &event, channel_id).await? {
            return Ok(replay);
        }
    }

    if matches!(
        &command,
        BusinessCommand::InvoiceVersion(_)
            | BusinessCommand::PaymentEvidence(_)
            | BusinessCommand::MoneyAdjustment(_)
            | BusinessCommand::MoneyFollowUp(_)
    ) {
        let community_role = state
            .db
            .get_relay_member(tenant.community(), &auth.pubkey().to_hex())
            .await
            .map_err(internal)?
            .map(|member| member.role);
        if !community_role.as_deref().is_some_and(is_admin) {
            return Err(forbidden(
                "community owner or admin role is required for money records",
            ));
        }
    }

    let mut heads = Vec::new();
    let mut appended = Vec::new();
    let mut expected_heads = Vec::new();
    let mut conversion_claim = None;
    let mut client_channel_to_create = None;
    let mut client_channel_to_sync = None;

    match command {
        BusinessCommand::PartyAction(action) => {
            require_member_or_admin(&role)?;
            if action.party.party_id != action.party_id {
                return Err(invalid("party id does not match party record"));
            }
            validate_party(&action)?;
            let d_tag = business_d_tag(*tenant.community().as_uuid(), "party", action.party_id);
            let current =
                current_head::<PartyHead>(state, tenant.community(), KIND_PARTY_HEAD, &d_tag)
                    .await?;
            if current
                .as_ref()
                .is_some_and(|stored| stored.channel_id != Some(channel_id))
            {
                return Err(conflict("party record belongs to another business channel"));
            }
            check_expected(
                action.action,
                action.expected_head_event_id.as_deref(),
                current.as_ref(),
            )?;
            let previous = current
                .as_ref()
                .map(|stored| parse_content::<PartyHead>(&stored.event))
                .transpose()?;
            let status = resolve_lifecycle_status(
                action.action,
                previous.as_ref().map(|head| head.status.as_str()),
                None,
                "active",
            )?;
            let head = PartyHead {
                schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
                party_id: action.party_id,
                status,
                party: action.party,
                source_action_event_id: event.id.to_hex(),
            };
            heads.push(HeadWrite {
                event: relay_head_event(
                    KIND_PARTY_HEAD,
                    channel_id,
                    &d_tag,
                    &head,
                    current.as_ref(),
                    state,
                )?,
                channel_id,
                d_tag,
                expected_event_id: current.map(|stored| stored.event.id.to_bytes().to_vec()),
            });
        }
        BusinessCommand::ServiceAction(action) => {
            require_admin(&role)?;
            validate_service_action(&action)?;
            let d_tag = business_d_tag(*tenant.community().as_uuid(), "service", action.service_id);
            let current =
                current_head::<ServiceHead>(state, tenant.community(), KIND_SERVICE_HEAD, &d_tag)
                    .await?;
            if current
                .as_ref()
                .is_some_and(|stored| stored.channel_id != Some(channel_id))
            {
                return Err(conflict("service belongs to another business channel"));
            }
            check_expected(
                action.action,
                action.expected_head_event_id.as_deref(),
                current.as_ref(),
            )?;
            let previous = current
                .as_ref()
                .map(|stored| parse_content::<ServiceHead>(&stored.event))
                .transpose()?;
            let status = resolve_lifecycle_status(
                action.action,
                previous.as_ref().map(|head| head.status.as_str()),
                None,
                "active",
            )?;
            let head = ServiceHead {
                schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
                service_id: action.service_id,
                status,
                service: action.service,
                source_action_event_id: event.id.to_hex(),
            };
            heads.push(HeadWrite {
                event: relay_head_event(
                    KIND_SERVICE_HEAD,
                    channel_id,
                    &d_tag,
                    &head,
                    current.as_ref(),
                    state,
                )?,
                channel_id,
                d_tag,
                expected_event_id: current.map(|stored| stored.event.id.to_bytes().to_vec()),
            });
        }
        BusinessCommand::ProspectAction(action) => {
            require_prospect_actor(&role, action.action)?;
            validate_prospect_action(&action)?;
            let d_tag = prospect_d_tag(*tenant.community().as_uuid(), action.prospect_id);
            let current =
                current_head::<ProspectHead>(state, tenant.community(), KIND_PROSPECT_HEAD, &d_tag)
                    .await?;
            if current
                .as_ref()
                .is_some_and(|stored| stored.channel_id != Some(channel_id))
            {
                return Err(conflict("prospect belongs to another business channel"));
            }
            check_expected(
                action.action,
                action.expected_head_event_id.as_deref(),
                current.as_ref(),
            )?;
            let previous = current
                .as_ref()
                .map(|stored| parse_content::<ProspectHead>(&stored.event))
                .transpose()?;
            if previous
                .as_ref()
                .is_some_and(|head| head.prospect.party.party_id != action.prospect.party.party_id)
            {
                return Err(conflict("prospect party identity cannot be changed"));
            }
            if action.prospect.stage == ProspectStage::Won
                && previous
                    .as_ref()
                    .is_none_or(|head| head.prospect.stage != ProspectStage::Won)
            {
                return Err(forbidden(
                    "only an accepted proposal can move a prospect to won",
                ));
            }

            let business_channel_id = community_business_channel.unwrap_or(channel_id);
            if let Some(activity) = action.activity.as_ref() {
                if let (Some(proposal_id), Some(version_event_id)) = (
                    activity.proposal_id,
                    activity.proposal_version_event_id.as_deref(),
                ) {
                    let proposal_d =
                        business_d_tag(*tenant.community().as_uuid(), "proposal", proposal_id);
                    let proposal_stored = current_head::<ProposalHead>(
                        state,
                        tenant.community(),
                        KIND_PROPOSAL_HEAD,
                        &proposal_d,
                    )
                    .await?
                    .ok_or_else(|| conflict("proposal is unavailable for this activity"))?;
                    let proposal: ProposalHead = parse_content(&proposal_stored.event)?;
                    if proposal_stored.channel_id != Some(business_channel_id)
                        || proposal.proposal_id != proposal_id
                        || proposal.current_version_event_id != version_event_id
                    {
                        return Err(conflict(
                            "proposal changed or belongs to another business channel",
                        ));
                    }
                    let version = load_version_event(
                        state,
                        tenant.community(),
                        version_event_id,
                        KIND_PROPOSAL_VERSION,
                    )
                    .await?;
                    let version_content: ProposalVersion = parse_content(&version.event)?;
                    let expected_version_d = proposal_version_d_tag(
                        *tenant.community().as_uuid(),
                        proposal_id,
                        proposal.revision,
                    );
                    let (version_channel, version_d) = command_coordinates(&version.event)?;
                    if version.channel_id != Some(business_channel_id)
                        || version_channel != business_channel_id
                        || version_d != expected_version_d
                        || version_content.proposal_id != proposal_id
                        || version_content.prospect_party_id != action.prospect.party.party_id
                        || version_content.revision != proposal.revision
                    {
                        return Err(conflict(
                            "proposal version does not belong to this prospect",
                        ));
                    }
                    expected_heads.push(ExpectedHead {
                        kind: KIND_PROPOSAL_HEAD,
                        d_tag: proposal_d,
                        event_id: proposal_stored.event.id.to_bytes().to_vec(),
                    });
                }
            }
            let party_d = business_d_tag(
                *tenant.community().as_uuid(),
                "party",
                action.prospect.party.party_id,
            );
            let party_current =
                current_head::<PartyHead>(state, tenant.community(), KIND_PARTY_HEAD, &party_d)
                    .await?;
            let party_current = match (action.action, party_current) {
                (RecordAction::Create, None) => None,
                (RecordAction::Create, Some(existing)) => {
                    if existing.channel_id != Some(business_channel_id) {
                        return Err(conflict(
                            "prospect party belongs to another business channel",
                        ));
                    }
                    let existing_party: PartyHead = parse_content(&existing.event)?;
                    if existing_party.status != "active"
                        || existing_party.party != action.prospect.party
                    {
                        return Err(conflict("prospect party identity already exists"));
                    }
                    Some(existing)
                }
                (_, Some(existing)) if existing.channel_id == Some(business_channel_id) => {
                    let existing_party: PartyHead = parse_content(&existing.event)?;
                    if existing_party.status != "active" {
                        return Err(conflict("prospect party identity is not active"));
                    }
                    Some(existing)
                }
                (_, _) => return Err(conflict("prospect party identity is not available")),
            };
            if let Some(party_current) = party_current.as_ref() {
                let existing_party: PartyHead = parse_content(&party_current.event)?;
                if existing_party.party != action.prospect.party {
                    expected_heads.push(ExpectedHead {
                        kind: KIND_PARTY_HEAD,
                        d_tag: party_d.clone(),
                        event_id: party_current.event.id.to_bytes().to_vec(),
                    });
                    let party_head = PartyHead {
                        schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
                        party_id: action.prospect.party.party_id,
                        status: "active".into(),
                        party: action.prospect.party.clone(),
                        source_action_event_id: event.id.to_hex(),
                    };
                    heads.push(HeadWrite {
                        event: relay_head_event(
                            KIND_PARTY_HEAD,
                            business_channel_id,
                            &party_d,
                            &party_head,
                            Some(party_current),
                            state,
                        )?,
                        channel_id: business_channel_id,
                        d_tag: party_d.clone(),
                        expected_event_id: Some(party_current.event.id.to_bytes().to_vec()),
                    });
                }
            } else {
                let party_head = PartyHead {
                    schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
                    party_id: action.prospect.party.party_id,
                    status: "active".into(),
                    party: action.prospect.party.clone(),
                    source_action_event_id: event.id.to_hex(),
                };
                heads.push(HeadWrite {
                    event: relay_head_event(
                        KIND_PARTY_HEAD,
                        business_channel_id,
                        &party_d,
                        &party_head,
                        None,
                        state,
                    )?,
                    channel_id: business_channel_id,
                    d_tag: party_d,
                    expected_event_id: None,
                });
            }

            let status = resolve_lifecycle_status(
                action.action,
                previous.as_ref().map(|head| head.status.as_str()),
                None,
                "active",
            )?;
            let mut activities = previous
                .as_ref()
                .map(|head| head.activities.clone())
                .unwrap_or_default();
            if let Some(activity) = action.activity {
                if activities.len() >= 500 {
                    return Err(invalid("prospect activity history is full"));
                }
                if activities
                    .iter()
                    .any(|existing| existing.activity_id == activity.activity_id)
                {
                    return Err(conflict("prospect activity id already exists"));
                }
                activities.push(ProspectActivity {
                    activity_id: activity.activity_id,
                    activity_kind: activity.activity_kind,
                    content: activity.content,
                    proposal_id: activity.proposal_id,
                    proposal_version_event_id: activity.proposal_version_event_id,
                    author_pubkey: auth.pubkey().to_hex(),
                    created_at: event.created_at.as_secs() as i64,
                });
            }
            let head = ProspectHead {
                schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
                prospect_id: action.prospect_id,
                status,
                prospect: action.prospect,
                activities,
                source_action_event_id: event.id.to_hex(),
            };
            heads.push(HeadWrite {
                event: relay_head_event(
                    KIND_PROSPECT_HEAD,
                    channel_id,
                    &d_tag,
                    &head,
                    current.as_ref(),
                    state,
                )?,
                channel_id,
                d_tag,
                expected_event_id: current.map(|stored| stored.event.id.to_bytes().to_vec()),
            });
        }
        BusinessCommand::ClientAction(action) => {
            require_admin(&role)?;
            validate_client_action(channel_id, &action)?;
            let business_channel_id = community_business_channel
                .ok_or_else(|| forbidden("party link is not authorized"))?;
            require_token_channel_scope(&auth, business_channel_id)?;
            if state
                .db
                .get_member_role(tenant.community(), business_channel_id, &actor)
                .await
                .map_err(internal)?
                .is_none()
            {
                return Err(forbidden("party link is not authorized"));
            }
            let party_d =
                business_d_tag(*tenant.community().as_uuid(), "party", action.head.party_id);
            #[cfg(test)]
            before_secondary_channel_read_for_test(business_channel_id);
            let party_event = current_head_in_channel::<PartyHead>(
                state,
                tenant.community(),
                business_channel_id,
                KIND_PARTY_HEAD,
                &party_d,
            )
            .await?
            .ok_or_else(|| forbidden("party link is not authorized"))?;
            let party: PartyHead = parse_content(&party_event.event)?;
            if party_event.channel_id != Some(business_channel_id)
                || party.party_id != action.head.party_id
                || party.status != "active"
            {
                return Err(forbidden("party link is not authorized"));
            }
            after_party_validation_for_test(event.id.to_bytes()).await;
            expected_heads.push(ExpectedHead {
                kind: KIND_PARTY_HEAD,
                d_tag: party_d,
                event_id: party_event.event.id.to_bytes().to_vec(),
            });
            let d_tag = client_d_tag(action.client_id, "client", action.client_id);
            let current =
                current_head::<ClientHead>(state, tenant.community(), KIND_CLIENT_HEAD, &d_tag)
                    .await?;
            if current
                .as_ref()
                .is_some_and(|stored| stored.channel_id != Some(channel_id))
            {
                return Err(conflict("client record belongs to another client channel"));
            }
            check_expected(
                action.action,
                action.expected_head_event_id.as_deref(),
                current.as_ref(),
            )?;
            let previous = current
                .as_ref()
                .map(|stored| parse_content::<ClientHead>(&stored.event))
                .transpose()?;
            let status = resolve_lifecycle_status(
                action.action,
                previous.as_ref().map(|head| head.status.as_str()),
                Some(&action.head.status),
                "active",
            )?;
            let head = ClientHead {
                schema_version: action.head.schema_version,
                client_id: action.head.client_id,
                party_id: action.head.party_id,
                display_name: action.head.display_name,
                approver_pubkeys: action.head.approver_pubkeys,
                status,
                source_action_event_id: event.id.to_hex(),
            };
            heads.push(HeadWrite {
                event: relay_head_event(
                    KIND_CLIENT_HEAD,
                    channel_id,
                    &d_tag,
                    &head,
                    current.as_ref(),
                    state,
                )?,
                channel_id,
                d_tag,
                expected_event_id: current.map(|stored| stored.event.id.to_bytes().to_vec()),
            });
        }
        BusinessCommand::WorkItemAction(action) => {
            validate_work_item_action(&role, &action)?;
            let d_tag = client_d_tag(action.client_id, "work", action.work_item_id);
            let current = current_head::<WorkItemHead>(
                state,
                tenant.community(),
                KIND_WORK_ITEM_HEAD,
                &d_tag,
            )
            .await?;
            if current
                .as_ref()
                .is_some_and(|stored| stored.channel_id != Some(channel_id))
            {
                return Err(conflict("work item belongs to another client channel"));
            }
            check_expected(
                action.action,
                action.expected_head_event_id.as_deref(),
                current.as_ref(),
            )?;
            let previous = current
                .as_ref()
                .map(|stored| parse_content::<WorkItemHead>(&stored.event))
                .transpose()?;
            if let Some(previous) = previous.as_ref() {
                if action.head.deliverables != previous.deliverables {
                    return Err(invalid(
                        "work-item actions cannot change deliverable version pointers",
                    ));
                }
                if !is_admin(&role)
                    && !previous
                        .assigned_pubkeys
                        .iter()
                        .any(|pubkey| pubkey == &auth.pubkey().to_hex())
                {
                    return Err(forbidden("actor is not assigned to this work item"));
                }
                if !is_admin(&role)
                    && (action.head.title != previous.title
                        || action.head.assigned_pubkeys != previous.assigned_pubkeys
                        || action.head.approver_pubkeys != previous.approver_pubkeys)
                {
                    return Err(forbidden(
                        "only an owner or admin can change work-item assignments or title",
                    ));
                }
            } else {
                if !is_admin(&role) {
                    return Err(forbidden("only an owner or admin can create a work item"));
                }
                if !action.head.deliverables.is_empty() {
                    return Err(invalid(
                        "work-item creation cannot supply unverified deliverable pointers",
                    ));
                }
            }
            let status = resolve_lifecycle_status(
                action.action,
                previous.as_ref().map(|head| head.status.as_str()),
                Some(&action.head.status),
                "open",
            )?;
            let head = WorkItemHead {
                schema_version: action.head.schema_version,
                client_id: action.head.client_id,
                work_item_id: action.head.work_item_id,
                title: action.head.title,
                status,
                assigned_pubkeys: action.head.assigned_pubkeys,
                approver_pubkeys: action.head.approver_pubkeys,
                deliverables: action.head.deliverables,
                source_event_id: event.id.to_hex(),
            };
            heads.push(HeadWrite {
                event: relay_head_event(
                    KIND_WORK_ITEM_HEAD,
                    channel_id,
                    &d_tag,
                    &head,
                    current.as_ref(),
                    state,
                )?,
                channel_id,
                d_tag,
                expected_event_id: current.map(|stored| stored.event.id.to_bytes().to_vec()),
            });
        }
        BusinessCommand::CompanyWorkItemAction(_) => {
            return Err(IngestError::Internal(
                "error: company work action bypassed its transactional broker".into(),
            ));
        }
        BusinessCommand::ProposalVersion(version) => {
            require_member_or_admin(&role)?;
            validate_proposal_version(&version)?;
            let business_channel_id = community_business_channel
                .ok_or_else(|| forbidden("community business channel has not been initialized"))?;
            let party_d = business_d_tag(
                *tenant.community().as_uuid(),
                "party",
                version.prospect_party_id,
            );
            let party_event = current_head_in_channel::<PartyHead>(
                state,
                tenant.community(),
                business_channel_id,
                KIND_PARTY_HEAD,
                &party_d,
            )
            .await?
            .ok_or_else(|| conflict("proposal prospect party is not available"))?;
            let party: PartyHead = parse_content(&party_event.event)?;
            if party_event.channel_id != Some(business_channel_id)
                || party.party_id != version.prospect_party_id
                || party.status != "active"
            {
                return Err(conflict("proposal prospect party is not available"));
            }
            after_party_validation_for_test(event.id.to_bytes()).await;
            expected_heads.push(ExpectedHead {
                kind: KIND_PARTY_HEAD,
                d_tag: party_d,
                event_id: party_event.event.id.to_bytes().to_vec(),
            });
            let proposal_head_d = business_d_tag(
                *tenant.community().as_uuid(),
                "proposal",
                version.proposal_id,
            );
            let current = current_head::<ProposalHead>(
                state,
                tenant.community(),
                KIND_PROPOSAL_HEAD,
                &proposal_head_d,
            )
            .await?;
            if current
                .as_ref()
                .is_some_and(|stored| stored.channel_id != Some(channel_id))
            {
                return Err(conflict("proposal belongs to another business channel"));
            }
            if version.revision == 1 {
                if current.is_some() || version.previous_version_event_id.is_some() {
                    return Err(conflict("proposal already has a current version"));
                }
            } else {
                let Some(current) = current.as_ref() else {
                    return Err(conflict("proposal prior version is missing"));
                };
                let prior: ProposalHead = parse_content(&current.event)?;
                if prior.revision.checked_add(1) != Some(version.revision)
                    || version.previous_version_event_id.as_deref()
                        != Some(prior.current_version_event_id.as_str())
                {
                    return Err(conflict("proposal version changed since it was loaded"));
                }
            }
            let version_event = event.clone();
            let version_digest = digest_hex(version_event.content.as_bytes());
            let head = ProposalHead {
                schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
                proposal_id: version.proposal_id,
                current_version_event_id: version_event.id.to_hex(),
                current_version_digest: version_digest,
                revision: version.revision,
                source_event_id: version_event.id.to_hex(),
            };
            let expected_event_id = current
                .as_ref()
                .map(|stored| stored.event.id.to_bytes().to_vec());
            if let Some(event_id) = expected_event_id.as_ref() {
                expected_heads.push(ExpectedHead {
                    kind: KIND_PROPOSAL_HEAD,
                    d_tag: proposal_head_d.clone(),
                    event_id: event_id.clone(),
                });
            }
            heads.push(HeadWrite {
                event: relay_head_event(
                    KIND_PROPOSAL_HEAD,
                    channel_id,
                    &proposal_head_d,
                    &head,
                    current.as_ref(),
                    state,
                )?,
                channel_id,
                d_tag: proposal_head_d,
                expected_event_id,
            });
        }
        BusinessCommand::ProposalAcceptance(acceptance) => {
            require_acceptor_channel_role(&role)?;
            require_token_channel_scope(&auth, acceptance.client_id)?;
            if let Some(replay) = existing_proposal_conversion_replay(
                tenant,
                state,
                &auth,
                channel_id,
                &event,
                &acceptance,
            )
            .await?
            {
                ensure_client_group_discovery(tenant, state, &auth, acceptance.client_id).await?;
                return Ok(replay);
            }
            let result =
                prepare_proposal_acceptance(tenant, state, &auth, channel_id, &event, acceptance)
                    .await?;
            heads.extend(result.heads);
            appended.extend(result.appended);
            expected_heads.push(result.current_proposal_head);
            expected_heads.push(result.current_party_head);
            if let Some(prospect_head) = result.expected_prospect_head {
                expected_heads.push(prospect_head);
            }
            conversion_claim = Some(result.claim);
            client_channel_to_create = result.client_channel_to_create;
            client_channel_to_sync = Some(result.client_channel_id);
        }
        BusinessCommand::DeliverableVersion(version) => {
            let current = current_work_item(
                state,
                tenant.community(),
                channel_id,
                version.client_id,
                version.work_item_id,
            )
            .await?;
            let (head_event, mut head) = current;
            authorize_work_assignee_or_admin(&role, &auth, &head)?;
            let pointer = validate_deliverable_version(&version, &event, &head)?;
            let current_pointer = head
                .deliverables
                .iter_mut()
                .find(|current| current.deliverable_id == version.deliverable_id);
            match current_pointer {
                Some(current) => {
                    let previous_event = load_version_event(
                        state,
                        tenant.community(),
                        &current.version_event_id,
                        KIND_DELIVERABLE_VERSION,
                    )
                    .await?;
                    let previous: DeliverableVersion = parse_content(&previous_event.event)?;
                    if previous.version.checked_add(1) != Some(version.version)
                        || version.previous_version_event_id.as_deref()
                            != Some(current.version_event_id.as_str())
                    {
                        return Err(conflict("deliverable version changed since it was loaded"));
                    }
                    *current = pointer;
                }
                None if version.version == 1 && version.previous_version_event_id.is_none() => {
                    head.deliverables.push(pointer);
                    head.deliverables.sort_by_key(|item| item.deliverable_id);
                }
                None => return Err(conflict("deliverable prior version is missing")),
            }
            head.source_event_id = event.id.to_hex();
            let d_tag = client_d_tag(version.client_id, "work", version.work_item_id);
            let expected_id = head_event.event.id.to_bytes().to_vec();
            heads.push(HeadWrite {
                event: relay_head_event(
                    KIND_WORK_ITEM_HEAD,
                    channel_id,
                    &d_tag,
                    &head,
                    Some(&head_event),
                    state,
                )?,
                channel_id,
                d_tag,
                expected_event_id: Some(expected_id),
            });
        }
        BusinessCommand::DeliverableApproval(approval) => {
            let (head_event, head) = current_work_item(
                state,
                tenant.community(),
                channel_id,
                approval.client_id,
                approval.work_item_id,
            )
            .await?;
            if !head
                .approver_pubkeys
                .iter()
                .any(|pubkey| pubkey == &auth.pubkey().to_hex())
            {
                return Err(forbidden("actor is not an approver for this work item"));
            }
            if let Some(replay) = replay_existing_command(state, tenant, &event, channel_id).await?
            {
                return Ok(replay);
            }
            let current = head
                .deliverables
                .iter()
                .find(|pointer| pointer.deliverable_id == approval.deliverable_id);
            if !approval_matches_current_version(&approval, current) {
                return Err(conflict(
                    "approval must target the current deliverable version",
                ));
            }
            let version_event = load_version_event(
                state,
                tenant.community(),
                &approval.version_event_id,
                KIND_DELIVERABLE_VERSION,
            )
            .await?;
            let version: DeliverableVersion = parse_content(&version_event.event)?;
            if version.client_id != approval.client_id
                || version.work_item_id != approval.work_item_id
                || version.deliverable_id != approval.deliverable_id
                || version.content_digest != approval.content_digest
            {
                return Err(conflict(
                    "approval does not match the stored deliverable version",
                ));
            }
            validate_hex_reference(&approval.media_digest)
                .map_err(|error| invalid(error.to_string()))?;
            expected_heads.push(ExpectedHead {
                kind: KIND_WORK_ITEM_HEAD,
                d_tag: client_d_tag(approval.client_id, "work", approval.work_item_id),
                event_id: head_event.event.id.to_bytes().to_vec(),
            });
        }
        BusinessCommand::InvoiceVersion(version)
            if version.action == InvoiceVersionAction::Create =>
        {
            validate_invoice_version(&version)?;
            let client_d = client_d_tag(version.client_id, "client", version.client_id);
            let client_event = current_head_in_channel::<ClientHead>(
                state,
                tenant.community(),
                channel_id,
                KIND_CLIENT_HEAD,
                &client_d,
            )
            .await?
            .ok_or_else(|| conflict("an invoice needs an existing client record"))?;
            let client: ClientHead = parse_content(&client_event.event)?;
            if client.client_id != version.client_id || client.status == "archived" {
                return Err(conflict("an invoice needs an active client record"));
            }
            let invoice_d = invoice_head_d_tag(version.client_id, version.invoice_id);
            ensure_new_record_coordinate(
                state,
                tenant.community(),
                channel_id,
                KIND_INVOICE_HEAD,
                &invoice_d,
            )
            .await?;
            let version_event_id = event.id.to_hex();
            let head = InvoiceHead {
                schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
                client_id: version.client_id,
                invoice_id: version.invoice_id,
                // A direct invoice is its own source: later edits quote this id.
                proposal_id: version.invoice_id,
                proposal_version_event_id: version_event_id.clone(),
                currency: version.currency.clone(),
                lines: version.lines.clone(),
                tax_lines: version.tax_lines.clone(),
                seller_tax_number: version.seller_tax_number.clone(),
                customer_tax_number: version.customer_tax_number.clone(),
                total_minor: version.total_minor,
                credited_minor: 0,
                written_off_minor: 0,
                collected_minor: 0,
                outstanding_minor: 0,
                payment_evidence_count: 0,
                version: version.version,
                current_version_event_id: version_event_id.clone(),
                status: InvoiceStatus::Draft,
                due_at: version.due_at,
                issued_at: None,
                source_event_id: version_event_id,
            };
            heads.push(HeadWrite {
                event: relay_event(KIND_INVOICE_HEAD, channel_id, &invoice_d, &head, state)?,
                channel_id,
                d_tag: invoice_d,
                expected_event_id: None,
            });
        }
        BusinessCommand::InvoiceVersion(version) => {
            validate_invoice_version(&version)?;
            let (head_event, current) = current_invoice_head(
                state,
                tenant.community(),
                channel_id,
                version.client_id,
                version.invoice_id,
            )
            .await?;
            if version.expected_head_event_id.as_deref()
                != Some(head_event.event.id.to_hex().as_str())
                || version.previous_version_event_id.as_deref()
                    != Some(current.current_version_event_id.as_str())
                || current.version.checked_add(1) != Some(version.version)
                || version.proposal_version_event_id.as_deref()
                    != Some(current.proposal_version_event_id.as_str())
            {
                return Err(conflict("invoice changed since this version was prepared"));
            }
            let status = match version.action {
                InvoiceVersionAction::ProposalAcceptance => {
                    return Err(forbidden(
                        "only proposal acceptance can create an invoice draft",
                    ));
                }
                InvoiceVersionAction::DraftEdit if current.status == InvoiceStatus::Draft => {
                    InvoiceStatus::Draft
                }
                InvoiceVersionAction::Issue if current.status == InvoiceStatus::Draft => {
                    InvoiceStatus::Issued
                }
                InvoiceVersionAction::Void
                    if matches!(current.status, InvoiceStatus::Draft | InvoiceStatus::Issued) =>
                {
                    if current.payment_evidence_count > 0 {
                        return Err(conflict(
                            "an invoice with payment evidence cannot be voided",
                        ));
                    }
                    InvoiceStatus::Void
                }
                _ => {
                    return Err(conflict(
                        "invoice lifecycle action is not valid in this state",
                    ));
                }
            };
            if version.status != status {
                return Err(invalid("invoice version status does not match its action"));
            }
            if matches!(
                version.action,
                InvoiceVersionAction::Issue | InvoiceVersionAction::Void
            ) && (version.currency != current.currency
                || version.lines != current.lines
                || version.tax_lines != current.tax_lines
                || version.seller_tax_number != current.seller_tax_number
                || version.customer_tax_number != current.customer_tax_number
                || version.total_minor != current.total_minor
                || version.due_at != current.due_at)
            {
                return Err(invalid(
                    "issuing or voiding an invoice cannot change its draft terms",
                ));
            }
            let mut head = current;
            head.currency = version.currency.clone();
            head.lines = version.lines.clone();
            head.tax_lines = version.tax_lines.clone();
            head.seller_tax_number = version.seller_tax_number.clone();
            head.customer_tax_number = version.customer_tax_number.clone();
            head.total_minor = version.total_minor;
            head.version = version.version;
            head.current_version_event_id = event.id.to_hex();
            head.status = status;
            head.due_at = version.due_at;
            if version.action == InvoiceVersionAction::Issue {
                head.issued_at = Some(event.created_at.as_secs() as i64);
            }
            head.outstanding_minor = if status == InvoiceStatus::Issued {
                invoice_outstanding_minor(&head)?
            } else {
                0
            };
            head.source_event_id = event.id.to_hex();
            let d_tag = invoice_head_d_tag(version.client_id, version.invoice_id);
            let expected_id = head_event.event.id.to_bytes().to_vec();
            heads.push(HeadWrite {
                event: relay_head_event(
                    KIND_INVOICE_HEAD,
                    channel_id,
                    &d_tag,
                    &head,
                    Some(&head_event),
                    state,
                )?,
                channel_id,
                d_tag: d_tag.clone(),
                expected_event_id: Some(expected_id.clone()),
            });
            expected_heads.push(ExpectedHead {
                kind: KIND_INVOICE_HEAD,
                d_tag,
                event_id: expected_id,
            });
        }
        BusinessCommand::PaymentEvidence(payment) => {
            validate_payment_evidence(&payment)?;
            let (head_event, mut head) = current_invoice_head(
                state,
                tenant.community(),
                channel_id,
                payment.client_id,
                payment.invoice_id,
            )
            .await?;
            if payment.expected_invoice_head_event_id != head_event.event.id.to_hex() {
                return Err(conflict(
                    "invoice changed before payment evidence was recorded",
                ));
            }
            if head.status != InvoiceStatus::Issued {
                return Err(conflict("payment evidence requires an issued invoice"));
            }
            if payment.currency != head.currency {
                return Err(invalid("payment currency must match the invoice currency"));
            }
            if payment.amount_minor > head.outstanding_minor {
                return Err(invalid("payment amount exceeds the outstanding balance"));
            }
            let payment_d = payment_d_tag(payment.client_id, payment.payment_id);
            ensure_new_record_coordinate(
                state,
                tenant.community(),
                channel_id,
                KIND_PAYMENT,
                &payment_d,
            )
            .await?;
            head.collected_minor = head
                .collected_minor
                .checked_add(payment.amount_minor)
                .ok_or_else(|| invalid("collected total overflows minor units"))?;
            head.payment_evidence_count = head
                .payment_evidence_count
                .checked_add(1)
                .ok_or_else(|| invalid("payment evidence count overflows"))?;
            head.outstanding_minor = invoice_outstanding_minor(&head)?;
            head.source_event_id = event.id.to_hex();
            let d_tag = invoice_head_d_tag(payment.client_id, payment.invoice_id);
            let expected_id = head_event.event.id.to_bytes().to_vec();
            heads.push(HeadWrite {
                event: relay_head_event(
                    KIND_INVOICE_HEAD,
                    channel_id,
                    &d_tag,
                    &head,
                    Some(&head_event),
                    state,
                )?,
                channel_id,
                d_tag: d_tag.clone(),
                expected_event_id: Some(expected_id.clone()),
            });
            expected_heads.push(ExpectedHead {
                kind: KIND_INVOICE_HEAD,
                d_tag,
                event_id: expected_id,
            });
        }
        BusinessCommand::MoneyAdjustment(adjustment) => {
            validate_money_adjustment(&adjustment)?;
            let (head_event, mut head) = current_invoice_head(
                state,
                tenant.community(),
                channel_id,
                adjustment.client_id,
                adjustment.invoice_id,
            )
            .await?;
            if adjustment.expected_invoice_head_event_id != head_event.event.id.to_hex() {
                return Err(conflict(
                    "invoice changed before the adjustment was recorded",
                ));
            }
            if head.status != InvoiceStatus::Issued {
                return Err(conflict("adjustments require an issued invoice"));
            }
            if adjustment.currency != head.currency {
                return Err(invalid(
                    "adjustment currency must match the invoice currency",
                ));
            }
            let adjustment_d =
                money_adjustment_d_tag(adjustment.client_id, adjustment.adjustment_id);
            ensure_new_record_coordinate(
                state,
                tenant.community(),
                channel_id,
                KIND_MONEY_ADJUSTMENT,
                &adjustment_d,
            )
            .await?;
            match adjustment.adjustment_type {
                MoneyAdjustmentType::CreditNote => {
                    let available = head.total_minor.saturating_sub(head.credited_minor);
                    if adjustment.amount_minor > available {
                        return Err(invalid("credit note exceeds the remaining invoice amount"));
                    }
                    head.credited_minor = head
                        .credited_minor
                        .checked_add(adjustment.amount_minor)
                        .ok_or_else(|| invalid("credited total overflows minor units"))?;
                }
                MoneyAdjustmentType::Refund => {
                    let credit_available = invoice_credit_available_minor(&head)?;
                    if adjustment.amount_minor > credit_available {
                        return Err(invalid("refund exceeds the available client credit"));
                    }
                    head.collected_minor = head
                        .collected_minor
                        .checked_sub(adjustment.amount_minor)
                        .ok_or_else(|| invalid("refund exceeds recorded collections"))?;
                }
                MoneyAdjustmentType::WriteOff => {
                    if adjustment.amount_minor > head.outstanding_minor {
                        return Err(invalid("write-off exceeds the outstanding balance"));
                    }
                    head.written_off_minor = head
                        .written_off_minor
                        .checked_add(adjustment.amount_minor)
                        .ok_or_else(|| invalid("write-off total overflows minor units"))?;
                }
            }
            head.outstanding_minor = invoice_outstanding_minor(&head)?;
            head.source_event_id = event.id.to_hex();
            let d_tag = invoice_head_d_tag(adjustment.client_id, adjustment.invoice_id);
            let expected_id = head_event.event.id.to_bytes().to_vec();
            heads.push(HeadWrite {
                event: relay_head_event(
                    KIND_INVOICE_HEAD,
                    channel_id,
                    &d_tag,
                    &head,
                    Some(&head_event),
                    state,
                )?,
                channel_id,
                d_tag: d_tag.clone(),
                expected_event_id: Some(expected_id.clone()),
            });
            expected_heads.push(ExpectedHead {
                kind: KIND_INVOICE_HEAD,
                d_tag,
                event_id: expected_id,
            });
        }
        BusinessCommand::MoneyFollowUp(action) => {
            validate_money_follow_up(&action)?;
            let (invoice_event, invoice) = current_invoice_head(
                state,
                tenant.community(),
                channel_id,
                action.client_id,
                action.invoice_id,
            )
            .await?;
            if action.expected_invoice_head_event_id != invoice_event.event.id.to_hex() {
                return Err(conflict(
                    "invoice changed before the follow-up was reviewed",
                ));
            }
            let now = chrono::Utc::now().timestamp();
            if invoice.status != InvoiceStatus::Issued
                || invoice.outstanding_minor <= 0
                || invoice.due_at.is_none_or(|due_at| due_at >= now)
            {
                return Err(conflict(
                    "follow-ups require an issued overdue invoice balance",
                ));
            }
            let d_tag = money_follow_up_d_tag(action.client_id, action.follow_up_id);
            let current = current_head::<MoneyFollowUpHead>(
                state,
                tenant.community(),
                KIND_MONEY_FOLLOW_UP_HEAD,
                &d_tag,
            )
            .await?;
            if let Some(stored) = current.as_ref() {
                if stored.channel_id != Some(channel_id) {
                    return Err(forbidden(
                        "follow-up is outside the authorized client channel",
                    ));
                }
                let stored_head: MoneyFollowUpHead = parse_content(&stored.event)?;
                if stored_head.client_id != action.client_id
                    || stored_head.invoice_id != action.invoice_id
                    || stored_head.follow_up_id != action.follow_up_id
                {
                    return Err(conflict(
                        "follow-up head coordinate does not match its content",
                    ));
                }
            }
            let (head, previous) = match (action.action, current) {
                (MoneyFollowUpActionKind::Draft, None)
                    if action.expected_head_event_id.is_none() =>
                {
                    (
                        MoneyFollowUpHead {
                            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
                            client_id: action.client_id,
                            invoice_id: action.invoice_id,
                            follow_up_id: action.follow_up_id,
                            status: MoneyFollowUpStatus::Draft,
                            version: 1,
                            current_version_event_id: event.id.to_hex(),
                            due_at: action.due_at,
                            draft_content: action.draft_content.clone(),
                            approval_intent_only: false,
                            approved_by_pubkey: None,
                            approved_at: None,
                            source_event_id: event.id.to_hex(),
                        },
                        None,
                    )
                }
                (MoneyFollowUpActionKind::Review, Some(stored))
                    if action.expected_head_event_id.as_deref()
                        == Some(stored.event.id.to_hex().as_str()) =>
                {
                    let mut head: MoneyFollowUpHead = parse_content(&stored.event)?;
                    if head.client_id != action.client_id
                        || head.invoice_id != action.invoice_id
                        || head.status != MoneyFollowUpStatus::Draft
                        || head.draft_content != action.draft_content
                        || head.due_at != action.due_at
                    {
                        return Err(conflict("follow-up draft changed before review"));
                    }
                    head.status = MoneyFollowUpStatus::InReview;
                    head.version = head
                        .version
                        .checked_add(1)
                        .ok_or_else(|| invalid("follow-up version overflows"))?;
                    head.current_version_event_id = event.id.to_hex();
                    head.source_event_id = event.id.to_hex();
                    (head, Some(stored))
                }
                (MoneyFollowUpActionKind::Approve, Some(stored))
                    if action.expected_head_event_id.as_deref()
                        == Some(stored.event.id.to_hex().as_str()) =>
                {
                    let mut head: MoneyFollowUpHead = parse_content(&stored.event)?;
                    if head.client_id != action.client_id
                        || head.invoice_id != action.invoice_id
                        || head.status != MoneyFollowUpStatus::InReview
                        || head.draft_content != action.draft_content
                        || head.due_at != action.due_at
                    {
                        return Err(conflict("follow-up draft changed before approval"));
                    }
                    head.status = MoneyFollowUpStatus::Approved;
                    head.version = head
                        .version
                        .checked_add(1)
                        .ok_or_else(|| invalid("follow-up version overflows"))?;
                    head.current_version_event_id = event.id.to_hex();
                    head.approval_intent_only = true;
                    head.approved_by_pubkey = Some(auth.pubkey().to_hex());
                    head.approved_at = Some(event.created_at.as_secs() as i64);
                    head.source_event_id = event.id.to_hex();
                    (head, Some(stored))
                }
                (MoneyFollowUpActionKind::Draft, Some(_)) => {
                    return Err(conflict("follow-up already exists"));
                }
                (_, None) => return Err(conflict("follow-up does not exist")),
                _ => return Err(conflict("follow-up action is not valid in this state")),
            };
            if let Some(previous) = previous.as_ref() {
                expected_heads.push(ExpectedHead {
                    kind: KIND_MONEY_FOLLOW_UP_HEAD,
                    d_tag: d_tag.clone(),
                    event_id: previous.event.id.to_bytes().to_vec(),
                });
            }
            let expected_id = previous
                .as_ref()
                .map(|stored| stored.event.id.to_bytes().to_vec());
            let head_event = relay_head_event(
                KIND_MONEY_FOLLOW_UP_HEAD,
                channel_id,
                &d_tag,
                &head,
                previous.as_ref(),
                state,
            )?;
            heads.push(HeadWrite {
                event: head_event,
                channel_id,
                d_tag,
                expected_event_id: expected_id,
            });
            expected_heads.push(ExpectedHead {
                kind: KIND_INVOICE_HEAD,
                d_tag: invoice_head_d_tag(action.client_id, action.invoice_id),
                event_id: invoice_event.event.id.to_bytes().to_vec(),
            });
        }
    }

    for head in &heads {
        require_token_channel_scope(&auth, head.channel_id)?;
    }
    for (_, emitted_channel_id) in &appended {
        require_token_channel_scope(&auth, *emitted_channel_id)?;
    }
    if let Some(client_channel) = client_channel_to_create.as_ref() {
        require_token_channel_scope(&auth, client_channel.channel_id)?;
    }
    if let Some(client_channel_id) = client_channel_to_sync {
        require_token_channel_scope(&auth, client_channel_id)?;
    }

    let result = persist(
        tenant,
        state,
        PersistBatch {
            command: event,
            channel_id,
            heads,
            appended,
            expected_heads,
            conversion_claim,
            client_channel_to_create,
            business_channel_to_register,
        },
    )
    .await?;
    if let Some(client_channel_id) = client_channel_to_sync {
        ensure_client_group_discovery(tenant, state, &auth, client_channel_id).await?;
    }
    Ok(result)
}

async fn replay_existing_command(
    state: &AppState,
    tenant: &TenantContext,
    event: &Event,
    channel_id: Uuid,
) -> Result<Option<IngestResult>, IngestError> {
    let existing = state
        .db
        .get_event_by_id_for_event_write(tenant.community(), &event.id.to_bytes())
        .await
        .map_err(internal)?;
    let Some(existing) = existing else {
        return Ok(None);
    };
    if existing.event.pubkey != event.pubkey || existing.channel_id != Some(channel_id) {
        return Err(forbidden(
            "command replay must match its original author and channel",
        ));
    }
    Ok(Some(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: "duplicate: already processed".into(),
    }))
}

fn initializes_business_channel(command: &BusinessCommand) -> bool {
    matches!(
        command,
        BusinessCommand::PartyAction(action) if action.action == RecordAction::Create
    ) || matches!(
        command,
        BusinessCommand::ServiceAction(action) if action.action == RecordAction::Create
    ) || matches!(
        command,
        BusinessCommand::ProspectAction(action) if action.action == RecordAction::Create
    )
}

pub(super) fn require_token_channel_scope(
    auth: &IngestAuth,
    channel_id: Uuid,
) -> Result<(), IngestError> {
    if auth
        .channel_ids()
        .is_some_and(|channel_ids| !channel_ids.contains(&channel_id))
    {
        return Err(IngestError::AuthFailed(
            "restricted: token is not scoped to this channel".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
fn install_party_validation_test_hook(
    event_id: [u8; 32],
    ready: Arc<tokio::sync::Notify>,
    resume: Arc<tokio::sync::Notify>,
) {
    let slot = PARTY_VALIDATION_TEST_HOOK.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut slot) = slot.lock() {
        *slot = Some((event_id, ready, resume));
    }
}

#[cfg(test)]
async fn after_party_validation_for_test(event_id: [u8; 32]) {
    let hook = PARTY_VALIDATION_TEST_HOOK.get().and_then(|slot| {
        let mut slot = slot.lock().ok()?;
        slot.as_ref()
            .is_some_and(|(target, _, _)| *target == event_id)
            .then(|| slot.take())
            .flatten()
    });
    if let Some((_, ready, resume)) = hook {
        ready.notify_one();
        resume.notified().await;
    }
}

#[cfg(test)]
fn install_expected_head_lock_test_hook(
    paused_event_id: [u8; 32],
    paused: Arc<tokio::sync::Notify>,
    resume: Arc<tokio::sync::Notify>,
    other_lock_attempt: Arc<tokio::sync::Notify>,
    other_first_lock: Arc<tokio::sync::Notify>,
) {
    let slot = EXPECTED_HEAD_LOCK_TEST_HOOK.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut slot) = slot.lock() {
        *slot = Some((
            paused_event_id,
            paused,
            resume,
            other_lock_attempt,
            other_first_lock,
        ));
    }
}

#[cfg(test)]
async fn before_expected_head_lock_for_test(event_id: [u8; 32]) {
    let hook = EXPECTED_HEAD_LOCK_TEST_HOOK.get().and_then(|slot| {
        let slot = slot.lock().ok()?;
        slot.as_ref()
            .map(|(paused_id, _, _, attempt, _)| (*paused_id, attempt.clone()))
    });
    if let Some((paused_id, other_lock_attempt)) = hook {
        if event_id != paused_id {
            other_lock_attempt.notify_one();
        }
    }
}

#[cfg(test)]
async fn after_first_expected_head_lock_for_test(event_id: [u8; 32]) {
    let hook = EXPECTED_HEAD_LOCK_TEST_HOOK.get().and_then(|slot| {
        let slot = slot.lock().ok()?;
        slot.as_ref().map(|(paused_id, paused, resume, _, other)| {
            (*paused_id, paused.clone(), resume.clone(), other.clone())
        })
    });
    if let Some((paused_id, paused, resume, other_first_lock)) = hook {
        if event_id == paused_id {
            paused.notify_one();
            resume.notified().await;
        } else {
            other_first_lock.notify_one();
        }
    }
}

#[cfg(not(test))]
async fn after_party_validation_for_test(_: [u8; 32]) {}

#[cfg(not(test))]
async fn after_first_expected_head_lock_for_test(_: [u8; 32]) {}

#[cfg(not(test))]
async fn before_expected_head_lock_for_test(_: [u8; 32]) {}

#[cfg(test)]
struct ExpectedHeadLockTestHookGuard {
    resume: Arc<tokio::sync::Notify>,
}

#[cfg(test)]
impl Drop for ExpectedHeadLockTestHookGuard {
    fn drop(&mut self) {
        self.resume.notify_one();
        if let Some(slot) = EXPECTED_HEAD_LOCK_TEST_HOOK.get() {
            if let Ok(mut slot) = slot.lock() {
                *slot = None;
            }
        }
    }
}

#[cfg(test)]
struct SecondaryChannelReadTestHookGuard {
    observed: Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(test)]
impl SecondaryChannelReadTestHookGuard {
    fn was_observed(&self) -> bool {
        self.observed.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[cfg(test)]
impl Drop for SecondaryChannelReadTestHookGuard {
    fn drop(&mut self) {
        if let Some(slot) = SECONDARY_CHANNEL_READ_TEST_HOOK.get() {
            if let Ok(mut slot) = slot.lock() {
                *slot = None;
            }
        }
    }
}

#[cfg(test)]
fn install_secondary_channel_read_test_hook(channel_id: Uuid) -> SecondaryChannelReadTestHookGuard {
    let observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let slot = SECONDARY_CHANNEL_READ_TEST_HOOK.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut slot) = slot.lock() {
        *slot = Some((channel_id, Arc::clone(&observed)));
    }
    SecondaryChannelReadTestHookGuard { observed }
}

#[cfg(test)]
fn before_secondary_channel_read_for_test(channel_id: Uuid) {
    let hook = SECONDARY_CHANNEL_READ_TEST_HOOK.get().and_then(|slot| {
        let slot = slot.lock().ok()?;
        slot.as_ref()
            .map(|(target, observed)| (*target, Arc::clone(observed)))
    });
    if let Some((target, observed)) = hook {
        if target == channel_id {
            observed.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

struct AcceptancePlan {
    heads: Vec<HeadWrite>,
    appended: Vec<(Event, Uuid)>,
    current_proposal_head: ExpectedHead,
    current_party_head: ExpectedHead,
    expected_prospect_head: Option<ExpectedHead>,
    claim: ConversionClaim,
    client_channel_id: Uuid,
    client_channel_to_create: Option<ClientChannelProvision>,
}

async fn existing_proposal_conversion_replay(
    tenant: &TenantContext,
    state: &AppState,
    auth: &IngestAuth,
    business_channel_id: Uuid,
    acceptance_event: &Event,
    acceptance: &ProposalAcceptance,
) -> Result<Option<IngestResult>, IngestError> {
    validate_acceptance_ids(acceptance)?;
    let version_event_id = decode_hash(&acceptance.proposal_version_event_id)?;
    let version_digest = decode_hash(&acceptance.proposal_version_digest)?;
    let accepted_by_pubkey = auth.pubkey().to_bytes();
    let existing = state
        .db
        .find_proposal_conversion_claim(
            tenant.community(),
            &buzz_db::business_records::ProposalConversionKey {
                business_channel_id,
                conversion_id: acceptance.conversion_id,
                proposal_id: acceptance.proposal_id,
                proposal_version_event_id: version_event_id,
                proposal_version_digest: version_digest,
                accepted_by_pubkey: accepted_by_pubkey.to_vec(),
                client_id: acceptance.client_id,
                work_item_id: acceptance.work_item_id,
                draft_invoice_id: acceptance.draft_invoice_id,
            },
        )
        .await
        .map_err(|error| match error {
            DbError::InvalidData(_) => {
                conflict("proposal conversion id was already claimed with different values")
            }
            other => internal(other),
        })?;
    Ok(existing.map(|claim| IngestResult {
        event_id: acceptance_event.id.to_hex(),
        accepted: true,
        message: format!(
            "duplicate: conversion already claimed; receipt={}",
            hex::encode(claim.receipt_event_id)
        ),
    }))
}

async fn prepare_proposal_acceptance(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    auth: &IngestAuth,
    business_channel_id: Uuid,
    acceptance_event: &Event,
    acceptance: ProposalAcceptance,
) -> Result<AcceptancePlan, IngestError> {
    validate_acceptance_ids(&acceptance)?;
    let version_id = decode_hash(&acceptance.proposal_version_event_id)?;
    let stored_version = state
        .db
        .get_event_by_id_for_event_write(tenant.community(), &version_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| conflict("proposal version does not exist"))?;
    if stored_version.event.kind.as_u16() as u32 != KIND_PROPOSAL_VERSION {
        return Err(conflict("referenced event is not a proposal version"));
    }
    let version: ProposalVersion = parse_content(&stored_version.event)?;
    if version.proposal_id != acceptance.proposal_id
        || digest_hex(stored_version.event.content.as_bytes()) != acceptance.proposal_version_digest
    {
        return Err(conflict("proposal version id or digest does not match"));
    }
    if version
        .expires_at
        .is_some_and(|expires| expires <= chrono::Utc::now().timestamp())
    {
        return Err(conflict("proposal version has expired"));
    }
    if version.named_acceptor_pubkey != auth.pubkey().to_hex() {
        return Err(forbidden(
            "only the named proposal acceptor can sign acceptance",
        ));
    }
    validate_proposal_acceptance_evidence(&acceptance)?;
    let proposal_head_d = business_d_tag(
        *tenant.community().as_uuid(),
        "proposal",
        acceptance.proposal_id,
    );
    let proposal_head = current_head::<ProposalHead>(
        state,
        tenant.community(),
        KIND_PROPOSAL_HEAD,
        &proposal_head_d,
    )
    .await?
    .ok_or_else(|| conflict("proposal has no current version"))?;
    let current_proposal: ProposalHead = parse_content(&proposal_head.event)?;
    if proposal_head.channel_id != Some(business_channel_id)
        || current_proposal.current_version_event_id != acceptance.proposal_version_event_id
        || current_proposal.current_version_digest != acceptance.proposal_version_digest
    {
        return Err(conflict(
            "acceptance must target the current proposal version",
        ));
    }
    let version_d = proposal_version_d_tag(
        *tenant.community().as_uuid(),
        acceptance.proposal_id,
        version.revision,
    );
    let (version_event_channel, version_event_d_tag) = command_coordinates(&stored_version.event)?;
    if version_event_d_tag != version_d
        || version_event_channel != business_channel_id
        || stored_version.channel_id != Some(business_channel_id)
    {
        return Err(conflict(
            "proposal version is outside the active business channel",
        ));
    }

    let party_d = business_d_tag(
        *tenant.community().as_uuid(),
        "party",
        version.prospect_party_id,
    );
    let party_stored = current_head_in_channel::<PartyHead>(
        state,
        tenant.community(),
        business_channel_id,
        KIND_PARTY_HEAD,
        &party_d,
    )
    .await?
    .ok_or_else(|| conflict("proposal prospect party is not available"))?;
    let party: PartyHead = parse_content(&party_stored.event)?;
    if party.party_id != version.prospect_party_id
        || party.status != "active"
        || party_stored.channel_id != Some(business_channel_id)
    {
        return Err(conflict("proposal prospect party is not available"));
    }
    after_party_validation_for_test(acceptance_event.id.to_bytes()).await;
    let current_party_head = ExpectedHead {
        kind: KIND_PARTY_HEAD,
        d_tag: party_d,
        event_id: party_stored.event.id.to_bytes().to_vec(),
    };

    let prospect_d = prospect_d_tag(*tenant.community().as_uuid(), version.prospect_party_id);
    let prospect_stored =
        current_head::<ProspectHead>(state, tenant.community(), KIND_PROSPECT_HEAD, &prospect_d)
            .await?;
    let (prospect_head_write, expected_prospect_head) = match prospect_stored {
        Some(stored) if stored.channel_id == Some(business_channel_id) => {
            let mut prospect_head: ProspectHead = parse_content(&stored.event)?;
            if prospect_head.status != "active"
                || prospect_head.prospect.party.party_id != version.prospect_party_id
            {
                return Err(conflict("proposal prospect is not available"));
            }
            let expected = ExpectedHead {
                kind: KIND_PROSPECT_HEAD,
                d_tag: prospect_d.clone(),
                event_id: stored.event.id.to_bytes().to_vec(),
            };
            prospect_head.prospect.stage = ProspectStage::Won;
            prospect_head.prospect.lost_reason = None;
            prospect_head.source_action_event_id = acceptance_event.id.to_hex();
            let write = HeadWrite {
                event: relay_head_event(
                    KIND_PROSPECT_HEAD,
                    business_channel_id,
                    &prospect_d,
                    &prospect_head,
                    Some(&stored),
                    state,
                )?,
                channel_id: business_channel_id,
                d_tag: prospect_d,
                expected_event_id: Some(stored.event.id.to_bytes().to_vec()),
            };
            (Some(write), Some(expected))
        }
        Some(_) => {
            return Err(conflict(
                "proposal prospect belongs to another business channel",
            ));
        }
        None => (None, None),
    };

    #[cfg(test)]
    before_secondary_channel_read_for_test(acceptance.client_id);
    let client_channel_to_create = match state
        .db
        .get_channel_for_event_write(tenant.community(), acceptance.client_id)
        .await
    {
        Ok(channel) => {
            if channel.visibility != "private"
                || channel.channel_type != "stream"
                || channel.archived_at.is_some()
            {
                return Err(forbidden(
                    "business records require a private stream client channel",
                ));
            }
            let acceptor_role = state
                .db
                .get_member_role(tenant.community(), channel.id, &auth.pubkey().to_bytes())
                .await
                .map_err(internal)?
                .ok_or_else(|| forbidden("named acceptor is not a member of the client channel"))?;
            require_acceptor_channel_role(&acceptor_role)?;
            None
        }
        Err(DbError::ChannelNotFound(_)) => {
            let name = buzz_core::channel::canonical_channel_name(&party.party.display_name);
            if name.is_empty() {
                return Err(invalid("client channel name is required"));
            }
            Some(ClientChannelProvision {
                channel_id: acceptance.client_id,
                name: name.to_owned(),
            })
        }
        Err(error) => return Err(internal(error)),
    };

    let client_d = client_d_tag(acceptance.client_id, "client", acceptance.client_id);
    let work_d = client_d_tag(acceptance.client_id, "work", acceptance.work_item_id);
    let invoice_d = invoice_head_d_tag(acceptance.client_id, acceptance.draft_invoice_id);

    let named_acceptor = auth.pubkey().to_hex();
    let client_head = ClientHead {
        schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
        client_id: acceptance.client_id,
        party_id: party.party_id,
        display_name: party.party.display_name.clone(),
        approver_pubkeys: vec![named_acceptor.clone()],
        status: "active".into(),
        source_action_event_id: acceptance_event.id.to_hex(),
    };
    let work_item = WorkItemHead {
        schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
        client_id: acceptance.client_id,
        work_item_id: acceptance.work_item_id,
        title: format!("Proposal {}", acceptance.proposal_id),
        status: "open".into(),
        assigned_pubkeys: Vec::new(),
        approver_pubkeys: vec![named_acceptor],
        deliverables: Vec::new(),
        source_event_id: acceptance_event.id.to_hex(),
    };
    let total_minor = invoice_total_minor(&version.lines, &[])
        .ok_or_else(|| invalid("proposal total overflows minor units"))?;
    let invoice_version = InvoiceVersion {
        schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
        client_id: acceptance.client_id,
        invoice_id: acceptance.draft_invoice_id,
        version: 1,
        previous_version_event_id: None,
        proposal_version_event_id: Some(acceptance.proposal_version_event_id.clone()),
        expected_head_event_id: None,
        action: InvoiceVersionAction::ProposalAcceptance,
        currency: version.currency.clone(),
        lines: version.lines.clone(),
        tax_lines: Vec::new(),
        seller_tax_number: None,
        customer_tax_number: None,
        total_minor,
        status: InvoiceStatus::Draft,
        due_at: None,
        void_reason: None,
    };
    let invoice_version_d = invoice_version_d_tag(
        acceptance.client_id,
        acceptance.draft_invoice_id,
        invoice_version.version,
    );
    let invoice_version_event = relay_event(
        KIND_INVOICE_VERSION,
        acceptance.client_id,
        &invoice_version_d,
        &invoice_version,
        state,
    )?;
    let invoice = InvoiceHead {
        schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
        client_id: acceptance.client_id,
        invoice_id: acceptance.draft_invoice_id,
        proposal_id: acceptance.proposal_id,
        proposal_version_event_id: acceptance.proposal_version_event_id.clone(),
        currency: version.currency.clone(),
        lines: version.lines.clone(),
        tax_lines: Vec::new(),
        seller_tax_number: None,
        customer_tax_number: None,
        total_minor,
        credited_minor: 0,
        written_off_minor: 0,
        collected_minor: 0,
        outstanding_minor: 0,
        payment_evidence_count: 0,
        version: invoice_version.version,
        current_version_event_id: invoice_version_event.id.to_hex(),
        status: InvoiceStatus::Draft,
        due_at: None,
        issued_at: None,
        source_event_id: acceptance_event.id.to_hex(),
    };
    let receipt_content = buzz_core::business_records::ProposalConversionReceipt {
        schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
        conversion_id: acceptance.conversion_id,
        proposal_id: acceptance.proposal_id,
        proposal_version_event_id: acceptance.proposal_version_event_id.clone(),
        client_id: acceptance.client_id,
        work_item_id: acceptance.work_item_id,
        draft_invoice_id: acceptance.draft_invoice_id,
        acceptance_event_id: acceptance_event.id.to_hex(),
    };
    let receipt_d = business_d_tag(
        *tenant.community().as_uuid(),
        "conversion",
        acceptance.conversion_id,
    );
    let receipt = relay_event(
        KIND_PROPOSAL_CONVERSION_RECEIPT,
        business_channel_id,
        &receipt_d,
        &receipt_content,
        state,
    )?;
    let claim = ConversionClaim {
        conversion_id: acceptance.conversion_id,
        proposal_id: acceptance.proposal_id,
        business_channel_id,
        version_event_id: version_id,
        version_digest: decode_hash(&acceptance.proposal_version_digest)?,
        acceptance_event_id: acceptance_event.id.to_bytes().to_vec(),
        receipt_event_id: receipt.id.to_bytes().to_vec(),
        accepted_by_pubkey: auth.pubkey().to_bytes().to_vec(),
        client_id: acceptance.client_id,
        work_item_id: acceptance.work_item_id,
        draft_invoice_id: acceptance.draft_invoice_id,
    };
    let expected_proposal_head = ExpectedHead {
        kind: KIND_PROPOSAL_HEAD,
        d_tag: proposal_head_d,
        event_id: proposal_head.event.id.to_bytes().to_vec(),
    };
    let mut heads = vec![
        HeadWrite {
            event: relay_event(
                KIND_CLIENT_HEAD,
                acceptance.client_id,
                &client_d,
                &client_head,
                state,
            )?,
            channel_id: acceptance.client_id,
            d_tag: client_d,
            expected_event_id: None,
        },
        HeadWrite {
            event: relay_event(
                KIND_WORK_ITEM_HEAD,
                acceptance.client_id,
                &work_d,
                &work_item,
                state,
            )?,
            channel_id: acceptance.client_id,
            d_tag: work_d,
            expected_event_id: None,
        },
        HeadWrite {
            event: relay_event(
                KIND_INVOICE_HEAD,
                acceptance.client_id,
                &invoice_d,
                &invoice,
                state,
            )?,
            channel_id: acceptance.client_id,
            d_tag: invoice_d,
            expected_event_id: None,
        },
    ];
    if let Some(prospect_head) = prospect_head_write {
        heads.push(prospect_head);
    }
    Ok(AcceptancePlan {
        heads,
        appended: vec![
            (receipt, business_channel_id),
            (invoice_version_event, acceptance.client_id),
        ],
        current_proposal_head: expected_proposal_head,
        current_party_head,
        expected_prospect_head,
        claim,
        client_channel_id: acceptance.client_id,
        client_channel_to_create,
    })
}

fn validate_acceptance_ids(acceptance: &ProposalAcceptance) -> Result<(), IngestError> {
    if acceptance.proposal_id.is_nil()
        || acceptance.conversion_id.is_nil()
        || acceptance.client_id.is_nil()
        || acceptance.work_item_id.is_nil()
        || acceptance.draft_invoice_id.is_nil()
    {
        return Err(invalid("proposal acceptance identifiers must not be nil"));
    }
    validate_hex_reference(&acceptance.proposal_version_event_id)
        .map_err(|error| invalid(error.to_string()))?;
    validate_hex_reference(&acceptance.proposal_version_digest)
        .map_err(|error| invalid(error.to_string()))?;
    Ok(())
}

fn validate_proposal_acceptance_evidence(
    acceptance: &ProposalAcceptance,
) -> Result<(), IngestError> {
    let Some(evidence) = acceptance.evidence.as_ref() else {
        return Err(invalid("proposal acceptance evidence is required"));
    };
    if evidence.accepted_by_name.trim().is_empty()
        || evidence.accepted_by_name.len() > 240
        || evidence.accepted_at <= 0
        || evidence.evidence_reference.trim().is_empty()
        || evidence.evidence_reference.len() > 1_000
        || !evidence.exact_terms_confirmed
    {
        return Err(invalid("proposal acceptance evidence is incomplete"));
    }
    Ok(())
}

async fn persist(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    batch: PersistBatch,
) -> Result<IngestResult, IngestError> {
    let PersistBatch {
        command,
        channel_id,
        heads,
        appended,
        mut expected_heads,
        conversion_claim,
        client_channel_to_create,
        business_channel_to_register,
    } = batch;
    let mut tx = state
        .db
        .begin_event_write_transaction()
        .await
        .map_err(internal)?;
    buzz_deletion::store(&state.db)
        .guard_transaction(&mut tx, tenant.community())
        .await
        .map_err(|error| {
            IngestError::Rejected(format!("restricted: community writes are fenced: {error}"))
        })?;

    if let Some(channel_id) = business_channel_to_register {
        let registered = buzz_db::business_records::register_business_channel_in_transaction(
            &mut tx,
            tenant.community(),
            channel_id,
        )
        .await
        .map_err(internal)?;
        if !registered {
            tx.rollback().await.map_err(internal)?;
            return Err(conflict(
                "another business channel was registered before this Party command committed",
            ));
        }
    }

    expected_heads.sort_by(|left, right| {
        left.kind
            .cmp(&right.kind)
            .then_with(|| left.d_tag.cmp(&right.d_tag))
    });
    for (index, expected) in expected_heads.into_iter().enumerate() {
        before_expected_head_lock_for_test(command.id.to_bytes()).await;
        let actual = buzz_db::replaceable::lock_parameterized_event_head_in_transaction(
            &mut tx,
            tenant.community(),
            expected.kind,
            &state.relay_keypair.public_key().to_bytes(),
            &expected.d_tag,
        )
        .await
        .map_err(internal)?;
        if actual.as_deref() != Some(expected.event_id.as_slice()) {
            tx.rollback().await.map_err(internal)?;
            return Err(conflict(
                "referenced business record changed before the command committed",
            ));
        }
        if index == 0 {
            after_first_expected_head_lock_for_test(command.id.to_bytes()).await;
        }
    }

    let mut conversion_receipt_event_id = None;
    if let Some(claim) = conversion_claim {
        let db_claim = buzz_db::business_records::ProposalConversionClaim {
            key: buzz_db::business_records::ProposalConversionKey {
                business_channel_id: claim.business_channel_id,
                conversion_id: claim.conversion_id,
                proposal_id: claim.proposal_id,
                proposal_version_event_id: claim.version_event_id,
                proposal_version_digest: claim.version_digest,
                accepted_by_pubkey: claim.accepted_by_pubkey,
                client_id: claim.client_id,
                work_item_id: claim.work_item_id,
                draft_invoice_id: claim.draft_invoice_id,
            },
            acceptance_event_id: claim.acceptance_event_id,
            receipt_event_id: claim.receipt_event_id,
        };
        let result = buzz_db::business_records::claim_proposal_conversion(
            &mut tx,
            tenant.community(),
            &db_claim,
        )
        .await
        .map_err(|error| match error {
            DbError::InvalidData(_) => {
                conflict("proposal conversion identifiers were claimed with different values")
            }
            other => internal(other),
        })?;
        if result.disposition == buzz_db::business_records::ConversionClaimDisposition::Existing {
            tx.rollback().await.map_err(internal)?;
            return Ok(IngestResult {
                event_id: command.id.to_hex(),
                accepted: true,
                message: format!(
                    "duplicate: conversion already claimed; receipt={}",
                    hex::encode(result.receipt_event_id)
                ),
            });
        }
        conversion_receipt_event_id = Some(result.receipt_event_id);
    }

    if let Some(client_channel) = client_channel_to_create {
        let created = buzz_db::channel::create_client_channel_in_transaction(
            &mut tx,
            tenant.community(),
            client_channel.channel_id,
            &client_channel.name,
            &command.pubkey.to_bytes(),
        )
        .await
        .map_err(internal)?;
        if !created {
            tx.rollback().await.map_err(internal)?;
            return Err(conflict(
                "client channel was created concurrently; retry proposal acceptance",
            ));
        }
    }

    let (stored_command, inserted) = buzz_db::event::insert_event_in_transaction(
        &mut tx,
        tenant.community(),
        &command,
        Some(channel_id),
    )
    .await
    .map_err(internal)?;
    if !inserted {
        tx.rollback().await.map_err(internal)?;
        return Ok(IngestResult {
            event_id: command.id.to_hex(),
            accepted: true,
            message: "duplicate: already processed".into(),
        });
    }

    let actor_hex = command.pubkey.to_hex();
    let mut stored_events = vec![(stored_command, actor_hex)];
    for (appended_event, appended_channel_id) in appended {
        let (stored, was_inserted) = buzz_db::event::insert_event_in_transaction(
            &mut tx,
            tenant.community(),
            &appended_event,
            Some(appended_channel_id),
        )
        .await
        .map_err(internal)?;
        if was_inserted {
            stored_events.push((stored, appended_event.pubkey.to_hex()));
        }
    }

    for head in heads {
        let precondition = head.expected_event_id.as_deref().map_or(
            ParameterizedReplacePrecondition::CreateOnly,
            ParameterizedReplacePrecondition::ExpectedRevision,
        );
        let replaced = state
            .db
            .replace_parameterized_event_in_transaction(
                &mut tx,
                tenant.community(),
                &head.event,
                &head.d_tag,
                Some(head.channel_id),
                precondition,
            )
            .await
            .map_err(internal)?;
        match replaced.status {
            ParameterizedReplaceStatus::Inserted => {
                stored_events.push((replaced.event, head.event.pubkey.to_hex()));
            }
            ParameterizedReplaceStatus::Duplicate => {}
            ParameterizedReplaceStatus::RevisionMismatch
            | ParameterizedReplaceStatus::RevisionMissing
            | ParameterizedReplaceStatus::Superseded
            | ParameterizedReplaceStatus::ReplayOnlyMiss => {
                tx.rollback().await.map_err(internal)?;
                return Err(conflict(
                    "target business record changed before the command committed",
                ));
            }
        }
    }
    tx.commit().await.map_err(internal)?;

    for (stored, actor) in stored_events {
        super::event::dispatch_persistent_event(
            tenant,
            state,
            &stored,
            stored.event.kind.as_u16() as u32,
            &actor,
            None,
        )
        .await;
    }

    Ok(IngestResult {
        event_id: command.id.to_hex(),
        accepted: true,
        message: conversion_receipt_event_id.map_or_else(String::new, |receipt_event_id| {
            format!("receipt={}", hex::encode(receipt_event_id))
        }),
    })
}

async fn ensure_client_group_discovery(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    auth: &IngestAuth,
    client_channel_id: Uuid,
) -> Result<(), IngestError> {
    require_token_channel_scope(auth, client_channel_id)?;
    let channel = get_private_stream_channel(state, tenant.community(), client_channel_id).await?;
    let actor = auth.pubkey().to_bytes();
    let role = state
        .db
        .get_member_role(tenant.community(), client_channel_id, &actor)
        .await
        .map_err(internal)?
        .ok_or_else(|| forbidden("named acceptor is not a member of the client channel"))?;
    require_acceptor_channel_role(&role)?;
    state.invalidate_membership(tenant, client_channel_id, &actor);
    crate::handlers::side_effects::emit_group_discovery_events(tenant, state, channel.id)
        .await
        .map_err(internal)
}

async fn get_private_stream_channel(
    state: &AppState,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<buzz_db::channel::ChannelRecord, IngestError> {
    let channel = match state
        .db
        .get_channel_for_event_write(community_id, channel_id)
        .await
    {
        Ok(channel) => channel,
        Err(DbError::ChannelNotFound(_)) => return Err(invalid("business channel does not exist")),
        Err(error) => return Err(internal(error)),
    };
    if channel.visibility != "private"
        || channel.channel_type != "stream"
        || channel.archived_at.is_some()
    {
        return Err(forbidden(
            "business records require a private stream channel",
        ));
    }
    Ok(channel)
}

pub(super) async fn get_company_work_stream_channel(
    state: &AppState,
    community_id: CommunityId,
    channel_id: Uuid,
) -> Result<buzz_db::channel::ChannelRecord, IngestError> {
    let channel = match state
        .db
        .get_channel_for_event_write(community_id, channel_id)
        .await
    {
        Ok(channel) => channel,
        Err(DbError::ChannelNotFound(_)) => {
            return Err(invalid("company work channel does not exist"))
        }
        Err(error) => return Err(internal(error)),
    };
    if channel.channel_type != "stream" || channel.archived_at.is_some() {
        return Err(forbidden("company work requires an active stream channel"));
    }
    Ok(channel)
}

pub(super) fn command_coordinates(event: &Event) -> Result<(Uuid, String), IngestError> {
    let h_tags = event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == "h")
        .collect::<Vec<_>>();
    let d_tags = event
        .tags
        .iter()
        .filter(|tag| tag.kind().to_string() == "d")
        .collect::<Vec<_>>();
    if h_tags.len() != 1 || d_tags.len() != 1 {
        return Err(invalid(
            "business command requires exactly one h tag and one d tag",
        ));
    }
    let h_parts = h_tags[0].as_slice();
    let d_parts = d_tags[0].as_slice();
    if h_parts.len() != 2 || d_parts.len() != 2 {
        return Err(invalid(
            "business h and d tags must each have exactly two values",
        ));
    }
    let channel_id = Uuid::parse_str(h_parts[1].as_str())
        .map_err(|_| invalid("business h tag must contain a channel UUID"))?;
    Ok((channel_id, d_parts[1].to_string()))
}

pub(super) async fn current_head<T: DeserializeOwned>(
    state: &AppState,
    community_id: CommunityId,
    kind: u32,
    d_tag: &str,
) -> Result<Option<StoredEvent>, IngestError> {
    let mut query = EventQuery::for_community(community_id);
    query.kinds = Some(vec![kind as i32]);
    query.pubkey = Some(state.relay_keypair.public_key().to_bytes().to_vec());
    query.d_tag = Some(d_tag.to_string());
    query.limit = Some(2);
    let mut rows = state
        .db
        .query_events_for_event_write(&query)
        .await
        .map_err(internal)?;
    if rows.len() > 1 {
        return Err(IngestError::Internal(
            "error: duplicate business head coordinate".into(),
        ));
    }
    let event = rows.pop();
    if let Some(event) = event.as_ref() {
        let _: T = parse_content(&event.event)?;
    }
    Ok(event)
}

async fn current_head_in_channel<T: DeserializeOwned>(
    state: &AppState,
    community_id: CommunityId,
    channel_id: Uuid,
    kind: u32,
    d_tag: &str,
) -> Result<Option<StoredEvent>, IngestError> {
    let mut query = EventQuery::for_community(community_id);
    query.channel_id = Some(channel_id);
    query.kinds = Some(vec![kind as i32]);
    query.pubkey = Some(state.relay_keypair.public_key().to_bytes().to_vec());
    query.d_tag = Some(d_tag.to_string());
    query.limit = Some(2);
    let mut rows = state
        .db
        .query_events_for_event_write(&query)
        .await
        .map_err(internal)?;
    if rows.len() > 1 {
        return Err(IngestError::Internal(
            "error: duplicate business head coordinate".into(),
        ));
    }
    let event = rows.pop();
    if let Some(event) = event.as_ref() {
        let _: T = parse_content(&event.event)?;
    }
    Ok(event)
}

async fn current_invoice_head(
    state: &AppState,
    community_id: CommunityId,
    channel_id: Uuid,
    client_id: Uuid,
    invoice_id: Uuid,
) -> Result<(StoredEvent, InvoiceHead), IngestError> {
    let d_tag = invoice_head_d_tag(client_id, invoice_id);
    let event = current_head::<InvoiceHead>(state, community_id, KIND_INVOICE_HEAD, &d_tag)
        .await?
        .ok_or_else(|| conflict("invoice does not exist"))?;
    if event.channel_id != Some(channel_id) {
        return Err(forbidden(
            "invoice is outside the authorized client channel",
        ));
    }
    let head: InvoiceHead = parse_content(&event.event)?;
    if head.client_id != client_id || head.invoice_id != invoice_id {
        return Err(conflict(
            "invoice head coordinate does not match its content",
        ));
    }
    Ok((event, head))
}

async fn ensure_new_record_coordinate(
    state: &AppState,
    community_id: CommunityId,
    channel_id: Uuid,
    kind: u32,
    d_tag: &str,
) -> Result<(), IngestError> {
    let mut query = EventQuery::for_community(community_id);
    query.channel_id = Some(channel_id);
    query.kinds = Some(vec![kind as i32]);
    query.d_tag = Some(d_tag.to_owned());
    query.limit = Some(1);
    let rows = state
        .db
        .query_events_for_event_write(&query)
        .await
        .map_err(internal)?;
    if !rows.is_empty() {
        return Err(conflict("record identifier has already been used"));
    }
    Ok(())
}

async fn current_work_item(
    state: &AppState,
    community_id: CommunityId,
    channel_id: Uuid,
    client_id: Uuid,
    work_item_id: Uuid,
) -> Result<(StoredEvent, WorkItemHead), IngestError> {
    let d_tag = client_d_tag(client_id, "work", work_item_id);
    let event = current_head::<WorkItemHead>(state, community_id, KIND_WORK_ITEM_HEAD, &d_tag)
        .await?
        .ok_or_else(|| conflict("work item does not exist"))?;
    if event.channel_id != Some(channel_id) {
        return Err(conflict("work item is outside the client channel"));
    }
    let head: WorkItemHead = parse_content(&event.event)?;
    if head.client_id != client_id || head.work_item_id != work_item_id {
        return Err(conflict(
            "work-item head coordinate does not match its content",
        ));
    }
    Ok((event, head))
}

async fn load_version_event(
    state: &AppState,
    community_id: CommunityId,
    event_id_hex: &str,
    expected_kind: u32,
) -> Result<StoredEvent, IngestError> {
    validate_hex_reference(event_id_hex).map_err(|error| invalid(error.to_string()))?;
    let event_id = decode_hash(event_id_hex)?;
    let stored = state
        .db
        .get_event_by_id_for_event_write(community_id, &event_id)
        .await
        .map_err(internal)?
        .ok_or_else(|| conflict("referenced version event does not exist"))?;
    if stored.event.kind.as_u16() as u32 != expected_kind {
        return Err(conflict("referenced event has the wrong kind"));
    }
    Ok(stored)
}

fn relay_event<T: Serialize>(
    kind: u32,
    channel_id: Uuid,
    d_tag: &str,
    content: &T,
    state: &AppState,
) -> Result<Event, IngestError> {
    relay_event_at(
        kind,
        channel_id,
        d_tag,
        content,
        nostr::Timestamp::now(),
        state,
    )
}

pub(super) fn relay_head_event<T: Serialize>(
    kind: u32,
    channel_id: Uuid,
    d_tag: &str,
    content: &T,
    previous: Option<&StoredEvent>,
    state: &AppState,
) -> Result<Event, IngestError> {
    let now = nostr::Timestamp::now().as_secs();
    let created_at = previous.map_or(now, |stored| {
        now.max(stored.event.created_at.as_secs().saturating_add(1))
    });
    relay_event_at(
        kind,
        channel_id,
        d_tag,
        content,
        nostr::Timestamp::from_secs(created_at),
        state,
    )
}

pub(crate) fn relay_global_head_event<T: Serialize>(
    kind: u32,
    d_tag: &str,
    content: &T,
    previous: Option<&StoredEvent>,
    state: &AppState,
) -> Result<Event, IngestError> {
    let now = nostr::Timestamp::now().as_secs();
    let created_at = previous.map_or(now, |stored| {
        now.max(stored.event.created_at.as_secs().saturating_add(1))
    });
    let content = serde_json::to_string(content).map_err(internal)?;
    let d_tag = Tag::parse(["d", d_tag])
        .map_err(|error| internal(format!("company goal d tag: {error}")))?;
    EventBuilder::new(Kind::Custom(kind as u16), content)
        .tag(d_tag)
        .custom_created_at(nostr::Timestamp::from_secs(created_at))
        .sign_with_keys(&state.relay_keypair)
        .map_err(internal)
}

fn relay_event_at<T: Serialize>(
    kind: u32,
    channel_id: Uuid,
    d_tag: &str,
    content: &T,
    created_at: nostr::Timestamp,
    state: &AppState,
) -> Result<Event, IngestError> {
    let content = serde_json::to_string(content).map_err(internal)?;
    let h_tag = Tag::parse(["h", channel_id.to_string().as_str()])
        .map_err(|error| internal(format!("business h tag: {error}")))?;
    let d_tag =
        Tag::parse(["d", d_tag]).map_err(|error| internal(format!("business d tag: {error}")))?;
    EventBuilder::new(Kind::Custom(kind as u16), content)
        .tags([h_tag, d_tag])
        .custom_created_at(created_at)
        .sign_with_keys(&state.relay_keypair)
        .map_err(internal)
}

fn parse_content<T: DeserializeOwned>(event: &Event) -> Result<T, IngestError> {
    serde_json::from_str(&event.content).map_err(|_| {
        IngestError::Internal("error: stored business record content is invalid".into())
    })
}

fn validate_party(action: &PartyAction) -> Result<(), IngestError> {
    if action.party_id.is_nil()
        || action.party.party_id.is_nil()
        || action.party.party_type != "person" && action.party.party_type != "organization"
    {
        return Err(invalid("partyType must be person or organization"));
    }
    if action.party.display_name.trim().is_empty() {
        return Err(invalid("party displayName is required"));
    }
    Ok(())
}

fn validate_service_action(action: &ServiceAction) -> Result<(), IngestError> {
    let service = &action.service;
    if action.service_id.is_nil()
        || service.service_id != action.service_id
        || service.name.trim().is_empty()
        || service.name.len() > 120
        || service.description.len() > 2_000
        || service.currency.len() != 3
        || !service
            .currency
            .bytes()
            .all(|byte| byte.is_ascii_uppercase())
        || service.monthly_fee_minor < 0
        || service.posts_per_month == 0
        || service.revision_rounds > 20
    {
        return Err(invalid(
            "service definition has invalid name, fee, or scope",
        ));
    }
    validate_optional_event_id(action.expected_head_event_id.as_deref())?;
    Ok(())
}

fn validate_prospect_action(action: &ProspectAction) -> Result<(), IngestError> {
    let prospect = &action.prospect;
    if action.prospect_id.is_nil()
        || prospect.prospect_id != action.prospect_id
        || prospect.party.party_id != action.prospect_id
        || !matches!(
            prospect.party.party_type.as_str(),
            "person" | "organization"
        )
        || prospect.party.display_name.trim().is_empty()
        || prospect.party.display_name.len() > 240
        || prospect.industry.trim().is_empty()
        || prospect.industry.len() > 200
        || prospect.vertical.trim().is_empty()
        || prospect.vertical.len() > 200
        || prospect.fit_score.is_some_and(|score| score > 100)
        || prospect.discovery_tags.len() > 500
        || prospect
            .potential_monthly_value_minor
            .is_some_and(|amount| amount < 0)
        || prospect.evidence.len() > 100
        || prospect.party.external_ids.len() > 32
        || (prospect.stage == ProspectStage::Lost
            && prospect
                .lost_reason
                .as_ref()
                .is_none_or(|reason| reason.trim().is_empty() || reason.len() > 1_000))
        || (prospect.stage != ProspectStage::Lost && prospect.lost_reason.is_some())
    {
        return Err(invalid("prospect identity, taxonomy, or stage is invalid"));
    }
    validate_optional_event_id(action.expected_head_event_id.as_deref())?;
    if prospect.discovery_tags.iter().any(|tag| {
        tag.tag.trim().is_empty() || tag.tag.len() > 240 || !valid_discovery_tag_kind(&tag.kind)
    }) {
        return Err(invalid("prospect discovery tags are invalid"));
    }
    for external_id in &prospect.party.external_ids {
        if external_id.trim().is_empty() || external_id.len() > 128 {
            return Err(invalid("prospect external identifiers are invalid"));
        }
    }
    validate_optional_text(prospect.website.as_deref(), 2_048, "website")?;
    if let Some(website) = prospect.website.as_deref() {
        validate_https_url(website, "website")?;
    }
    validate_optional_text(prospect.contact_name.as_deref(), 160, "contact name")?;
    validate_optional_text(prospect.location.as_deref(), 200, "location")?;
    validate_optional_text(prospect.email.as_deref(), 254, "email")?;
    if prospect
        .email
        .as_deref()
        .is_some_and(|email| !email.contains('@'))
    {
        return Err(invalid("prospect email is invalid"));
    }
    validate_optional_text(prospect.phone.as_deref(), 50, "phone")?;
    if prospect
        .last_verified_at
        .is_some_and(|timestamp| timestamp <= 0)
    {
        return Err(invalid(
            "prospect lastVerifiedAt must be a positive timestamp",
        ));
    }
    for evidence in &prospect.evidence {
        if evidence.title.trim().is_empty()
            || evidence.title.len() > 200
            || evidence.excerpt.len() > 2_000
            || evidence.observed_at <= 0
        {
            return Err(invalid("prospect evidence is invalid"));
        }
        validate_https_url(&evidence.url, "evidence URL")?;
    }
    if let Some(activity) = action.activity.as_ref() {
        if activity.activity_id.is_nil()
            || activity.content.trim().is_empty()
            || activity.content.len() > 2_000
            || activity.proposal_id.is_some() != activity.proposal_version_event_id.is_some()
        {
            return Err(invalid("prospect activity is invalid"));
        }
        validate_optional_event_id(activity.proposal_version_event_id.as_deref())?;
    }
    Ok(())
}

fn valid_discovery_tag_kind(kind: &str) -> bool {
    matches!(
        kind,
        "product"
            | "service"
            | "specialty"
            | "customer"
            | "technology"
            | "equipment"
            | "material"
            | "channel"
            | "role"
            | "regulation"
            | "term"
    )
}

fn validate_optional_text(
    value: Option<&str>,
    max_bytes: usize,
    field: &str,
) -> Result<(), IngestError> {
    if value.is_some_and(|text| text.trim().is_empty() || text.len() > max_bytes) {
        return Err(invalid(format!("prospect {field} is invalid")));
    }
    Ok(())
}

fn validate_https_url(value: &str, field: &str) -> Result<(), IngestError> {
    let parsed =
        url::Url::parse(value).map_err(|_| invalid(format!("prospect {field} is invalid")))?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(invalid(format!("prospect {field} must use HTTPS")));
    }
    Ok(())
}

async fn handle_company_work_item_action(
    tenant: &TenantContext,
    state: &Arc<AppState>,
    event: Event,
    auth: IngestAuth,
    channel_id: Uuid,
    d_tag: String,
    action: CompanyWorkItemAction,
) -> Result<IngestResult, IngestError> {
    validate_company_work_item_action(&action)
        .map_err(|error| invalid(format!("company work action: {error}")))?;
    validate_company_work_d_tag(&d_tag, action.work_item_id)
        .map_err(|error| invalid(format!("company work d tag: {error}")))?;

    let community_id = tenant.community();
    let actor_pubkey = auth.pubkey().to_hex();
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

    let actor_pubkey_bytes = auth.pubkey().to_bytes();
    let move_destination_channel_id = if action.action == CompanyWorkItemActionKind::Update {
        if let Some(root_id) = action
            .head
            .as_ref()
            .and_then(|head| head.thread_root_event_id.as_deref())
        {
            let destination_channel_id =
                load_company_work_message_any_channel(state, community_id, root_id)
                    .await?
                    .channel_id
                    .ok_or_else(|| invalid("destination thread is not in a stream channel"))?;
            lock_company_work_move_channels(
                &mut tx,
                state,
                community_id,
                channel_id,
                destination_channel_id,
                root_id,
                actor_pubkey_bytes.as_slice(),
            )
            .await?;
            Some(destination_channel_id)
        } else {
            require_company_work_channel_membership(
                &mut tx,
                community_id,
                channel_id,
                actor_pubkey_bytes.as_slice(),
            )
            .await?;
            None
        }
    } else {
        require_company_work_channel_membership(
            &mut tx,
            community_id,
            channel_id,
            actor_pubkey_bytes.as_slice(),
        )
        .await?;
        None
    };
    let community_role = sqlx::query_scalar::<_, String>(
        "SELECT role::text FROM relay_members WHERE community_id = $1 AND pubkey = $2 FOR UPDATE",
    )
    .bind(community_id.as_uuid())
    .bind(&actor_pubkey)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?;
    // Company work is scoped to the active channel. A relay-members row adds
    // community-wide owner or admin authority, but its absence does not revoke
    // the channel member's ability to manage work in that channel.
    let community_role = match community_role {
        Some(role) => role,
        None => "member".to_owned(),
    };

    let already_stored = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM events WHERE community_id = $1 AND id = $2)",
    )
    .bind(community_id.as_uuid())
    .bind(event.id.to_bytes().as_slice())
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    if already_stored {
        tx.rollback().await.map_err(internal)?;
        return Ok(IngestResult {
            event_id: event.id.to_hex(),
            accepted: true,
            message: "duplicate: already processed".into(),
        });
    }

    let relay_pubkey = state.relay_keypair.public_key().to_bytes();
    let current_head_id = buzz_db::replaceable::lock_parameterized_event_head_in_transaction(
        &mut tx,
        community_id,
        KIND_WORK_ITEM_HEAD,
        &relay_pubkey,
        &d_tag,
    )
    .await
    .map_err(internal)?;
    let expected = action.expected_head_event_id.as_deref();
    match (action.action, expected, current_head_id.as_deref()) {
        (CompanyWorkItemActionKind::Create, None, None) => {}
        (CompanyWorkItemActionKind::Create, _, Some(_)) => {
            tx.rollback().await.map_err(internal)?;
            return Err(conflict("company work item already exists"));
        }
        (_, Some(expected), Some(actual)) if expected == hex::encode(actual) => {}
        (_, Some(_), Some(_)) => {
            tx.rollback().await.map_err(internal)?;
            return Err(conflict(
                "company work item changed; retry from its latest head",
            ));
        }
        (_, _, None) => {
            tx.rollback().await.map_err(internal)?;
            return Err(conflict("company work item does not exist"));
        }
        _ => {
            tx.rollback().await.map_err(internal)?;
            return Err(invalid(
                "expectedHeadEventId does not match the company work action",
            ));
        }
    }

    let current_stored =
        current_head::<CompanyWorkItemHead>(state, community_id, KIND_WORK_ITEM_HEAD, &d_tag)
            .await?;
    if current_stored
        .as_ref()
        .map(|stored| stored.event.id.to_bytes().to_vec())
        != current_head_id
    {
        tx.rollback().await.map_err(internal)?;
        return Err(IngestError::Internal(
            "error: company work head changed while its transaction lock was held".into(),
        ));
    }
    if current_stored
        .as_ref()
        .is_some_and(|stored| stored.channel_id != Some(channel_id))
    {
        tx.rollback().await.map_err(internal)?;
        return Err(conflict("company work item belongs to another channel"));
    }
    let previous = current_stored
        .as_ref()
        .map(|stored| parse_content::<CompanyWorkItemHead>(&stored.event))
        .transpose()?;
    if previous
        .as_ref()
        .is_some_and(|head| head.work_item_id != action.work_item_id)
    {
        tx.rollback().await.map_err(internal)?;
        return Err(IngestError::Internal(
            "error: company work head does not match its d tag".into(),
        ));
    }

    let next_goal_id = action.head.as_ref().and_then(|head| head.goal_id);
    let goal_changes = previous.as_ref().and_then(|head| head.goal_id) != next_goal_id;
    if action.action == CompanyWorkItemActionKind::Create
        || (action.action == CompanyWorkItemActionKind::Update && goal_changes)
    {
        if let Some(goal_id) = next_goal_id {
            sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
                .bind(format!("company-goal-tree:{}", community_id.as_uuid()))
                .execute(&mut *tx)
                .await
                .map_err(internal)?;
            ensure_company_work_goal_is_live(state, community_id, goal_id).await?;
        }
    }

    let actor_is_community_admin = is_community_admin(&community_role);
    let mut next_channel_id = channel_id;
    let next_head = match action.action {
        CompanyWorkItemActionKind::Create => {
            let input = action
                .head
                .ok_or_else(|| invalid("company work create head is required"))?;
            ensure_company_work_people_are_members(
                &mut tx,
                community_id,
                channel_id,
                &input.requester_pubkey,
                &input.assigned_pubkeys,
            )
            .await?;
            validate_company_work_thread(
                state,
                community_id,
                channel_id,
                input.source_event_id.as_deref(),
                input.thread_root_event_id.as_deref(),
                true,
            )
            .await?;
            let accepted_at = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
            if let Some(due_at) = input.due_at.as_deref() {
                validate_work_due_after_acceptance(due_at, Some(&accepted_at))?;
            }
            CompanyWorkItemHead {
                schema_version: input.schema_version,
                work_item_id: input.work_item_id,
                title: input.title,
                status: CompanyWorkStatus::Active,
                assigned_pubkeys: input.assigned_pubkeys,
                approver_pubkeys: input.approver_pubkeys,
                deliverables: input.deliverables,
                requester_pubkey: input.requester_pubkey,
                done_condition: input.done_condition,
                goal_id: input.goal_id,
                source_event_id: input.source_event_id,
                thread_root_event_id: input.thread_root_event_id,
                evidence: input.evidence,
                status_reason: None,
                verification: None,
                source_action_event_id: event.id.to_hex(),
                accepted_at: Some(accepted_at),
                due_at: input.due_at,
            }
        }
        CompanyWorkItemActionKind::Update => {
            let previous = previous
                .as_ref()
                .ok_or_else(|| conflict("company work item does not exist"))?;
            if previous.status == CompanyWorkStatus::Archived {
                tx.rollback().await.map_err(internal)?;
                return Err(conflict("archived company work items are read-only"));
            }
            let is_owner = previous
                .assigned_pubkeys
                .iter()
                .any(|pubkey| pubkey == &actor_pubkey);
            let is_requester = previous.requester_pubkey == actor_pubkey;
            if !is_owner && !is_requester && !actor_is_community_admin {
                tx.rollback().await.map_err(internal)?;
                return Err(forbidden(
                    "only the work owner, requester, or a community owner or admin can edit this item",
                ));
            }
            let input = action
                .head
                .ok_or_else(|| invalid("company work update head is required"))?;
            if input.status != previous.status {
                tx.rollback().await.map_err(internal)?;
                return Err(invalid("status changes require a status action"));
            }
            if input.deliverables != previous.deliverables {
                tx.rollback().await.map_err(internal)?;
                return Err(invalid(
                    "company work actions cannot change deliverable version pointers",
                ));
            }
            if input.due_at.is_some() && input.due_at != previous.due_at {
                tx.rollback().await.map_err(internal)?;
                return Err(invalid(
                    "due dates must use the explicit set_due_date or clear_due_date action",
                ));
            }
            if input.source_event_id != previous.source_event_id {
                tx.rollback().await.map_err(internal)?;
                return Err(invalid("sourceEventId cannot change after creation"));
            }
            if input.thread_root_event_id != previous.thread_root_event_id {
                let Some(destination_channel_id) = move_destination_channel_id else {
                    tx.rollback().await.map_err(internal)?;
                    return Err(invalid("moving work requires a destination thread"));
                };
                validate_company_work_thread(
                    state,
                    community_id,
                    destination_channel_id,
                    None,
                    input.thread_root_event_id.as_deref(),
                    false,
                )
                .await?;
                ensure_company_work_people_are_members(
                    &mut tx,
                    community_id,
                    destination_channel_id,
                    &input.requester_pubkey,
                    &input.assigned_pubkeys,
                )
                .await?;
                next_channel_id = destination_channel_id;
            } else {
                validate_company_work_thread(
                    state,
                    community_id,
                    channel_id,
                    None,
                    input.thread_root_event_id.as_deref(),
                    false,
                )
                .await?;
            }
            ensure_company_work_people_are_members(
                &mut tx,
                community_id,
                channel_id,
                &input.requester_pubkey,
                &input.assigned_pubkeys,
            )
            .await?;
            CompanyWorkItemHead {
                schema_version: input.schema_version,
                work_item_id: input.work_item_id,
                title: input.title,
                status: previous.status,
                assigned_pubkeys: input.assigned_pubkeys,
                approver_pubkeys: input.approver_pubkeys,
                deliverables: previous.deliverables.clone(),
                requester_pubkey: input.requester_pubkey,
                done_condition: input.done_condition,
                goal_id: input.goal_id,
                source_event_id: previous.source_event_id.clone(),
                thread_root_event_id: input.thread_root_event_id,
                evidence: input.evidence,
                status_reason: previous.status_reason.clone(),
                verification: previous.verification.clone(),
                source_action_event_id: event.id.to_hex(),
                accepted_at: previous.accepted_at.clone(),
                due_at: previous.due_at.clone(),
            }
        }
        CompanyWorkItemActionKind::SetStatus => {
            let previous = previous
                .as_ref()
                .ok_or_else(|| conflict("company work item does not exist"))?;
            if matches!(
                previous.status,
                CompanyWorkStatus::Archived | CompanyWorkStatus::DoneVerified
            ) {
                tx.rollback().await.map_err(internal)?;
                return Err(conflict("this company work item status cannot be changed"));
            }
            let is_owner = previous
                .assigned_pubkeys
                .iter()
                .any(|pubkey| pubkey == &actor_pubkey);
            if !is_owner && !actor_is_community_admin {
                tx.rollback().await.map_err(internal)?;
                return Err(forbidden(
                    "only the work owner or a community owner or admin can change status",
                ));
            }
            let status = action
                .status
                .ok_or_else(|| invalid("company work status is required"))?;
            if status == CompanyWorkStatus::DoneUnverified && !is_owner {
                tx.rollback().await.map_err(internal)?;
                return Err(forbidden(
                    "only the assigned owner can submit work for verification",
                ));
            }
            let mut next = previous.clone();
            next.status = status;
            next.status_reason = action.reason;
            next.source_action_event_id = event.id.to_hex();
            next
        }
        CompanyWorkItemActionKind::Verify => {
            let previous = previous
                .as_ref()
                .ok_or_else(|| conflict("company work item does not exist"))?;
            if previous.status != CompanyWorkStatus::DoneUnverified {
                tx.rollback().await.map_err(internal)?;
                return Err(conflict("company work item is not awaiting verification"));
            }
            if !actor_is_community_admin && previous.requester_pubkey != actor_pubkey {
                tx.rollback().await.map_err(internal)?;
                return Err(forbidden(
                    "only a community owner, admin, or the work requester can verify this item",
                ));
            }
            let input = action
                .verification
                .ok_or_else(|| invalid("company work verification is required"))?;
            let mut next = previous.clone();
            next.status = match input.verdict {
                buzz_core::business_records::CompanyWorkVerdict::Pass => {
                    CompanyWorkStatus::DoneVerified
                }
                buzz_core::business_records::CompanyWorkVerdict::RevisionRequested => {
                    CompanyWorkStatus::Active
                }
            };
            next.status_reason = Some(input.reason.clone());
            next.verification = Some(CompanyWorkVerification {
                verdict: input.verdict,
                reason: input.reason,
                evidence: input.evidence,
                reviewer_pubkey: actor_pubkey.clone(),
                reviewed_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
                source_action_event_id: event.id.to_hex(),
            });
            next.source_action_event_id = event.id.to_hex();
            next
        }
        CompanyWorkItemActionKind::Archive => {
            let previous = previous
                .as_ref()
                .ok_or_else(|| conflict("company work item does not exist"))?;
            let is_owner = previous
                .assigned_pubkeys
                .iter()
                .any(|pubkey| pubkey == &actor_pubkey);
            let is_requester = previous.requester_pubkey == actor_pubkey;
            if !is_owner && !is_requester && !actor_is_community_admin {
                tx.rollback().await.map_err(internal)?;
                return Err(forbidden(
                    "only the work owner, requester, or a community owner or admin can archive this item",
                ));
            }
            let mut next = previous.clone();
            next.status = CompanyWorkStatus::Archived;
            next.source_action_event_id = event.id.to_hex();
            next
        }
        CompanyWorkItemActionKind::Restore => {
            let previous = previous
                .as_ref()
                .ok_or_else(|| conflict("company work item does not exist"))?;
            let is_owner = previous
                .assigned_pubkeys
                .iter()
                .any(|pubkey| pubkey == &actor_pubkey);
            let is_requester = previous.requester_pubkey == actor_pubkey;
            if previous.status != CompanyWorkStatus::Archived
                || (!is_owner && !is_requester && !actor_is_community_admin)
            {
                tx.rollback().await.map_err(internal)?;
                return Err(forbidden(
                    "only the work owner, requester, or a community owner or admin can restore archived work",
                ));
            }
            let mut next = previous.clone();
            next.status = CompanyWorkStatus::Active;
            next.source_action_event_id = event.id.to_hex();
            next
        }
        CompanyWorkItemActionKind::SetDueDate => {
            let previous = previous
                .as_ref()
                .ok_or_else(|| conflict("company work item does not exist"))?;
            if previous.status == CompanyWorkStatus::Archived {
                tx.rollback().await.map_err(internal)?;
                return Err(conflict("archived company work items are read-only"));
            }
            let is_owner = previous
                .assigned_pubkeys
                .iter()
                .any(|pubkey| pubkey == &actor_pubkey);
            let is_requester = previous.requester_pubkey == actor_pubkey;
            if !is_owner && !is_requester && !actor_is_community_admin {
                tx.rollback().await.map_err(internal)?;
                return Err(forbidden(
                    "only the work owner, requester, or a community owner or admin can edit this item",
                ));
            }
            let due_at = action
                .due_at
                .as_deref()
                .ok_or_else(|| invalid("dueAt is required"))?;
            validate_work_due_after_acceptance(due_at, previous.accepted_at.as_deref())?;
            if previous.due_at.as_deref() == Some(due_at) {
                tx.rollback().await.map_err(internal)?;
                return Err(conflict("company work item already has this due date"));
            }
            let mut next = previous.clone();
            next.due_at = Some(due_at.to_owned());
            next.source_action_event_id = event.id.to_hex();
            next
        }
        CompanyWorkItemActionKind::ClearDueDate => {
            let previous = previous
                .as_ref()
                .ok_or_else(|| conflict("company work item does not exist"))?;
            if previous.status == CompanyWorkStatus::Archived {
                tx.rollback().await.map_err(internal)?;
                return Err(conflict("archived company work items are read-only"));
            }
            let is_owner = previous
                .assigned_pubkeys
                .iter()
                .any(|pubkey| pubkey == &actor_pubkey);
            let is_requester = previous.requester_pubkey == actor_pubkey;
            if !is_owner && !is_requester && !actor_is_community_admin {
                tx.rollback().await.map_err(internal)?;
                return Err(forbidden(
                    "only the work owner, requester, or a community owner or admin can edit this item",
                ));
            }
            if previous.due_at.is_none() {
                tx.rollback().await.map_err(internal)?;
                return Err(conflict("company work item has no due date to clear"));
            }
            let mut next = previous.clone();
            next.due_at = None;
            next.source_action_event_id = event.id.to_hex();
            next
        }
    };

    let next_event = relay_head_event(
        KIND_WORK_ITEM_HEAD,
        next_channel_id,
        &d_tag,
        &next_head,
        current_stored.as_ref(),
        state,
    )?;
    let (stored_action, inserted) = buzz_db::event::insert_event_in_transaction(
        &mut tx,
        community_id,
        &event,
        Some(channel_id),
    )
    .await
    .map_err(internal)?;
    if !inserted {
        tx.rollback().await.map_err(internal)?;
        return Ok(IngestResult {
            event_id: event.id.to_hex(),
            accepted: true,
            message: "duplicate: already processed".into(),
        });
    }

    let precondition = current_head_id.as_deref().map_or(
        ParameterizedReplacePrecondition::CreateOnly,
        ParameterizedReplacePrecondition::ExpectedRevision,
    );
    let replaced = state
        .db
        .replace_parameterized_event_in_transaction(
            &mut tx,
            community_id,
            &next_event,
            &d_tag,
            Some(next_channel_id),
            precondition,
        )
        .await
        .map_err(internal)?;
    match replaced.status {
        ParameterizedReplaceStatus::Inserted => {}
        ParameterizedReplaceStatus::Duplicate => {
            tx.rollback().await.map_err(internal)?;
            return Ok(IngestResult {
                event_id: event.id.to_hex(),
                accepted: true,
                message: "duplicate: already processed".into(),
            });
        }
        ParameterizedReplaceStatus::RevisionMismatch
        | ParameterizedReplaceStatus::RevisionMissing
        | ParameterizedReplaceStatus::Superseded
        | ParameterizedReplaceStatus::ReplayOnlyMiss => {
            tx.rollback().await.map_err(internal)?;
            return Err(conflict(
                "company work item changed before the action committed",
            ));
        }
    }
    let watchdog_head = if previous.as_ref().is_some_and(|previous| {
        previous.status != next_head.status
            || previous.thread_root_event_id != next_head.thread_root_event_id
            || next_channel_id != channel_id
    }) {
        super::company_work_tracking::sync_watchdog_for_work_change(
            &mut tx,
            community_id,
            channel_id,
            next_channel_id,
            &next_head,
            state,
        )
        .await?
    } else {
        None
    };
    tx.commit().await.map_err(internal)?;

    super::event::dispatch_persistent_event(
        tenant,
        state,
        &stored_action,
        KIND_WORK_ITEM_ACTION,
        &actor_pubkey,
        None,
    )
    .await;
    if let Some(watchdog_head) = watchdog_head.as_ref() {
        super::event::dispatch_persistent_event(
            tenant,
            state,
            watchdog_head,
            KIND_COMPANY_WORK_TRACKING_HEAD,
            &watchdog_head.event.pubkey.to_hex(),
            None,
        )
        .await;
    }
    super::event::dispatch_persistent_event(
        tenant,
        state,
        &replaced.event,
        KIND_WORK_ITEM_HEAD,
        &next_event.pubkey.to_hex(),
        None,
    )
    .await;
    Ok(IngestResult {
        event_id: event.id.to_hex(),
        accepted: true,
        message: String::new(),
    })
}

pub(super) async fn ensure_company_work_people_are_members(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    community_id: CommunityId,
    channel_id: Uuid,
    requester_pubkey: &str,
    assigned_pubkeys: &[String],
) -> Result<(), IngestError> {
    let mut people = assigned_pubkeys.to_vec();
    people.push(requester_pubkey.to_owned());
    people.sort();
    people.dedup();
    for pubkey in people {
        let pubkey_bytes = hex::decode(&pubkey).map_err(|_| invalid("work pubkey is invalid"))?;
        let is_member = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM channel_members cm \
             JOIN channels c ON cm.community_id = c.community_id AND cm.channel_id = c.id \
             WHERE cm.community_id = $1 AND cm.channel_id = $2 AND cm.pubkey = $3 \
               AND cm.removed_at IS NULL AND c.deleted_at IS NULL \
               AND c.archived_at IS NULL)",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .bind(&pubkey_bytes)
        .fetch_one(&mut **tx)
        .await
        .map_err(internal)?;
        if !is_member {
            return Err(forbidden(
                "work owner and requester must be members of the tagged channel",
            ));
        }
    }
    Ok(())
}

fn validate_work_due_after_acceptance(
    due_at: &str,
    accepted_at: Option<&str>,
) -> Result<(), IngestError> {
    validate_utc_rfc3339(due_at)
        .map_err(|error| invalid(format!("company work due date: {error}")))?;
    let due_at = chrono::DateTime::parse_from_rfc3339(due_at)
        .map_err(|_| invalid("company work due date is invalid"))?;
    if let Some(accepted_at) = accepted_at {
        validate_utc_rfc3339(accepted_at)
            .map_err(|_| IngestError::Internal("company work acceptedAt is invalid".into()))?;
        let accepted_at = chrono::DateTime::parse_from_rfc3339(accepted_at)
            .map_err(|_| IngestError::Internal("company work acceptedAt is invalid".into()))?;
        if due_at <= accepted_at {
            return Err(invalid(
                "company work due date must be later than its acceptance time",
            ));
        }
    }
    Ok(())
}

pub(super) async fn require_company_work_channel_membership(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    community_id: CommunityId,
    channel_id: Uuid,
    actor_pubkey: &[u8],
) -> Result<(), IngestError> {
    sqlx::query_scalar::<_, String>(
        "SELECT cm.role::text FROM channel_members cm \
         JOIN channels c ON cm.community_id = c.community_id AND cm.channel_id = c.id \
         WHERE cm.community_id = $1 AND cm.channel_id = $2 AND cm.pubkey = $3 \
           AND cm.removed_at IS NULL AND c.deleted_at IS NULL AND c.archived_at IS NULL \
         FOR UPDATE OF cm, c",
    )
    .bind(community_id.as_uuid())
    .bind(channel_id)
    .bind(actor_pubkey)
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal)?
    .ok_or_else(|| forbidden("actor is not a member of the work channel"))?;
    Ok(())
}

async fn lock_company_work_move_channels(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    state: &AppState,
    community_id: CommunityId,
    source_channel_id: Uuid,
    destination_channel_id: Uuid,
    root_event_id: &str,
    actor_pubkey: &[u8],
) -> Result<(), IngestError> {
    let root = load_company_work_message_any_channel(state, community_id, root_event_id).await?;
    if root.channel_id != Some(destination_channel_id)
        || buzz_core::nip10::parse_thread_markers(&root.event.tags)
            .resolve()
            .is_some()
    {
        return Err(invalid("destination must be an available thread root"));
    }

    let mut channel_ids = vec![source_channel_id, destination_channel_id];
    channel_ids.sort_unstable();
    channel_ids.dedup();
    let mut source_visibility = None;
    let mut destination_visibility = None;
    for channel_id in channel_ids.iter().copied() {
        let active_channel = sqlx::query_as::<_, (Uuid, String)>(
            "SELECT id, visibility::text FROM channels WHERE community_id = $1 AND id = $2 \
             AND channel_type::text = 'stream' AND deleted_at IS NULL \
             AND archived_at IS NULL FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(internal)?;
        let Some((active_channel_id, visibility)) = active_channel else {
            return Err(forbidden("source or destination channel is unavailable"));
        };
        if active_channel_id != channel_id {
            return Err(forbidden("source or destination channel is unavailable"));
        }
        if channel_id == source_channel_id {
            source_visibility = Some(visibility);
        } else {
            destination_visibility = Some(visibility);
        }
    }

    let locked_root_channel_id = sqlx::query_as::<_, (Option<Uuid>,)>(
        "SELECT channel_id FROM events WHERE community_id = $1 AND id = $2 \
         AND kind IN ($3, $4) AND deleted_at IS NULL FOR SHARE",
    )
    .bind(community_id.as_uuid())
    .bind(hex::decode(root_event_id).map_err(|_| invalid("thread root event id is invalid"))?)
    .bind(KIND_STREAM_MESSAGE as i32)
    .bind(KIND_STREAM_MESSAGE_V2 as i32)
    .fetch_optional(&mut **tx)
    .await
    .map_err(internal)?
    .and_then(|row| row.0);
    if locked_root_channel_id != Some(destination_channel_id) {
        return Err(invalid("destination thread is no longer available"));
    }

    let mut source_members = None;
    let mut destination_members = None;
    for channel_id in channel_ids {
        let members = sqlx::query_as::<_, (Vec<u8>, String)>(
            "SELECT pubkey, role::text FROM channel_members WHERE community_id = $1 \
             AND channel_id = $2 AND removed_at IS NULL ORDER BY pubkey FOR UPDATE",
        )
        .bind(community_id.as_uuid())
        .bind(channel_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(internal)?;
        if channel_id == source_channel_id {
            source_members = Some(members);
        } else {
            destination_members = Some(members);
        }
    }
    let source_members = source_members.unwrap_or_default();
    let destination_members = destination_members.unwrap_or_else(|| source_members.clone());
    if source_channel_id == destination_channel_id {
        destination_visibility = source_visibility.clone();
    }
    if source_visibility != destination_visibility || source_members != destination_members {
        return Err(forbidden(
            "work cannot move because the destination has a different audience",
        ));
    }
    if !source_members
        .iter()
        .any(|(member, _)| member.as_slice() == actor_pubkey)
    {
        return Err(forbidden("actor is not a member of both work channels"));
    }
    Ok(())
}

async fn ensure_company_work_goal_is_live(
    state: &AppState,
    community_id: CommunityId,
    goal_id: Uuid,
) -> Result<(), IngestError> {
    let head = current_head::<GoalHead>(state, community_id, KIND_GOAL_HEAD, &goal_d_tag(goal_id))
        .await?
        .ok_or_else(|| conflict("linked company goal does not exist"))?;
    let head: GoalHead = parse_content(&head.event)?;
    if head.goal_id != goal_id
        || head.goal.is_none()
        || matches!(head.status, GoalStatus::Archived | GoalStatus::Deleted)
    {
        return Err(conflict("company work must reference a live company goal"));
    }
    Ok(())
}

async fn validate_company_work_thread(
    state: &AppState,
    community_id: CommunityId,
    channel_id: Uuid,
    source_event_id: Option<&str>,
    thread_root_event_id: Option<&str>,
    require_source_relation: bool,
) -> Result<(), IngestError> {
    let root = match thread_root_event_id {
        Some(root) => Some(load_company_work_message(state, community_id, channel_id, root).await?),
        None => None,
    };
    let Some(source_event_id) = source_event_id else {
        if require_source_relation && root.is_some() {
            return Err(invalid(
                "threadRootEventId requires a sourceEventId on create",
            ));
        }
        return Ok(());
    };
    let source =
        load_company_work_message(state, community_id, channel_id, source_event_id).await?;
    if require_source_relation {
        let root_id = thread_root_event_id
            .ok_or_else(|| invalid("sourceEventId requires a threadRootEventId on create"))?;
        if source_event_id != root_id
            && !source.event.tags.iter().any(|tag| {
                let values = tag.as_slice();
                tag.kind().to_string() == "e" && values.get(1).is_some_and(|value| value == root_id)
            })
        {
            return Err(invalid(
                "threadRootEventId must be the source message or its thread root",
            ));
        }
    }
    if root.is_none() {
        return Err(invalid("source message must have a thread root"));
    }
    Ok(())
}

async fn load_company_work_message(
    state: &AppState,
    community_id: CommunityId,
    channel_id: Uuid,
    event_id: &str,
) -> Result<StoredEvent, IngestError> {
    let message = load_company_work_message_any_channel(state, community_id, event_id).await?;
    if message.channel_id != Some(channel_id) {
        return Err(invalid(
            "source or thread message is unavailable in the tagged channel",
        ));
    }
    Ok(message)
}

async fn load_company_work_message_any_channel(
    state: &AppState,
    community_id: CommunityId,
    event_id: &str,
) -> Result<StoredEvent, IngestError> {
    let event_id_bytes =
        hex::decode(event_id).map_err(|_| invalid("message event id is invalid"))?;
    let mut query = EventQuery::for_community(community_id);
    query.kinds = Some(vec![
        KIND_STREAM_MESSAGE as i32,
        KIND_STREAM_MESSAGE_V2 as i32,
    ]);
    query.ids = Some(vec![event_id_bytes]);
    query.limit = Some(2);
    let mut rows = state
        .db
        .query_events_for_event_write(&query)
        .await
        .map_err(internal)?;
    if rows.len() > 1 {
        return Err(IngestError::Internal(
            "error: duplicate company work source message".into(),
        ));
    }
    rows.pop()
        .ok_or_else(|| invalid("source or destination thread is unavailable"))
}

fn validate_client_action(channel_id: Uuid, action: &ClientAction) -> Result<(), IngestError> {
    if action.head.schema_version != BUSINESS_RECORD_SCHEMA_VERSION
        || action.client_id.is_nil()
        || action.head.party_id.is_nil()
        || action.client_id != channel_id
        || action.head.client_id != channel_id
        || action.head.status.trim().is_empty()
        || action.head.display_name.trim().is_empty()
    {
        return Err(invalid(
            "client action content does not match its client channel",
        ));
    }
    for pubkey in &action.head.approver_pubkeys {
        parse_pubkey_hex(pubkey)?;
    }
    Ok(())
}

fn validate_work_item_action(role: &str, action: &WorkItemAction) -> Result<(), IngestError> {
    if action.head.schema_version != BUSINESS_RECORD_SCHEMA_VERSION
        || action.client_id.is_nil()
        || action.work_item_id.is_nil()
        || action.head.client_id != action.client_id
        || action.head.work_item_id != action.work_item_id
        || action.head.title.trim().is_empty()
        || action.head.status.trim().is_empty()
    {
        return Err(invalid(
            "work-item action content does not match its coordinate",
        ));
    }
    if role == "guest" || role == "bot" {
        return Err(forbidden("guest or bot roles cannot mutate work items"));
    }
    for pubkey in action
        .head
        .assigned_pubkeys
        .iter()
        .chain(action.head.approver_pubkeys.iter())
    {
        parse_pubkey_hex(pubkey)?;
    }
    for pointer in &action.head.deliverables {
        validate_hex_reference(&pointer.version_event_id)
            .map_err(|error| invalid(error.to_string()))?;
        validate_hex_reference(&pointer.content_digest)
            .map_err(|error| invalid(error.to_string()))?;
        validate_hex_reference(&pointer.media_digest)
            .map_err(|error| invalid(error.to_string()))?;
        validate_hex_reference(&pointer.version_digest)
            .map_err(|error| invalid(error.to_string()))?;
    }
    Ok(())
}

fn validate_proposal_version(version: &ProposalVersion) -> Result<(), IngestError> {
    if version.proposal_id.is_nil()
        || version.prospect_party_id.is_nil()
        || version.revision == 0
        || version.lines.is_empty()
        || version.currency.len() != 3
        || !version
            .currency
            .bytes()
            .all(|byte| byte.is_ascii_uppercase())
        || version.terms.trim().is_empty()
    {
        return Err(invalid(
            "proposal version requires revision, currency, lines, and terms",
        ));
    }
    parse_pubkey_hex(&version.named_acceptor_pubkey)?;
    validate_optional_event_id(version.previous_version_event_id.as_deref())?;
    for line in &version.lines {
        if line.description.trim().is_empty()
            || line.quantity_hundredths == 0
            || line.unit_amount_minor < 0
        {
            return Err(invalid(
                "proposal lines require positive quantity and non-negative amount",
            ));
        }
    }
    let _ = version
        .lines
        .iter()
        .try_fold(0_i64, |total, line| {
            let line_total = i64::from(line.quantity_hundredths)
                .checked_mul(line.unit_amount_minor)?
                .checked_div(100)?;
            total.checked_add(line_total)
        })
        .ok_or_else(|| invalid("proposal total overflows minor units"))?;
    Ok(())
}

fn validate_deliverable_version(
    version: &DeliverableVersion,
    event: &Event,
    work_item: &WorkItemHead,
) -> Result<DeliverablePointer, IngestError> {
    if version.schema_version != BUSINESS_RECORD_SCHEMA_VERSION
        || version.client_id.is_nil()
        || version.work_item_id.is_nil()
        || version.deliverable_id.is_nil()
        || version.client_id != work_item.client_id
        || version.work_item_id != work_item.work_item_id
        || version.version == 0
    {
        return Err(invalid("deliverable version does not match its work item"));
    }
    validate_optional_event_id(version.previous_version_event_id.as_deref())?;
    validate_hex_reference(&version.content_digest).map_err(|error| invalid(error.to_string()))?;
    if version
        .media_digests
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(invalid("media digests must be unique and sorted"));
    }
    for digest in &version.media_digests {
        validate_hex_reference(digest).map_err(|error| invalid(error.to_string()))?;
    }
    let body_digest = digest_hex(&serde_json::to_vec(&version.body).map_err(internal)?);
    if body_digest != version.content_digest {
        return Err(invalid("contentDigest does not match the canonical body"));
    }
    let media_digest = digest_hex(&serde_json::to_vec(&version.media_digests).map_err(internal)?);
    let mut version_digest_bytes = decode_hash(&version.content_digest)?;
    version_digest_bytes.extend_from_slice(&decode_hash(&media_digest)?);
    let version_digest = digest_hex(&version_digest_bytes);
    Ok(DeliverablePointer {
        deliverable_id: version.deliverable_id,
        version_event_id: event.id.to_hex(),
        content_digest: version.content_digest.clone(),
        media_digest,
        version_digest,
    })
}

fn validate_invoice_version(version: &InvoiceVersion) -> Result<(), IngestError> {
    if version.schema_version != BUSINESS_RECORD_SCHEMA_VERSION
        || version.client_id.is_nil()
        || version.invoice_id.is_nil()
        || version.version == 0
        || version.lines.is_empty()
        || version.lines.len() > 100
        || !is_iso_currency_code(&version.currency)
        || version.due_at.is_some_and(|due_at| due_at <= 0)
    {
        return Err(invalid(
            "invoice version has invalid identifiers, currency, or terms",
        ));
    }
    validate_optional_event_id(version.previous_version_event_id.as_deref())?;
    validate_optional_event_id(version.proposal_version_event_id.as_deref())?;
    validate_optional_event_id(version.expected_head_event_id.as_deref())?;
    for line in &version.lines {
        if line.description.trim().is_empty()
            || line.description.len() > 2_000
            || line.quantity_hundredths == 0
            || line.unit_amount_minor < 0
        {
            return Err(invalid(
                "invoice lines require a description, positive quantity, and non-negative amount",
            ));
        }
    }
    if version.action == InvoiceVersionAction::Issue && version.due_at.is_none() {
        return Err(invalid("issuing an invoice requires a due date"));
    }
    if version.tax_lines.len() > 100
        || version.tax_lines.iter().any(|tax_line| {
            tax_line
                .label
                .as_deref()
                .is_some_and(|label| label.trim().is_empty() || label.len() > 200)
        })
        || [
            version.seller_tax_number.as_deref(),
            version.customer_tax_number.as_deref(),
        ]
        .into_iter()
        .flatten()
        .any(|number| number.trim().is_empty() || number.len() > 128)
    {
        return Err(invalid("invoice tax details are invalid"));
    }
    let calculated_total = invoice_total_minor(&version.lines, &version.tax_lines)
        .ok_or_else(|| invalid("invoice total overflows minor units"))?;
    if version.total_minor != calculated_total {
        return Err(invalid(
            "invoice total does not match its line items and tax",
        ));
    }
    let valid_transition_shape = match version.action {
        InvoiceVersionAction::ProposalAcceptance => {
            version.previous_version_event_id.is_none()
                && version.expected_head_event_id.is_none()
                && version.proposal_version_event_id.is_some()
                && version.status == InvoiceStatus::Draft
                && version.void_reason.is_none()
        }
        InvoiceVersionAction::Create => {
            version.version == 1
                && version.previous_version_event_id.is_none()
                && version.expected_head_event_id.is_none()
                && version.proposal_version_event_id.is_none()
                && version.status == InvoiceStatus::Draft
                && version.void_reason.is_none()
        }
        InvoiceVersionAction::DraftEdit => {
            version.previous_version_event_id.is_some()
                && version.expected_head_event_id.is_some()
                && version.status == InvoiceStatus::Draft
                && version.void_reason.is_none()
        }
        InvoiceVersionAction::Issue => {
            version.previous_version_event_id.is_some()
                && version.expected_head_event_id.is_some()
                && version.status == InvoiceStatus::Issued
                && version.void_reason.is_none()
        }
        InvoiceVersionAction::Void => {
            version.previous_version_event_id.is_some()
                && version.expected_head_event_id.is_some()
                && version.status == InvoiceStatus::Void
                && version
                    .void_reason
                    .as_deref()
                    .is_some_and(|reason| !reason.trim().is_empty() && reason.len() <= 1_000)
        }
    };
    if !valid_transition_shape {
        return Err(invalid(
            "invoice version action, status, and evidence do not match",
        ));
    }
    Ok(())
}

fn validate_payment_evidence(payment: &PaymentEvidence) -> Result<(), IngestError> {
    if payment.schema_version != BUSINESS_RECORD_SCHEMA_VERSION
        || payment.client_id.is_nil()
        || payment.invoice_id.is_nil()
        || payment.payment_id.is_nil()
        || payment.amount_minor <= 0
        || !is_iso_currency_code(&payment.currency)
        || payment.occurred_at <= 0
        || payment.provider.trim().is_empty()
        || payment.provider.trim() != payment.provider
        || payment.provider.len() > 120
        || payment.evidence_ref.trim().is_empty()
        || payment.evidence_ref.len() > 2_000
    {
        return Err(invalid(
            "payment evidence has invalid identifiers, amount, currency, or evidence",
        ));
    }
    if payment
        .provider_reference
        .as_deref()
        .is_some_and(|reference| reference.trim().is_empty() || reference.len() > 240)
        || (payment.provider != "manual" && payment.provider_reference.is_none())
    {
        return Err(invalid(
            "named payment providers require a provider reference",
        ));
    }
    validate_hex_reference(&payment.expected_invoice_head_event_id)
        .map_err(|error| invalid(error.to_string()))
}

fn validate_money_adjustment(adjustment: &MoneyAdjustment) -> Result<(), IngestError> {
    if adjustment.schema_version != BUSINESS_RECORD_SCHEMA_VERSION
        || adjustment.client_id.is_nil()
        || adjustment.invoice_id.is_nil()
        || adjustment.adjustment_id.is_nil()
        || adjustment.amount_minor <= 0
        || !is_iso_currency_code(&adjustment.currency)
        || adjustment.occurred_at <= 0
        || adjustment.reason.trim().is_empty()
        || adjustment.reason.len() > 1_000
        || adjustment.evidence_ref.trim().is_empty()
        || adjustment.evidence_ref.len() > 2_000
    {
        return Err(invalid(
            "money adjustment requires a positive amount, reason, currency, and evidence",
        ));
    }
    validate_hex_reference(&adjustment.expected_invoice_head_event_id)
        .map_err(|error| invalid(error.to_string()))
}

fn validate_money_follow_up(action: &MoneyFollowUpAction) -> Result<(), IngestError> {
    if action.schema_version != BUSINESS_RECORD_SCHEMA_VERSION
        || action.client_id.is_nil()
        || action.invoice_id.is_nil()
        || action.follow_up_id.is_nil()
        || action.draft_content.trim().is_empty()
        || action.draft_content.len() > 5_000
        || action.due_at.is_some_and(|due_at| due_at <= 0)
    {
        return Err(invalid(
            "money follow-up draft has invalid identifiers, time, or content",
        ));
    }
    validate_optional_event_id(action.expected_head_event_id.as_deref())?;
    validate_hex_reference(&action.expected_invoice_head_event_id)
        .map_err(|error| invalid(error.to_string()))?;
    if (action.action == MoneyFollowUpActionKind::Draft) != action.expected_head_event_id.is_none()
    {
        return Err(invalid(
            "follow-up draft and transition head fields do not match",
        ));
    }
    Ok(())
}

fn invoice_outstanding_minor(head: &InvoiceHead) -> Result<i64, IngestError> {
    if head.status != InvoiceStatus::Issued {
        return Ok(0);
    }
    let outstanding = i128::from(head.total_minor)
        - i128::from(head.credited_minor)
        - i128::from(head.written_off_minor)
        - i128::from(head.collected_minor);
    i64::try_from(outstanding.max(0)).map_err(|_| {
        IngestError::Internal("error: outstanding total is outside minor-unit range".into())
    })
}

fn invoice_credit_available_minor(head: &InvoiceHead) -> Result<i64, IngestError> {
    let remaining_obligation = i128::from(head.total_minor)
        - i128::from(head.credited_minor)
        - i128::from(head.written_off_minor);
    let credit = (i128::from(head.collected_minor) - remaining_obligation).max(0);
    i64::try_from(credit).map_err(|_| {
        IngestError::Internal("error: client credit is outside minor-unit range".into())
    })
}

fn authorize_work_assignee_or_admin(
    role: &str,
    auth: &IngestAuth,
    work_item: &WorkItemHead,
) -> Result<(), IngestError> {
    let assigned = work_item
        .assigned_pubkeys
        .iter()
        .any(|pubkey| pubkey == &auth.pubkey().to_hex());
    if !assigned && !is_admin(role) {
        return Err(forbidden("actor is not assigned to this work item"));
    }
    Ok(())
}

fn check_expected(
    action: RecordAction,
    expected: Option<&str>,
    current: Option<&StoredEvent>,
) -> Result<(), IngestError> {
    match (action, expected, current) {
        (RecordAction::Create, None, None) => Ok(()),
        (RecordAction::Create, _, _) => {
            Err(conflict("record already exists or create has a prior head"))
        }
        (
            RecordAction::Update | RecordAction::Archive | RecordAction::Restore,
            Some(expected),
            Some(current),
        ) if expected == current.event.id.to_hex() => Ok(()),
        (RecordAction::Update | RecordAction::Archive | RecordAction::Restore, _, _) => {
            Err(conflict("expected head does not match the current record"))
        }
    }
}

fn resolve_lifecycle_status(
    action: RecordAction,
    current_status: Option<&str>,
    requested_status: Option<&str>,
    restored_status: &str,
) -> Result<String, IngestError> {
    match (action, current_status) {
        (RecordAction::Create, None) if requested_status == Some("archived") => {
            Err(invalid("create cannot start in archived state"))
        }
        (RecordAction::Create, None) => Ok(requested_status.unwrap_or(restored_status).to_owned()),
        (RecordAction::Create, Some(_)) => Err(conflict("record already exists")),
        (RecordAction::Update, Some("archived")) => {
            Err(conflict("archived records must be restored before update"))
        }
        (RecordAction::Update, Some(_)) if requested_status == Some("archived") => {
            Err(invalid("use the archive action to archive a record"))
        }
        (RecordAction::Update, Some(status)) => Ok(requested_status.unwrap_or(status).to_owned()),
        (RecordAction::Update, None) => Err(conflict("record does not exist")),
        (RecordAction::Archive, Some("archived")) => Err(conflict("record is already archived")),
        (RecordAction::Archive, Some(_)) => Ok("archived".to_owned()),
        (RecordAction::Archive, None) => Err(conflict("record does not exist")),
        (RecordAction::Restore, Some("archived")) => Ok(restored_status.to_owned()),
        (RecordAction::Restore, Some(_)) => Err(conflict("record is not archived")),
        (RecordAction::Restore, None) => Err(conflict("record does not exist")),
    }
}

fn validate_optional_event_id(value: Option<&str>) -> Result<(), IngestError> {
    if let Some(value) = value {
        validate_hex_reference(value).map_err(|error| invalid(error.to_string()))?;
    }
    Ok(())
}

fn parse_pubkey_hex(value: &str) -> Result<nostr::PublicKey, IngestError> {
    validate_hex_reference(value).map_err(|_| invalid("pubkey must be 32 lowercase hex bytes"))?;
    nostr::PublicKey::from_hex(value).map_err(|_| invalid("pubkey must be 32 lowercase hex bytes"))
}

fn decode_hash(value: &str) -> Result<Vec<u8>, IngestError> {
    validate_hex_reference(value).map_err(|error| invalid(error.to_string()))?;
    hex::decode(value).map_err(|_| invalid("event id or digest must be lowercase hex"))
}

fn digest_hex(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

fn is_admin(role: &str) -> bool {
    role == "owner" || role == "admin"
}

pub(super) fn is_community_admin(role: &str) -> bool {
    role == "owner" || role == "admin"
}

fn require_admin(role: &str) -> Result<(), IngestError> {
    if is_admin(role) {
        Ok(())
    } else {
        Err(forbidden("owner or admin role is required"))
    }
}

fn require_member_or_admin(role: &str) -> Result<(), IngestError> {
    if matches!(role, "owner" | "admin" | "member") {
        Ok(())
    } else {
        Err(forbidden("member, admin, or owner role is required"))
    }
}

/// Members, admins and owners manage prospects. Agents hold the `bot` role in
/// the business channel, so they may update an existing prospect (notes, stage,
/// qualification, saved) but not create, archive or restore one.
fn require_prospect_actor(role: &str, action: RecordAction) -> Result<(), IngestError> {
    if role == "bot" && action == RecordAction::Update {
        return Ok(());
    }
    require_member_or_admin(role)
}

fn require_acceptor_channel_role(role: &str) -> Result<(), IngestError> {
    if matches!(role, "owner" | "admin" | "member" | "guest") {
        Ok(())
    } else {
        Err(forbidden(
            "a named acceptor with private-channel membership is required",
        ))
    }
}

pub(super) fn conflict(message: &str) -> IngestError {
    IngestError::Rejected(format!("conflict: {message}"))
}

pub(super) fn forbidden(message: &str) -> IngestError {
    IngestError::AuthFailed(format!("forbidden: {message}"))
}

pub(super) fn invalid(message: impl Into<String>) -> IngestError {
    IngestError::Rejected(format!("invalid: {}", message.into()))
}

pub(super) fn internal(error: impl std::fmt::Display) -> IngestError {
    IngestError::Internal(format!("error: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prospect_actions_allow_agents_to_update_but_not_create_or_archive() {
        for role in ["owner", "admin", "member"] {
            for action in [
                RecordAction::Create,
                RecordAction::Update,
                RecordAction::Archive,
                RecordAction::Restore,
            ] {
                assert!(require_prospect_actor(role, action).is_ok(), "{role}");
            }
        }
        assert!(require_prospect_actor("bot", RecordAction::Update).is_ok());
        for action in [
            RecordAction::Create,
            RecordAction::Archive,
            RecordAction::Restore,
        ] {
            assert!(require_prospect_actor("bot", action).is_err());
        }
        for action in [RecordAction::Create, RecordAction::Update] {
            assert!(require_prospect_actor("guest", action).is_err());
        }
    }

    #[test]
    fn invoice_issue_requires_a_due_date() {
        let version = InvoiceVersion {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id: Uuid::from_u128(1),
            invoice_id: Uuid::from_u128(2),
            version: 2,
            previous_version_event_id: Some("a".repeat(64)),
            proposal_version_event_id: Some("b".repeat(64)),
            expected_head_event_id: Some("c".repeat(64)),
            action: InvoiceVersionAction::Issue,
            currency: "ZAR".into(),
            lines: vec![buzz_core::business_records::ProposalLine {
                service_id: None,
                description: "Approved work".into(),
                quantity_hundredths: 100,
                unit_amount_minor: 100,
            }],
            tax_lines: Vec::new(),
            seller_tax_number: None,
            customer_tax_number: None,
            total_minor: 100,
            status: InvoiceStatus::Issued,
            due_at: None,
            void_reason: None,
        };

        assert!(matches!(
            validate_invoice_version(&version),
            Err(IngestError::Rejected(message)) if message.contains("requires a due date")
        ));
        assert!(validate_invoice_version(&InvoiceVersion {
            due_at: Some(1),
            ..version
        })
        .is_ok());
    }

    fn direct_invoice_version() -> InvoiceVersion {
        InvoiceVersion {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id: Uuid::from_u128(1),
            invoice_id: Uuid::from_u128(2),
            version: 1,
            previous_version_event_id: None,
            proposal_version_event_id: None,
            expected_head_event_id: None,
            action: InvoiceVersionAction::Create,
            currency: "ZAR".into(),
            lines: vec![buzz_core::business_records::ProposalLine {
                service_id: None,
                description: "First monthly retainer".into(),
                quantity_hundredths: 100,
                unit_amount_minor: 850_000,
            }],
            tax_lines: Vec::new(),
            seller_tax_number: None,
            customer_tax_number: None,
            total_minor: 850_000,
            status: InvoiceStatus::Draft,
            due_at: None,
            void_reason: None,
        }
    }

    #[test]
    fn a_direct_invoice_is_a_first_draft_without_a_proposal() {
        assert!(validate_invoice_version(&direct_invoice_version()).is_ok());
        for broken in [
            InvoiceVersion {
                version: 2,
                ..direct_invoice_version()
            },
            InvoiceVersion {
                proposal_version_event_id: Some("a".repeat(64)),
                ..direct_invoice_version()
            },
            InvoiceVersion {
                previous_version_event_id: Some("a".repeat(64)),
                ..direct_invoice_version()
            },
            InvoiceVersion {
                expected_head_event_id: Some("a".repeat(64)),
                ..direct_invoice_version()
            },
            InvoiceVersion {
                status: InvoiceStatus::Issued,
                ..direct_invoice_version()
            },
        ] {
            assert!(validate_invoice_version(&broken).is_err());
        }
    }

    fn prospect_action() -> ProspectAction {
        let prospect_id = Uuid::from_u128(31);
        ProspectAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            prospect_id,
            action: RecordAction::Create,
            expected_head_event_id: None,
            prospect: buzz_core::business_records::ProspectRecordInput {
                prospect_id,
                party: buzz_core::business_records::PartyRecord {
                    party_id: prospect_id,
                    party_type: "organization".into(),
                    display_name: "Example Studio".into(),
                    external_ids: Vec::new(),
                },
                industry: "Professional Services".into(),
                vertical: "Marketing Agency".into(),
                fit_score: Some(87),
                origin: Some(buzz_core::business_records::ProspectOrigin::Discovery),
                discovery_tags: Vec::new(),
                potential_monthly_value_minor: Some(450_000),
                website: Some("https://example.test".into()),
                contact_name: Some("Rene Example".into()),
                location: Some("Cape Town".into()),
                email: Some("hello@example.test".into()),
                phone: None,
                evidence: Vec::new(),
                last_verified_at: None,
                qualification: buzz_core::business_records::ProspectQualification::Unreviewed,
                saved: false,
                stage: ProspectStage::Qualified,
                lost_reason: None,
            },
            activity: None,
        }
    }

    fn event_with_tags(tags: Vec<Tag>) -> Event {
        EventBuilder::new(Kind::Custom(KIND_CLIENT_ACTION as u16), "{}")
            .tags(tags)
            .sign_with_keys(&nostr::Keys::generate())
            .expect("signed command")
    }

    #[test]
    fn command_coordinates_accept_one_exact_h_and_d_tag() {
        let channel_id = Uuid::from_u128(1);
        let d_tag = client_d_tag(channel_id, "client", channel_id);
        let event = event_with_tags(vec![
            Tag::parse(["h", channel_id.to_string().as_str()]).expect("h tag"),
            Tag::parse(["d", d_tag.as_str()]).expect("d tag"),
        ]);

        assert_eq!(
            command_coordinates(&event).expect("coordinates"),
            (channel_id, d_tag)
        );
    }

    #[test]
    fn command_coordinates_reject_missing_or_duplicate_scope_tags() {
        let channel_id = Uuid::from_u128(1);
        let h = Tag::parse(["h", channel_id.to_string().as_str()]).expect("h tag");
        let d = Tag::parse(["d", "client:1:client:1"]).expect("d tag");
        let missing_h = event_with_tags(vec![d.clone()]);
        let duplicate_h = event_with_tags(vec![h.clone(), h, d]);

        assert!(matches!(
            command_coordinates(&missing_h),
            Err(IngestError::Rejected(message)) if message.contains("exactly one h tag")
        ));
        assert!(matches!(
            command_coordinates(&duplicate_h),
            Err(IngestError::Rejected(message)) if message.contains("exactly one h tag")
        ));
    }

    #[test]
    fn command_coordinates_rejects_extended_tag_shapes() {
        let channel_id = Uuid::from_u128(1);
        let event = event_with_tags(vec![
            Tag::parse(["h", channel_id.to_string().as_str(), "extra"]).expect("h tag"),
            Tag::parse(["d", "client:1:client:1"]).expect("d tag"),
        ]);

        assert!(matches!(
            command_coordinates(&event),
            Err(IngestError::Rejected(message)) if message.contains("exactly two values")
        ));
    }

    #[test]
    fn lifecycle_actions_archive_and_restore_canonical_status() {
        assert!(
            resolve_lifecycle_status(RecordAction::Create, None, Some("archived"), "active")
                .is_err()
        );
        assert!(resolve_lifecycle_status(
            RecordAction::Update,
            Some("active"),
            Some("archived"),
            "active"
        )
        .is_err());
        assert_eq!(
            resolve_lifecycle_status(RecordAction::Archive, Some("active"), None, "active")
                .expect("archive"),
            "archived"
        );
        assert_eq!(
            resolve_lifecycle_status(RecordAction::Restore, Some("archived"), None, "open")
                .expect("restore"),
            "open"
        );
        assert!(resolve_lifecycle_status(
            RecordAction::Update,
            Some("archived"),
            Some("active"),
            "active"
        )
        .is_err());
        assert!(
            resolve_lifecycle_status(RecordAction::Restore, Some("active"), None, "active")
                .is_err()
        );
    }

    #[test]
    fn service_validation_rejects_negative_fees_and_empty_scope() {
        let service_id = Uuid::from_u128(41);
        let mut action = ServiceAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            service_id,
            action: RecordAction::Create,
            expected_head_event_id: None,
            service: buzz_core::business_records::ServiceRecord {
                service_id,
                name: "Monthly social content".into(),
                description: "Planning and publishing".into(),
                currency: "ZAR".into(),
                monthly_fee_minor: 25_000,
                posts_per_month: 12,
                revision_rounds: 2,
            },
        };
        assert!(validate_service_action(&action).is_ok());
        action.service.monthly_fee_minor = -1;
        assert!(validate_service_action(&action).is_err());
        action.service.monthly_fee_minor = 25_000;
        action.service.posts_per_month = 0;
        assert!(validate_service_action(&action).is_err());
    }

    #[test]
    fn prospect_validation_requires_https_evidence_and_a_lost_reason() {
        let mut action = prospect_action();
        assert!(validate_prospect_action(&action).is_ok());
        action
            .prospect
            .evidence
            .push(buzz_core::business_records::ProspectEvidence {
                title: "Profile".into(),
                url: "http://example.test/profile".into(),
                excerpt: "Public profile".into(),
                observed_at: 1,
            });
        assert!(validate_prospect_action(&action).is_err());
        action.prospect.evidence[0].url = "https://example.test/profile".into();
        action.prospect.stage = ProspectStage::Lost;
        assert!(validate_prospect_action(&action).is_err());
        action.prospect.lost_reason = Some("Timing".into());
        assert!(validate_prospect_action(&action).is_ok());
    }

    #[test]
    fn prospect_validation_accepts_known_discovery_tags_and_rejects_invalid_ones() {
        let mut action = prospect_action();
        action.prospect.discovery_tags =
            vec![buzz_core::business_records::ProspectDiscoveryTagInput {
                tag: "wedding guest".into(),
                kind: "customer".into(),
                picked: true,
            }];
        assert!(validate_prospect_action(&action).is_ok());

        action.prospect.discovery_tags[0].kind = "unknown".into();
        assert!(validate_prospect_action(&action).is_err());
        action.prospect.discovery_tags[0].kind = "customer".into();
        action.prospect.discovery_tags[0].tag = " ".into();
        assert!(validate_prospect_action(&action).is_err());
        action.prospect.discovery_tags[0].tag = "x".repeat(241);
        assert!(validate_prospect_action(&action).is_err());
    }

    #[test]
    fn proposal_revision_activity_requires_a_proposal_and_version_pair() {
        let mut action = prospect_action();
        action.activity = Some(buzz_core::business_records::ProspectActivityInput {
            activity_id: Uuid::from_u128(44),
            activity_kind: buzz_core::business_records::ProspectActivityKind::Note,
            content: "Clarify the monthly reporting scope".into(),
            proposal_id: Some(Uuid::from_u128(45)),
            proposal_version_event_id: None,
        });
        assert!(validate_prospect_action(&action).is_err());

        let activity = action.activity.as_mut().expect("revision activity");
        activity.proposal_version_event_id = Some("a".repeat(64));
        assert!(validate_prospect_action(&action).is_ok());

        action
            .activity
            .as_mut()
            .expect("revision activity")
            .proposal_version_event_id = Some("not-an-event-id".into());
        assert!(validate_prospect_action(&action).is_err());
    }

    #[test]
    fn proposal_acceptance_requires_complete_exact_terms_evidence() {
        let mut acceptance = ProposalAcceptance {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            proposal_id: Uuid::from_u128(1),
            proposal_version_event_id: "a".repeat(64),
            proposal_version_digest: "b".repeat(64),
            conversion_id: Uuid::from_u128(2),
            client_id: Uuid::from_u128(3),
            work_item_id: Uuid::from_u128(4),
            draft_invoice_id: Uuid::from_u128(5),
            evidence: None,
        };
        assert!(validate_proposal_acceptance_evidence(&acceptance).is_err());
        acceptance.evidence = Some(buzz_core::business_records::ProposalAcceptanceEvidence {
            accepted_by_name: "Lerato Molefe".into(),
            accepted_at: 1,
            evidence_reference: "signature record".into(),
            exact_terms_confirmed: true,
        });
        assert!(validate_proposal_acceptance_evidence(&acceptance).is_ok());
        acceptance
            .evidence
            .as_mut()
            .expect("test evidence")
            .exact_terms_confirmed = false;
        assert!(validate_proposal_acceptance_evidence(&acceptance).is_err());
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use buzz_core::business_records::{
        deliverable_approval_d_tag, deliverable_version_d_tag, DeliverableApproval,
        DraftInvoiceHead, InvoiceTaxLine, ProposalLine, WorkItemHeadInput,
    };
    use buzz_db::channel::{ChannelType, ChannelVisibility};
    use nostr::{Keys, Tag};
    use serde::Serialize;
    use std::time::Duration;
    use tokio::sync::Notify;

    fn test_acceptance_evidence() -> buzz_core::business_records::ProposalAcceptanceEvidence {
        buzz_core::business_records::ProposalAcceptanceEvidence {
            accepted_by_name: "Test acceptor".into(),
            accepted_at: chrono::Utc::now().timestamp(),
            evidence_reference: "signed test acceptance".into(),
            exact_terms_confirmed: true,
        }
    }

    static BUSINESS_RECORDS_DB_TEST_LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> =
        std::sync::OnceLock::new();

    struct Fixture {
        pool: sqlx::PgPool,
        state: Arc<AppState>,
        tenant: TenantContext,
        _serial_guard: tokio::sync::MutexGuard<'static, ()>,
    }

    async fn fixture() -> Fixture {
        let serial_guard = BUSINESS_RECORDS_DB_TEST_LOCK
            .get_or_init(|| tokio::sync::Mutex::new(()))
            .lock()
            .await;
        let database_url = std::env::var("BUZZ_TEST_DATABASE_URL")
            .expect("BUZZ_TEST_DATABASE_URL must name the disposable test database");
        let pool = sqlx::PgPool::connect(&database_url)
            .await
            .expect("connect to the disposable test database");
        let state = crate::state::tests::test_state_with_database_url_and_acquire_timeout(
            &database_url,
            Duration::from_secs(10),
        )
        .await;
        let community_id = Uuid::new_v4();
        let host = format!("business-records-{}.test", community_id.simple());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
            .bind(community_id)
            .bind(&host)
            .execute(&pool)
            .await
            .expect("insert test community");
        Fixture {
            pool,
            state,
            tenant: TenantContext::resolved(CommunityId::from_uuid(community_id), host),
            _serial_guard: serial_guard,
        }
    }

    async fn private_stream(fixture: &Fixture, name: &str, owner: &Keys) -> Uuid {
        let pubkey = owner.public_key().to_bytes();
        let channel = fixture
            .state
            .db
            .create_channel(
                fixture.tenant.community(),
                &format!("{name}-{}", Uuid::new_v4().simple()),
                ChannelType::Stream,
                ChannelVisibility::Private,
                None,
                &pubkey,
                None,
            )
            .await
            .expect("create private stream channel");
        channel.id
    }

    async fn business_stream(fixture: &Fixture, owner: &Keys) -> Uuid {
        let channel_id = private_stream(fixture, "business", owner).await;
        fixture
            .state
            .db
            .add_relay_member(
                fixture.tenant.community(),
                &owner.public_key().to_hex(),
                "owner",
                None,
            )
            .await
            .expect("add community owner");
        channel_id
    }

    fn auth(keys: &Keys) -> IngestAuth {
        IngestAuth::Http {
            pubkey: keys.public_key(),
            scopes: vec![buzz_auth::Scope::MessagesWrite],
            auth_method: crate::handlers::ingest::HttpAuthMethod::DevPubkey,
        }
    }

    fn scoped_nip42_auth(keys: &Keys, channel_ids: Vec<Uuid>) -> IngestAuth {
        IngestAuth::Nip42 {
            pubkey: keys.public_key(),
            scopes: vec![buzz_auth::Scope::MessagesWrite],
            channel_ids: Some(channel_ids),
            conn_id: Uuid::new_v4(),
        }
    }

    fn signed_command<T: Serialize>(
        keys: &Keys,
        kind: u32,
        channel_id: Uuid,
        d_tag: &str,
        content: &T,
    ) -> Event {
        let h_tag = Tag::parse(["h", channel_id.to_string().as_str()]).expect("h tag");
        let d_tag = Tag::parse(["d", d_tag]).expect("d tag");
        let content = serde_json::to_string(content).expect("serialize command");
        EventBuilder::new(Kind::Custom(kind as u16), content)
            .tags([h_tag, d_tag])
            .sign_with_keys(keys)
            .expect("sign command")
    }

    async fn add_community_role(fixture: &Fixture, keys: &Keys, role: &str) {
        fixture
            .state
            .db
            .add_relay_member(
                fixture.tenant.community(),
                &keys.public_key().to_hex(),
                role,
                None,
            )
            .await
            .expect("add test community role");
    }

    async fn seed_draft_invoice(
        fixture: &Fixture,
        keys: &Keys,
        client_id: Uuid,
        invoice_id: Uuid,
        total_minor: i64,
        due_at: Option<i64>,
    ) -> InvoiceHead {
        let proposal_id = Uuid::new_v4();
        let proposal_version_event_id = "a".repeat(64);
        let lines = vec![ProposalLine {
            service_id: None,
            description: "Test project work".into(),
            quantity_hundredths: 100,
            unit_amount_minor: total_minor,
        }];
        let version = InvoiceVersion {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            version: 1,
            previous_version_event_id: None,
            proposal_version_event_id: Some(proposal_version_event_id.clone()),
            expected_head_event_id: None,
            action: InvoiceVersionAction::ProposalAcceptance,
            currency: "ZAR".into(),
            lines: lines.clone(),
            tax_lines: Vec::new(),
            seller_tax_number: None,
            customer_tax_number: None,
            total_minor,
            status: InvoiceStatus::Draft,
            due_at,
            void_reason: None,
        };
        let version_d = invoice_version_d_tag(client_id, invoice_id, 1);
        let version_event =
            signed_command(keys, KIND_INVOICE_VERSION, client_id, &version_d, &version);
        let (_, inserted) = fixture
            .state
            .db
            .insert_event(fixture.tenant.community(), &version_event, Some(client_id))
            .await
            .expect("insert seeded invoice version");
        assert!(inserted, "seeded invoice version is unique");

        let head = InvoiceHead {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            proposal_id,
            proposal_version_event_id,
            currency: "ZAR".into(),
            lines,
            tax_lines: Vec::new(),
            seller_tax_number: None,
            customer_tax_number: None,
            total_minor,
            credited_minor: 0,
            written_off_minor: 0,
            collected_minor: 0,
            outstanding_minor: 0,
            payment_evidence_count: 0,
            version: 1,
            current_version_event_id: version_event.id.to_hex(),
            status: InvoiceStatus::Draft,
            due_at,
            issued_at: None,
            source_event_id: version_event.id.to_hex(),
        };
        let head_d = invoice_head_d_tag(client_id, invoice_id);
        let head_event = relay_event(KIND_INVOICE_HEAD, client_id, &head_d, &head, &fixture.state)
            .expect("sign seeded invoice head");
        let replaced = fixture
            .state
            .db
            .replace_parameterized_event(
                fixture.tenant.community(),
                &head_event,
                &head_d,
                Some(client_id),
            )
            .await
            .expect("insert seeded invoice head");
        assert!(replaced.1, "seeded invoice head should be inserted");
        head
    }

    async fn issue_test_invoice(
        fixture: &Fixture,
        keys: &Keys,
        client_id: Uuid,
        invoice_id: Uuid,
    ) -> (StoredEvent, InvoiceHead) {
        let (head_event, head) = current_invoice_head(
            &fixture.state,
            fixture.tenant.community(),
            client_id,
            client_id,
            invoice_id,
        )
        .await
        .expect("load draft invoice");
        let version = InvoiceVersion {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            version: head.version + 1,
            previous_version_event_id: Some(head.current_version_event_id.clone()),
            proposal_version_event_id: Some(head.proposal_version_event_id.clone()),
            expected_head_event_id: Some(head_event.event.id.to_hex()),
            action: InvoiceVersionAction::Issue,
            currency: head.currency.clone(),
            lines: head.lines.clone(),
            tax_lines: head.tax_lines.clone(),
            seller_tax_number: head.seller_tax_number.clone(),
            customer_tax_number: head.customer_tax_number.clone(),
            total_minor: head.total_minor,
            status: InvoiceStatus::Issued,
            due_at: head.due_at,
            void_reason: None,
        };
        let version_d = invoice_version_d_tag(client_id, invoice_id, version.version);
        let event = signed_command(keys, KIND_INVOICE_VERSION, client_id, &version_d, &version);
        let result = handle(&fixture.tenant, &fixture.state, event, auth(keys))
            .await
            .expect("issue test invoice through the production broker");
        assert!(result.accepted);
        current_invoice_head(
            &fixture.state,
            fixture.tenant.community(),
            client_id,
            client_id,
            invoice_id,
        )
        .await
        .expect("load issued invoice")
    }

    fn payment_evidence(
        client_id: Uuid,
        invoice_id: Uuid,
        invoice_head_event_id: &str,
        amount_minor: i64,
        currency: &str,
    ) -> PaymentEvidence {
        PaymentEvidence {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            payment_id: Uuid::new_v4(),
            provider: "manual".into(),
            provider_reference: None,
            amount_minor,
            currency: currency.into(),
            occurred_at: chrono::Utc::now().timestamp(),
            evidence_ref: "test receipt".into(),
            expected_invoice_head_event_id: invoice_head_event_id.into(),
        }
    }

    fn money_adjustment(
        client_id: Uuid,
        invoice_id: Uuid,
        invoice_head_event_id: &str,
        adjustment_type: MoneyAdjustmentType,
        amount_minor: i64,
    ) -> MoneyAdjustment {
        MoneyAdjustment {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            adjustment_id: Uuid::new_v4(),
            adjustment_type,
            amount_minor,
            currency: "ZAR".into(),
            occurred_at: chrono::Utc::now().timestamp(),
            reason: "Documented test adjustment".into(),
            evidence_ref: "test supporting evidence".into(),
            expected_invoice_head_event_id: invoice_head_event_id.into(),
        }
    }

    #[test]
    fn money_adjustments_require_a_positive_effective_date() {
        let invoice_head_event_id = "a".repeat(64);
        let mut adjustment = money_adjustment(
            Uuid::new_v4(),
            Uuid::new_v4(),
            &invoice_head_event_id,
            MoneyAdjustmentType::CreditNote,
            100,
        );
        assert!(validate_money_adjustment(&adjustment).is_ok());

        adjustment.occurred_at = 0;
        assert!(validate_money_adjustment(&adjustment).is_err());
    }

    fn party_action(
        party_id: Uuid,
        action: RecordAction,
        expected_head_event_id: Option<String>,
        display_name: &str,
    ) -> PartyAction {
        PartyAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            party_id,
            action,
            expected_head_event_id,
            party: buzz_core::business_records::PartyRecord {
                party_id,
                party_type: "organization".into(),
                display_name: display_name.into(),
                external_ids: Vec::new(),
            },
        }
    }

    async fn create_party(
        fixture: &Fixture,
        keys: &Keys,
        business_channel_id: Uuid,
        party_id: Uuid,
    ) -> (Event, StoredEvent) {
        let d_tag = business_d_tag(*fixture.tenant.community().as_uuid(), "party", party_id);
        let event = signed_command(
            keys,
            KIND_PARTY_ACTION,
            business_channel_id,
            &d_tag,
            &party_action(party_id, RecordAction::Create, None, "Prospect"),
        );
        let result = handle(&fixture.tenant, &fixture.state, event.clone(), auth(keys))
            .await
            .expect("create party through production handler");
        assert!(result.accepted);
        let head = current_head::<PartyHead>(
            &fixture.state,
            fixture.tenant.community(),
            KIND_PARTY_HEAD,
            &d_tag,
        )
        .await
        .expect("load party head")
        .expect("party head exists");
        (event, head)
    }

    fn proposal_version(
        proposal_id: Uuid,
        party_id: Uuid,
        acceptor: &Keys,
        revision: u32,
        previous_version_event_id: Option<String>,
    ) -> ProposalVersion {
        ProposalVersion {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            proposal_id,
            prospect_party_id: party_id,
            named_acceptor_pubkey: acceptor.public_key().to_hex(),
            revision,
            previous_version_event_id,
            expires_at: None,
            currency: "USD".into(),
            lines: vec![ProposalLine {
                service_id: None,
                description: "Initial scope".into(),
                quantity_hundredths: 100,
                unit_amount_minor: 1000,
            }],
            terms: "Payment due on completion".into(),
        }
    }

    fn client_action(
        client_id: Uuid,
        party_id: Uuid,
        action: RecordAction,
        expected_head_event_id: Option<String>,
        display_name: &str,
        approver: &Keys,
    ) -> ClientAction {
        ClientAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            action,
            expected_head_event_id,
            head: buzz_core::business_records::ClientHeadInput {
                schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
                client_id,
                party_id,
                display_name: display_name.into(),
                approver_pubkeys: vec![approver.public_key().to_hex()],
                status: "active".into(),
            },
        }
    }

    fn work_item_action(
        client_id: Uuid,
        work_item_id: Uuid,
        action: RecordAction,
        expected_head_event_id: Option<String>,
        title: &str,
        actor: &Keys,
    ) -> WorkItemAction {
        let pubkey = actor.public_key().to_hex();
        WorkItemAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            work_item_id,
            action,
            expected_head_event_id,
            head: WorkItemHeadInput {
                schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
                client_id,
                work_item_id,
                title: title.into(),
                status: "open".into(),
                assigned_pubkeys: vec![pubkey.clone()],
                approver_pubkeys: vec![pubkey],
                deliverables: Vec::new(),
            },
        }
    }

    fn deliverable_version(
        client_id: Uuid,
        work_item_id: Uuid,
        deliverable_id: Uuid,
        version: u32,
        previous_version_event_id: Option<String>,
        body_text: &str,
    ) -> DeliverableVersion {
        let body = serde_json::json!({"text": body_text});
        let content_digest = digest_hex(&serde_json::to_vec(&body).expect("serialize body"));
        DeliverableVersion {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            work_item_id,
            deliverable_id,
            version,
            previous_version_event_id,
            content_digest,
            media_digests: Vec::new(),
            body,
        }
    }

    async fn expect_duplicate(fixture: &Fixture, keys: &Keys, channel_id: Uuid, event: &Event) {
        let replay = handle(&fixture.tenant, &fixture.state, event.clone(), auth(keys))
            .await
            .expect("committed command retry must be accepted");
        assert!(replay.accepted);
        assert!(replay.message.contains("duplicate"));
        assert_eq!(replay.event_id, event.id.to_hex());
        assert_eq!(
            channel_id,
            command_coordinates(event).expect("coordinates").0
        );
    }

    async fn expect_event_missing(fixture: &Fixture, event: &Event) {
        let stored = fixture
            .state
            .db
            .get_event_by_id_for_event_write(fixture.tenant.community(), &event.id.to_bytes())
            .await
            .expect("query command event");
        assert!(stored.is_none(), "rejected command must not be stored");
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn production_handler_rejects_party_and_proposal_commands_from_client_channel() {
        let fixture = fixture().await;
        let business_owner = Keys::generate();
        let business_channel_id = business_stream(&fixture, &business_owner).await;
        let existing_party_id = Uuid::new_v4();
        create_party(
            &fixture,
            &business_owner,
            business_channel_id,
            existing_party_id,
        )
        .await;

        let client_member = Keys::generate();
        let client_channel_id = private_stream(&fixture, "client", &client_member).await;
        let poisoned_party_id = Uuid::new_v4();
        let party_d_tag = business_d_tag(
            *fixture.tenant.community().as_uuid(),
            "party",
            poisoned_party_id,
        );
        let party_event = signed_command(
            &client_member,
            KIND_PARTY_ACTION,
            client_channel_id,
            &party_d_tag,
            &party_action(
                poisoned_party_id,
                RecordAction::Create,
                None,
                "Untrusted client party",
            ),
        );
        let party_result = handle(
            &fixture.tenant,
            &fixture.state,
            party_event.clone(),
            auth(&client_member),
        )
        .await;
        assert!(
            matches!(party_result, Err(IngestError::AuthFailed(message)) if message.contains("registered business channel"))
        );
        expect_event_missing(&fixture, &party_event).await;

        let proposal_id = Uuid::new_v4();
        let proposal = proposal_version(proposal_id, existing_party_id, &client_member, 1, None);
        let proposal_d_tag =
            proposal_version_d_tag(*fixture.tenant.community().as_uuid(), proposal_id, 1);
        let proposal_event = signed_command(
            &client_member,
            KIND_PROPOSAL_VERSION,
            client_channel_id,
            &proposal_d_tag,
            &proposal,
        );
        let proposal_result = handle(
            &fixture.tenant,
            &fixture.state,
            proposal_event.clone(),
            auth(&client_member),
        )
        .await;
        assert!(
            matches!(proposal_result, Err(IngestError::AuthFailed(message)) if message.contains("registered business channel"))
        );
        expect_event_missing(&fixture, &proposal_event).await;
        assert!(current_head::<PartyHead>(
            &fixture.state,
            fixture.tenant.community(),
            KIND_PARTY_HEAD,
            &party_d_tag,
        )
        .await
        .expect("check poisoned party head")
        .is_none());
        assert!(current_head::<ProposalHead>(
            &fixture.state,
            fixture.tenant.community(),
            KIND_PROPOSAL_HEAD,
            &business_d_tag(
                *fixture.tenant.community().as_uuid(),
                "proposal",
                proposal_id
            ),
        )
        .await
        .expect("check proposal head")
        .is_none());
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn client_link_requires_business_membership_and_an_internal_party_head() {
        let fixture = fixture().await;
        let business_owner = Keys::generate();
        let business_channel_id = business_stream(&fixture, &business_owner).await;
        let party_id = Uuid::new_v4();
        create_party(&fixture, &business_owner, business_channel_id, party_id).await;

        let unrelated_client_admin = Keys::generate();
        let client_channel_id =
            private_stream(&fixture, "unrelated-client", &unrelated_client_admin).await;
        let client_id = client_channel_id;
        let action = client_action(
            client_id,
            party_id,
            RecordAction::Create,
            None,
            "Attempted link",
            &unrelated_client_admin,
        );
        let d_tag = client_d_tag(client_id, "client", client_id);
        let event = signed_command(
            &unrelated_client_admin,
            KIND_CLIENT_ACTION,
            client_channel_id,
            &d_tag,
            &action,
        );
        let denied = handle(
            &fixture.tenant,
            &fixture.state,
            event.clone(),
            auth(&unrelated_client_admin),
        )
        .await;
        assert!(
            matches!(denied, Err(IngestError::AuthFailed(message)) if message.contains("party link is not authorized"))
        );
        expect_event_missing(&fixture, &event).await;

        let foreign_party_id = Uuid::new_v4();
        let foreign_channel_owner = Keys::generate();
        let foreign_channel_id =
            private_stream(&fixture, "foreign-client", &foreign_channel_owner).await;
        let foreign_d_tag = business_d_tag(
            *fixture.tenant.community().as_uuid(),
            "party",
            foreign_party_id,
        );
        let foreign_head = PartyHead {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            party_id: foreign_party_id,
            status: "active".into(),
            party: buzz_core::business_records::PartyRecord {
                party_id: foreign_party_id,
                party_type: "organization".into(),
                display_name: "Foreign channel record".into(),
                external_ids: Vec::new(),
            },
            source_action_event_id: "0".repeat(64),
        };
        let foreign_head_event = relay_event(
            KIND_PARTY_HEAD,
            foreign_channel_id,
            &foreign_d_tag,
            &foreign_head,
            &fixture.state,
        )
        .expect("sign foreign fixture head");
        fixture
            .state
            .db
            .replace_parameterized_event(
                fixture.tenant.community(),
                &foreign_head_event,
                &foreign_d_tag,
                Some(foreign_channel_id),
            )
            .await
            .expect("insert foreign-channel fixture head");

        let internal_member_client_channel =
            private_stream(&fixture, "business-member-client", &business_owner).await;
        let foreign_link = client_action(
            internal_member_client_channel,
            foreign_party_id,
            RecordAction::Create,
            None,
            "Must not copy foreign party",
            &business_owner,
        );
        let foreign_client_d = client_d_tag(
            internal_member_client_channel,
            "client",
            internal_member_client_channel,
        );
        let foreign_link_event = signed_command(
            &business_owner,
            KIND_CLIENT_ACTION,
            internal_member_client_channel,
            &foreign_client_d,
            &foreign_link,
        );
        let foreign_denied = handle(
            &fixture.tenant,
            &fixture.state,
            foreign_link_event.clone(),
            auth(&business_owner),
        )
        .await;
        assert!(
            matches!(foreign_denied, Err(IngestError::AuthFailed(message)) if message.contains("party link is not authorized"))
        );
        expect_event_missing(&fixture, &foreign_link_event).await;
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn nip42_scoped_client_link_rejects_out_of_scope_party_read_without_writes() {
        let fixture = fixture().await;
        let actor = Keys::generate();
        let business_channel_id = business_stream(&fixture, &actor).await;
        let party_id = Uuid::new_v4();
        create_party(&fixture, &actor, business_channel_id, party_id).await;
        let client_channel_id = private_stream(&fixture, "scoped-client-link", &actor).await;
        let actor_bytes = actor.public_key().to_bytes().to_vec();

        for channel_id in [business_channel_id, client_channel_id] {
            assert!(fixture
                .state
                .db
                .get_member_role(fixture.tenant.community(), channel_id, &actor_bytes)
                .await
                .expect("load actor membership")
                .is_some());
        }
        let secondary_business_read = install_secondary_channel_read_test_hook(business_channel_id);

        let client_d = client_d_tag(client_channel_id, "client", client_channel_id);
        let link_event = signed_command(
            &actor,
            KIND_CLIENT_ACTION,
            client_channel_id,
            &client_d,
            &client_action(
                client_channel_id,
                party_id,
                RecordAction::Create,
                None,
                "Scoped client",
                &actor,
            ),
        );
        let result = handle(
            &fixture.tenant,
            &fixture.state,
            link_event.clone(),
            scoped_nip42_auth(&actor, vec![client_channel_id]),
        )
        .await;

        assert!(matches!(
            result,
            Err(IngestError::AuthFailed(message)) if message.contains("not scoped to this channel")
        ));
        assert!(
            !secondary_business_read.was_observed(),
            "out-of-scope client link read the business channel"
        );
        expect_event_missing(&fixture, &link_event).await;
        assert!(current_head::<ClientHead>(
            &fixture.state,
            fixture.tenant.community(),
            KIND_CLIENT_HEAD,
            &client_d,
        )
        .await
        .expect("load client head")
        .is_none());
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn nip42_scoped_proposal_acceptance_rejects_out_of_scope_client_effects() {
        let fixture = fixture().await;
        let acceptor = Keys::generate();
        let business_channel_id = business_stream(&fixture, &acceptor).await;
        let party_id = Uuid::new_v4();
        create_party(&fixture, &acceptor, business_channel_id, party_id).await;
        let proposal_id = Uuid::new_v4();
        let proposal_event = signed_command(
            &acceptor,
            KIND_PROPOSAL_VERSION,
            business_channel_id,
            &proposal_version_d_tag(*fixture.tenant.community().as_uuid(), proposal_id, 1),
            &proposal_version(proposal_id, party_id, &acceptor, 1, None),
        );
        handle(
            &fixture.tenant,
            &fixture.state,
            proposal_event.clone(),
            auth(&acceptor),
        )
        .await
        .expect("create proposal version");

        let client_channel_id =
            private_stream(&fixture, "scoped-proposal-acceptance", &acceptor).await;
        let actor_bytes = acceptor.public_key().to_bytes().to_vec();
        for channel_id in [business_channel_id, client_channel_id] {
            assert!(fixture
                .state
                .db
                .get_member_role(fixture.tenant.community(), channel_id, &actor_bytes)
                .await
                .expect("load acceptor membership")
                .is_some());
        }

        let conversion_id = Uuid::new_v4();
        let work_item_id = Uuid::new_v4();
        let draft_invoice_id = Uuid::new_v4();
        let acceptance = ProposalAcceptance {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            proposal_id,
            proposal_version_event_id: proposal_event.id.to_hex(),
            proposal_version_digest: digest_hex(proposal_event.content.as_bytes()),
            conversion_id,
            client_id: client_channel_id,
            work_item_id,
            draft_invoice_id,
            evidence: Some(test_acceptance_evidence()),
        };
        let acceptance_d = business_d_tag(
            *fixture.tenant.community().as_uuid(),
            "conversion",
            conversion_id,
        );
        let acceptance_event = signed_command(
            &acceptor,
            KIND_PROPOSAL_ACCEPTANCE,
            business_channel_id,
            &acceptance_d,
            &acceptance,
        );
        let secondary_client_read = install_secondary_channel_read_test_hook(client_channel_id);
        let result = handle(
            &fixture.tenant,
            &fixture.state,
            acceptance_event.clone(),
            scoped_nip42_auth(&acceptor, vec![business_channel_id]),
        )
        .await;

        assert!(matches!(
            result,
            Err(IngestError::AuthFailed(message)) if message.contains("not scoped to this channel")
        ));
        assert!(
            !secondary_client_read.was_observed(),
            "out-of-scope proposal acceptance read the client channel"
        );
        expect_event_missing(&fixture, &acceptance_event).await;
        for (kind, d_tag) in [
            (
                KIND_CLIENT_HEAD,
                client_d_tag(client_channel_id, "client", client_channel_id),
            ),
            (
                KIND_WORK_ITEM_HEAD,
                client_d_tag(client_channel_id, "work", work_item_id),
            ),
            (
                KIND_INVOICE_HEAD,
                client_d_tag(client_channel_id, "invoice", draft_invoice_id),
            ),
            (KIND_PROPOSAL_CONVERSION_RECEIPT, acceptance_d),
        ] {
            let event = current_head::<serde_json::Value>(
                &fixture.state,
                fixture.tenant.community(),
                kind,
                &d_tag,
            )
            .await
            .expect("query side-effect event");
            assert!(
                event.is_none(),
                "out-of-scope head {kind}/{d_tag} was stored"
            );
        }
        let claim_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM business_proposal_conversion_claims \
             WHERE community_id = $1 AND conversion_id = $2",
        )
        .bind(fixture.tenant.community().as_uuid())
        .bind(conversion_id)
        .fetch_one(&fixture.pool)
        .await
        .expect("count conversion claims");
        assert_eq!(claim_count, 0, "out-of-scope conversion claim was stored");
        let client_group_d = client_channel_id.to_string();
        for kind in [
            KIND_NIP29_GROUP_METADATA,
            KIND_NIP29_GROUP_ADMINS,
            KIND_NIP29_GROUP_MEMBERS,
        ] {
            let event = current_head::<serde_json::Value>(
                &fixture.state,
                fixture.tenant.community(),
                kind,
                &client_group_d,
            )
            .await
            .expect("query client channel discovery event");
            assert!(
                event.is_none(),
                "out-of-scope acceptance emitted client channel discovery kind {kind}"
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn proposal_and_client_creation_require_an_active_internal_party() {
        let fixture = fixture().await;
        let business_owner = Keys::generate();
        let business_channel_id = business_stream(&fixture, &business_owner).await;
        let party_id = Uuid::new_v4();
        let (_, initial_party_head) =
            create_party(&fixture, &business_owner, business_channel_id, party_id).await;
        let initial_party: PartyHead = parse_content(&initial_party_head.event).expect("party");
        let archived_party_event = signed_command(
            &business_owner,
            KIND_PARTY_ACTION,
            business_channel_id,
            &business_d_tag(*fixture.tenant.community().as_uuid(), "party", party_id),
            &party_action(
                party_id,
                RecordAction::Archive,
                Some(initial_party_head.event.id.to_hex()),
                &initial_party.party.display_name,
            ),
        );
        handle(
            &fixture.tenant,
            &fixture.state,
            archived_party_event,
            auth(&business_owner),
        )
        .await
        .expect("archive party");

        let proposal_id = Uuid::new_v4();
        let proposal = proposal_version(proposal_id, party_id, &business_owner, 1, None);
        let proposal_d =
            proposal_version_d_tag(*fixture.tenant.community().as_uuid(), proposal_id, 1);
        let proposal_event = signed_command(
            &business_owner,
            KIND_PROPOSAL_VERSION,
            business_channel_id,
            &proposal_d,
            &proposal,
        );
        let proposal_result = handle(
            &fixture.tenant,
            &fixture.state,
            proposal_event.clone(),
            auth(&business_owner),
        )
        .await;
        assert!(
            matches!(proposal_result, Err(IngestError::Rejected(message)) if message.contains("prospect party is not available"))
        );
        expect_event_missing(&fixture, &proposal_event).await;

        let client_channel_id =
            private_stream(&fixture, "archived-party-client", &business_owner).await;
        let client = client_action(
            client_channel_id,
            party_id,
            RecordAction::Create,
            None,
            "Archived party client",
            &business_owner,
        );
        let client_d = client_d_tag(client_channel_id, "client", client_channel_id);
        let client_event = signed_command(
            &business_owner,
            KIND_CLIENT_ACTION,
            client_channel_id,
            &client_d,
            &client,
        );
        let client_result = handle(
            &fixture.tenant,
            &fixture.state,
            client_event.clone(),
            auth(&business_owner),
        )
        .await;
        assert!(
            matches!(client_result, Err(IngestError::AuthFailed(message)) if message.contains("party link is not authorized"))
        );
        expect_event_missing(&fixture, &client_event).await;
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn committed_mutable_commands_replay_after_their_heads_advance_and_old_approval_is_stale()
    {
        let fixture = fixture().await;
        let actor = Keys::generate();
        let business_channel_id = business_stream(&fixture, &actor).await;
        let party_id = Uuid::new_v4();
        let (party_create_event, party_head) =
            create_party(&fixture, &actor, business_channel_id, party_id).await;
        let party_update = signed_command(
            &actor,
            KIND_PARTY_ACTION,
            business_channel_id,
            &business_d_tag(*fixture.tenant.community().as_uuid(), "party", party_id),
            &party_action(
                party_id,
                RecordAction::Update,
                Some(party_head.event.id.to_hex()),
                "Updated prospect",
            ),
        );
        handle(&fixture.tenant, &fixture.state, party_update, auth(&actor))
            .await
            .expect("update party");
        expect_duplicate(&fixture, &actor, business_channel_id, &party_create_event).await;

        let client_channel_id = private_stream(&fixture, "retry-client", &actor).await;
        let client_d = client_d_tag(client_channel_id, "client", client_channel_id);
        let client_create = signed_command(
            &actor,
            KIND_CLIENT_ACTION,
            client_channel_id,
            &client_d,
            &client_action(
                client_channel_id,
                party_id,
                RecordAction::Create,
                None,
                "Retry client",
                &actor,
            ),
        );
        handle(
            &fixture.tenant,
            &fixture.state,
            client_create.clone(),
            auth(&actor),
        )
        .await
        .expect("create client");
        let client_head = current_head::<ClientHead>(
            &fixture.state,
            fixture.tenant.community(),
            KIND_CLIENT_HEAD,
            &client_d,
        )
        .await
        .expect("load client head")
        .expect("client head exists");
        let client_update = signed_command(
            &actor,
            KIND_CLIENT_ACTION,
            client_channel_id,
            &client_d,
            &client_action(
                client_channel_id,
                party_id,
                RecordAction::Update,
                Some(client_head.event.id.to_hex()),
                "Updated retry client",
                &actor,
            ),
        );
        handle(&fixture.tenant, &fixture.state, client_update, auth(&actor))
            .await
            .expect("update client");
        expect_duplicate(&fixture, &actor, client_channel_id, &client_create).await;

        let work_item_id = Uuid::new_v4();
        let work_d = client_d_tag(client_channel_id, "work", work_item_id);
        let work_create = signed_command(
            &actor,
            KIND_WORK_ITEM_ACTION,
            client_channel_id,
            &work_d,
            &work_item_action(
                client_channel_id,
                work_item_id,
                RecordAction::Create,
                None,
                "Retry work",
                &actor,
            ),
        );
        handle(
            &fixture.tenant,
            &fixture.state,
            work_create.clone(),
            auth(&actor),
        )
        .await
        .expect("create work item");
        let work_head = current_head::<WorkItemHead>(
            &fixture.state,
            fixture.tenant.community(),
            KIND_WORK_ITEM_HEAD,
            &work_d,
        )
        .await
        .expect("load work item")
        .expect("work item exists");
        let work_update = signed_command(
            &actor,
            KIND_WORK_ITEM_ACTION,
            client_channel_id,
            &work_d,
            &work_item_action(
                client_channel_id,
                work_item_id,
                RecordAction::Update,
                Some(work_head.event.id.to_hex()),
                "Updated retry work",
                &actor,
            ),
        );
        handle(&fixture.tenant, &fixture.state, work_update, auth(&actor))
            .await
            .expect("update work item");
        expect_duplicate(&fixture, &actor, client_channel_id, &work_create).await;

        let proposal_id = Uuid::new_v4();
        let proposal_d1 =
            proposal_version_d_tag(*fixture.tenant.community().as_uuid(), proposal_id, 1);
        let proposal_v1 = signed_command(
            &actor,
            KIND_PROPOSAL_VERSION,
            business_channel_id,
            &proposal_d1,
            &proposal_version(proposal_id, party_id, &actor, 1, None),
        );
        handle(
            &fixture.tenant,
            &fixture.state,
            proposal_v1.clone(),
            auth(&actor),
        )
        .await
        .expect("create proposal version");
        let proposal_d2 =
            proposal_version_d_tag(*fixture.tenant.community().as_uuid(), proposal_id, 2);
        let proposal_v2 = signed_command(
            &actor,
            KIND_PROPOSAL_VERSION,
            business_channel_id,
            &proposal_d2,
            &proposal_version(
                proposal_id,
                party_id,
                &actor,
                2,
                Some(proposal_v1.id.to_hex()),
            ),
        );
        handle(&fixture.tenant, &fixture.state, proposal_v2, auth(&actor))
            .await
            .expect("create second proposal version");
        expect_duplicate(&fixture, &actor, business_channel_id, &proposal_v1).await;

        let deliverable_id = Uuid::new_v4();
        let version1 = deliverable_version(
            client_channel_id,
            work_item_id,
            deliverable_id,
            1,
            None,
            "first deliverable version",
        );
        let version1_event = signed_command(
            &actor,
            KIND_DELIVERABLE_VERSION,
            client_channel_id,
            &deliverable_version_d_tag(client_channel_id, deliverable_id, 1),
            &version1,
        );
        handle(
            &fixture.tenant,
            &fixture.state,
            version1_event.clone(),
            auth(&actor),
        )
        .await
        .expect("create first deliverable version");
        let media_digest = digest_hex(&serde_json::to_vec(&Vec::<String>::new()).expect("media"));
        let approval = DeliverableApproval {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id: client_channel_id,
            work_item_id,
            deliverable_id,
            version_event_id: version1_event.id.to_hex(),
            content_digest: version1.content_digest.clone(),
            media_digest,
            decision: buzz_core::business_records::ApprovalDecision::Approved,
            note: Some("accepted exact version".into()),
        };
        let approval_d = deliverable_approval_d_tag(client_channel_id, &version1_event.id.to_hex());
        let approval_event = signed_command(
            &actor,
            KIND_DELIVERABLE_APPROVAL,
            client_channel_id,
            &approval_d,
            &approval,
        );
        handle(
            &fixture.tenant,
            &fixture.state,
            approval_event.clone(),
            auth(&actor),
        )
        .await
        .expect("approve exact current deliverable version");

        let version2 = deliverable_version(
            client_channel_id,
            work_item_id,
            deliverable_id,
            2,
            Some(version1_event.id.to_hex()),
            "second deliverable version",
        );
        let version2_event = signed_command(
            &actor,
            KIND_DELIVERABLE_VERSION,
            client_channel_id,
            &deliverable_version_d_tag(client_channel_id, deliverable_id, 2),
            &version2,
        );
        handle(
            &fixture.tenant,
            &fixture.state,
            version2_event,
            auth(&actor),
        )
        .await
        .expect("create second deliverable version");
        expect_duplicate(&fixture, &actor, client_channel_id, &version1_event).await;
        expect_duplicate(&fixture, &actor, client_channel_id, &approval_event).await;

        let stale_approval = DeliverableApproval {
            note: Some("stale approval attempt".into()),
            ..approval
        };
        let stale_approval_event = signed_command(
            &actor,
            KIND_DELIVERABLE_APPROVAL,
            client_channel_id,
            &approval_d,
            &stale_approval,
        );
        let stale_result = handle(
            &fixture.tenant,
            &fixture.state,
            stale_approval_event.clone(),
            auth(&actor),
        )
        .await;
        assert!(
            matches!(stale_result, Err(IngestError::Rejected(message)) if message.contains("approval must target the current deliverable version"))
        );
        expect_event_missing(&fixture, &stale_approval_event).await;
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn proposal_acceptance_replays_its_original_receipt_and_rejects_conflicting_claims() {
        let fixture = fixture().await;
        let acceptor = Keys::generate();
        let business_channel_id = business_stream(&fixture, &acceptor).await;
        let party_id = Uuid::new_v4();
        create_party(&fixture, &acceptor, business_channel_id, party_id).await;

        let proposal_id = Uuid::new_v4();
        let proposal_d =
            proposal_version_d_tag(*fixture.tenant.community().as_uuid(), proposal_id, 1);
        let proposal_event = signed_command(
            &acceptor,
            KIND_PROPOSAL_VERSION,
            business_channel_id,
            &proposal_d,
            &proposal_version(proposal_id, party_id, &acceptor, 1, None),
        );
        handle(
            &fixture.tenant,
            &fixture.state,
            proposal_event.clone(),
            auth(&acceptor),
        )
        .await
        .expect("create proposal version");
        let proposal_digest = digest_hex(proposal_event.content.as_bytes());
        let conversion_id = Uuid::new_v4();
        let client_id = Uuid::new_v4();
        let work_item_id = Uuid::new_v4();
        let draft_invoice_id = Uuid::new_v4();
        let acceptance = ProposalAcceptance {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            proposal_id,
            proposal_version_event_id: proposal_event.id.to_hex(),
            proposal_version_digest: proposal_digest,
            conversion_id,
            client_id,
            work_item_id,
            draft_invoice_id,
            evidence: Some(test_acceptance_evidence()),
        };
        let acceptance_d = business_d_tag(
            *fixture.tenant.community().as_uuid(),
            "conversion",
            conversion_id,
        );
        let acceptance_event = signed_command(
            &acceptor,
            KIND_PROPOSAL_ACCEPTANCE,
            business_channel_id,
            &acceptance_d,
            &acceptance,
        );
        let first = handle(
            &fixture.tenant,
            &fixture.state,
            acceptance_event.clone(),
            auth(&acceptor),
        )
        .await
        .expect("convert proposal");
        assert!(first.accepted);
        let receipt_id = first
            .message
            .split("receipt=")
            .nth(1)
            .expect("first result names receipt")
            .to_owned();

        let replay = handle(
            &fixture.tenant,
            &fixture.state,
            acceptance_event.clone(),
            auth(&acceptor),
        )
        .await
        .expect("exact acceptance replay");
        assert!(replay.accepted);
        assert!(replay.message.contains(&format!("receipt={receipt_id}")));

        let client_head = current_head::<ClientHead>(
            &fixture.state,
            fixture.tenant.community(),
            KIND_CLIENT_HEAD,
            &client_d_tag(client_id, "client", client_id),
        )
        .await
        .expect("load converted client")
        .expect("one converted client head");
        assert_eq!(client_head.channel_id, Some(client_id));
        let client_channel_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM channels WHERE community_id = $1 AND id = $2")
                .bind(fixture.tenant.community().as_uuid())
                .bind(client_id)
                .fetch_one(&fixture.pool)
                .await
                .expect("count converted client channels");
        assert_eq!(client_channel_count, 1);
        let work_head = current_head::<WorkItemHead>(
            &fixture.state,
            fixture.tenant.community(),
            KIND_WORK_ITEM_HEAD,
            &client_d_tag(client_id, "work", work_item_id),
        )
        .await
        .expect("load converted work")
        .expect("one converted work head");
        assert_eq!(work_head.channel_id, Some(client_id));
        let invoice_head = current_head::<DraftInvoiceHead>(
            &fixture.state,
            fixture.tenant.community(),
            KIND_INVOICE_HEAD,
            &client_d_tag(client_id, "invoice", draft_invoice_id),
        )
        .await
        .expect("load converted invoice")
        .expect("one converted invoice head");
        assert_eq!(invoice_head.channel_id, Some(client_id));
        let receipt_event_id = hex::decode(&receipt_id).expect("receipt id is lowercase hex");
        let receipt = fixture
            .state
            .db
            .get_event_by_id_for_event_write(fixture.tenant.community(), &receipt_event_id)
            .await
            .expect("load conversion receipt by event id")
            .expect("one conversion receipt");
        assert_eq!(
            receipt.event.kind.as_u16() as u32,
            KIND_PROPOSAL_CONVERSION_RECEIPT
        );
        let receipt_content: buzz_core::business_records::ProposalConversionReceipt =
            parse_content(&receipt.event).expect("parse conversion receipt");
        assert_eq!(receipt_content.conversion_id, conversion_id);
        let receipt_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM events \
             WHERE community_id = $1 AND kind = $2 AND tags @> $3",
        )
        .bind(fixture.tenant.community().as_uuid())
        .bind(KIND_PROPOSAL_CONVERSION_RECEIPT as i32)
        .bind(serde_json::json!([["d", &acceptance_d]]))
        .fetch_one(&fixture.pool)
        .await
        .expect("count conversion receipts");
        assert_eq!(receipt_count, 1);

        let conflicting = ProposalAcceptance {
            work_item_id: Uuid::new_v4(),
            draft_invoice_id: Uuid::new_v4(),
            ..acceptance
        };
        let conflicting_event = signed_command(
            &acceptor,
            KIND_PROPOSAL_ACCEPTANCE,
            business_channel_id,
            &acceptance_d,
            &conflicting,
        );
        let denied = handle(
            &fixture.tenant,
            &fixture.state,
            conflicting_event.clone(),
            auth(&acceptor),
        )
        .await;
        assert!(
            matches!(denied, Err(IngestError::Rejected(message)) if message.contains("conversion id was already claimed"))
        );
        expect_event_missing(&fixture, &conflicting_event).await;
        let claim_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM business_proposal_conversion_claims \
             WHERE community_id = $1 AND conversion_id = $2",
        )
        .bind(fixture.tenant.community().as_uuid())
        .bind(conversion_id)
        .fetch_one(&fixture.pool)
        .await
        .expect("count conversion claims");
        assert_eq!(claim_count, 1);
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn concurrent_proposal_update_and_acceptance_use_canonical_lock_order() {
        let fixture = fixture().await;
        let actor = Keys::generate();
        let business_channel_id = business_stream(&fixture, &actor).await;
        let party_id = Uuid::new_v4();
        create_party(&fixture, &actor, business_channel_id, party_id).await;
        let client_channel_id = private_stream(&fixture, "lock-order-client", &actor).await;

        let proposal_id = Uuid::new_v4();
        let proposal_v1 = signed_command(
            &actor,
            KIND_PROPOSAL_VERSION,
            business_channel_id,
            &proposal_version_d_tag(*fixture.tenant.community().as_uuid(), proposal_id, 1),
            &proposal_version(proposal_id, party_id, &actor, 1, None),
        );
        handle(
            &fixture.tenant,
            &fixture.state,
            proposal_v1.clone(),
            auth(&actor),
        )
        .await
        .expect("create proposal v1");

        let proposal_v2 = signed_command(
            &actor,
            KIND_PROPOSAL_VERSION,
            business_channel_id,
            &proposal_version_d_tag(*fixture.tenant.community().as_uuid(), proposal_id, 2),
            &proposal_version(
                proposal_id,
                party_id,
                &actor,
                2,
                Some(proposal_v1.id.to_hex()),
            ),
        );
        let acceptance = ProposalAcceptance {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            proposal_id,
            proposal_version_event_id: proposal_v1.id.to_hex(),
            proposal_version_digest: digest_hex(proposal_v1.content.as_bytes()),
            conversion_id: Uuid::new_v4(),
            client_id: client_channel_id,
            work_item_id: Uuid::new_v4(),
            draft_invoice_id: Uuid::new_v4(),
            evidence: Some(test_acceptance_evidence()),
        };
        let acceptance_event = signed_command(
            &actor,
            KIND_PROPOSAL_ACCEPTANCE,
            business_channel_id,
            &business_d_tag(
                *fixture.tenant.community().as_uuid(),
                "conversion",
                acceptance.conversion_id,
            ),
            &acceptance,
        );

        let paused = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        let other_lock_attempt = Arc::new(Notify::new());
        let other_first_lock = Arc::new(Notify::new());
        install_expected_head_lock_test_hook(
            proposal_v2.id.to_bytes(),
            Arc::clone(&paused),
            Arc::clone(&resume),
            Arc::clone(&other_lock_attempt),
            Arc::clone(&other_first_lock),
        );
        let hook_guard = ExpectedHeadLockTestHookGuard {
            resume: Arc::clone(&resume),
        };

        let version_tenant = fixture.tenant.clone();
        let version_state = Arc::clone(&fixture.state);
        let version_actor = actor.clone();
        let mut version_task = tokio::spawn(async move {
            handle(
                &version_tenant,
                &version_state,
                proposal_v2,
                auth(&version_actor),
            )
            .await
        });
        if tokio::time::timeout(Duration::from_secs(5), paused.notified())
            .await
            .is_err()
        {
            resume.notify_one();
            version_task.abort();
            let _ = version_task.await;
            drop(hook_guard);
            panic!("proposal update did not acquire its first expected-head lock");
        }

        let acceptance_tenant = fixture.tenant.clone();
        let acceptance_state = Arc::clone(&fixture.state);
        let acceptance_actor = actor.clone();
        let acceptance_for_task = acceptance_event.clone();
        let mut acceptance_task = tokio::spawn(async move {
            handle(
                &acceptance_tenant,
                &acceptance_state,
                acceptance_for_task,
                auth(&acceptance_actor),
            )
            .await
        });
        if tokio::time::timeout(Duration::from_secs(5), other_lock_attempt.notified())
            .await
            .is_err()
        {
            resume.notify_one();
            version_task.abort();
            acceptance_task.abort();
            let _ = version_task.await;
            let _ = acceptance_task.await;
            drop(hook_guard);
            panic!("acceptance did not reach its first expected-head lock");
        }

        let inverted_lock_order =
            tokio::time::timeout(Duration::from_millis(150), other_first_lock.notified())
                .await
                .is_ok();
        resume.notify_one();
        if inverted_lock_order {
            version_task.abort();
            acceptance_task.abort();
        }

        let version_output = tokio::time::timeout(Duration::from_secs(5), &mut version_task)
            .await
            .ok();
        let acceptance_output = tokio::time::timeout(Duration::from_secs(5), &mut acceptance_task)
            .await
            .ok();
        let handlers_completed = version_output.is_some() && acceptance_output.is_some();
        if !handlers_completed {
            version_task.abort();
            acceptance_task.abort();
            let _ = version_task.await;
            let _ = acceptance_task.await;
        }
        drop(hook_guard);

        assert!(
            !inverted_lock_order,
            "acceptance acquired a different first lock while proposal update held its first lock"
        );
        assert!(handlers_completed, "concurrent handlers deadlocked");
        let version_result = version_output
            .expect("version handler completed")
            .expect("version handler task succeeded")
            .expect("proposal v2 should commit");
        assert!(version_result.accepted);
        let acceptance_result = acceptance_output
            .expect("acceptance handler completed")
            .expect("acceptance handler task succeeded");
        assert!(matches!(
            acceptance_result,
            Err(IngestError::Rejected(message))
                if message.contains("referenced business record changed before the command committed")
        ));
        expect_event_missing(&fixture, &acceptance_event).await;
        let claim_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM business_proposal_conversion_claims \
             WHERE community_id = $1 AND conversion_id = $2",
        )
        .bind(fixture.tenant.community().as_uuid())
        .bind(acceptance.conversion_id)
        .fetch_one(&fixture.pool)
        .await
        .expect("count conversion claims");
        assert_eq!(claim_count, 0, "conflicted acceptance has no claim");
    }

    fn start_party_race(
        fixture: &Fixture,
        keys: &Keys,
        event: Event,
    ) -> (
        tokio::task::JoinHandle<Result<IngestResult, IngestError>>,
        Arc<Notify>,
        Arc<Notify>,
    ) {
        let ready = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        install_party_validation_test_hook(
            event.id.to_bytes(),
            Arc::clone(&ready),
            Arc::clone(&resume),
        );
        let state = Arc::clone(&fixture.state);
        let tenant = fixture.tenant.clone();
        let auth = auth(keys);
        let task = tokio::spawn(async move { handle(&tenant, &state, event, auth).await });
        (task, ready, resume)
    }

    async fn archive_party(
        fixture: &Fixture,
        keys: &Keys,
        business_channel_id: Uuid,
        party_id: Uuid,
    ) -> StoredEvent {
        let d_tag = business_d_tag(*fixture.tenant.community().as_uuid(), "party", party_id);
        let head = current_head::<PartyHead>(
            &fixture.state,
            fixture.tenant.community(),
            KIND_PARTY_HEAD,
            &d_tag,
        )
        .await
        .expect("load party before archive")
        .expect("party exists before archive");
        let content: PartyHead = parse_content(&head.event).expect("party content");
        let event = signed_command(
            keys,
            KIND_PARTY_ACTION,
            business_channel_id,
            &d_tag,
            &party_action(
                party_id,
                RecordAction::Archive,
                Some(head.event.id.to_hex()),
                &content.party.display_name,
            ),
        );
        handle(&fixture.tenant, &fixture.state, event, auth(keys))
            .await
            .expect("archive Party during link race");
        current_head::<PartyHead>(
            &fixture.state,
            fixture.tenant.community(),
            KIND_PARTY_HEAD,
            &d_tag,
        )
        .await
        .expect("load archived party")
        .expect("archived party head exists")
    }

    async fn wait_for_party_read(
        ready: &Notify,
        handler: &mut tokio::task::JoinHandle<Result<IngestResult, IngestError>>,
    ) {
        tokio::select! {
            result = tokio::time::timeout(Duration::from_secs(10), ready.notified()) => {
                result.expect("handler reached the active Party read before persistence");
            }
            result = handler => {
                match result {
                    Ok(Ok(_)) => panic!("handler accepted before the active Party read"),
                    Ok(Err(IngestError::Rejected(error))) => {
                        panic!("handler rejected before the active Party read: {error}");
                    }
                    Ok(Err(IngestError::AuthFailed(error))) => {
                        panic!("handler denied before the active Party read: {error}");
                    }
                    Ok(Err(IngestError::Internal(error))) => {
                        panic!("handler failed before the active Party read: {error}");
                    }
                    Err(error) => panic!("handler task failed before the active Party read: {error}"),
                }
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn party_archive_between_validation_and_commit_conflicts_for_link_proposal_and_conversion(
    ) {
        let fixture = fixture().await;
        let owner = Keys::generate();
        let business_channel_id = business_stream(&fixture, &owner).await;
        let client_channel_id = private_stream(&fixture, "race-client", &owner).await;

        let client_party_id = Uuid::new_v4();
        create_party(&fixture, &owner, business_channel_id, client_party_id).await;
        let client_d = client_d_tag(client_channel_id, "client", client_channel_id);
        let client_event = signed_command(
            &owner,
            KIND_CLIENT_ACTION,
            client_channel_id,
            &client_d,
            &client_action(
                client_channel_id,
                client_party_id,
                RecordAction::Create,
                None,
                "Race client",
                &owner,
            ),
        );
        let (mut task, ready, resume) = start_party_race(&fixture, &owner, client_event.clone());
        wait_for_party_read(&ready, &mut task).await;
        archive_party(&fixture, &owner, business_channel_id, client_party_id).await;
        resume.notify_one();
        let link_result = task.await.expect("join client link");
        assert!(
            matches!(link_result, Err(IngestError::Rejected(message)) if message.contains("referenced business record changed before the command committed"))
        );
        expect_event_missing(&fixture, &client_event).await;

        let proposal_party_id = Uuid::new_v4();
        create_party(&fixture, &owner, business_channel_id, proposal_party_id).await;
        let proposal_id = Uuid::new_v4();
        let proposal_d =
            proposal_version_d_tag(*fixture.tenant.community().as_uuid(), proposal_id, 1);
        let proposal_event = signed_command(
            &owner,
            KIND_PROPOSAL_VERSION,
            business_channel_id,
            &proposal_d,
            &proposal_version(proposal_id, proposal_party_id, &owner, 1, None),
        );
        let (mut task, ready, resume) = start_party_race(&fixture, &owner, proposal_event.clone());
        wait_for_party_read(&ready, &mut task).await;
        archive_party(&fixture, &owner, business_channel_id, proposal_party_id).await;
        resume.notify_one();
        let proposal_result = task.await.expect("join proposal create");
        assert!(
            matches!(proposal_result, Err(IngestError::Rejected(message)) if message.contains("referenced business record changed before the command committed"))
        );
        expect_event_missing(&fixture, &proposal_event).await;

        let conversion_party_id = Uuid::new_v4();
        create_party(&fixture, &owner, business_channel_id, conversion_party_id).await;
        let conversion_proposal_id = Uuid::new_v4();
        let conversion_proposal_d = proposal_version_d_tag(
            *fixture.tenant.community().as_uuid(),
            conversion_proposal_id,
            1,
        );
        let conversion_proposal = signed_command(
            &owner,
            KIND_PROPOSAL_VERSION,
            business_channel_id,
            &conversion_proposal_d,
            &proposal_version(conversion_proposal_id, conversion_party_id, &owner, 1, None),
        );
        handle(
            &fixture.tenant,
            &fixture.state,
            conversion_proposal.clone(),
            auth(&owner),
        )
        .await
        .expect("create proposal for conversion race");
        let conversion = ProposalAcceptance {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            proposal_id: conversion_proposal_id,
            proposal_version_event_id: conversion_proposal.id.to_hex(),
            proposal_version_digest: digest_hex(conversion_proposal.content.as_bytes()),
            conversion_id: Uuid::new_v4(),
            client_id: Uuid::new_v4(),
            work_item_id: Uuid::new_v4(),
            draft_invoice_id: Uuid::new_v4(),
            evidence: Some(test_acceptance_evidence()),
        };
        let acceptance_d = business_d_tag(
            *fixture.tenant.community().as_uuid(),
            "conversion",
            conversion.conversion_id,
        );
        let acceptance_event = signed_command(
            &owner,
            KIND_PROPOSAL_ACCEPTANCE,
            business_channel_id,
            &acceptance_d,
            &conversion,
        );
        let (mut task, ready, resume) =
            start_party_race(&fixture, &owner, acceptance_event.clone());
        wait_for_party_read(&ready, &mut task).await;
        archive_party(&fixture, &owner, business_channel_id, conversion_party_id).await;
        resume.notify_one();
        let conversion_result = task.await.expect("join proposal conversion");
        assert!(
            matches!(conversion_result, Err(IngestError::Rejected(message)) if message.contains("referenced business record changed before the command committed"))
        );
        expect_event_missing(&fixture, &acceptance_event).await;
        let claim_count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM business_proposal_conversion_claims \
             WHERE community_id = $1 AND conversion_id = $2",
        )
        .bind(fixture.tenant.community().as_uuid())
        .bind(conversion.conversion_id)
        .fetch_one(&fixture.pool)
        .await
        .expect("count raced conversion claim");
        assert_eq!(claim_count, 0);
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn money_writes_require_community_owner_or_admin_and_matching_client_scope() {
        let fixture = fixture().await;
        let owner = Keys::generate();
        business_stream(&fixture, &owner).await;
        let owner_client = private_stream(&fixture, "money-owner-client", &owner).await;
        let owner_invoice = Uuid::new_v4();
        seed_draft_invoice(
            &fixture,
            &owner,
            owner_client,
            owner_invoice,
            10_000,
            Some(1_800_000_000),
        )
        .await;
        let (owner_head_event, owner_head) =
            issue_test_invoice(&fixture, &owner, owner_client, owner_invoice).await;
        assert_eq!(owner_head.status, InvoiceStatus::Issued);
        assert_eq!(owner_head.outstanding_minor, 10_000);

        let admin = Keys::generate();
        let admin_client = private_stream(&fixture, "money-admin-client", &admin).await;
        add_community_role(&fixture, &admin, "admin").await;
        let admin_invoice = Uuid::new_v4();
        seed_draft_invoice(
            &fixture,
            &admin,
            admin_client,
            admin_invoice,
            2_500,
            Some(1_800_000_000),
        )
        .await;
        let (_, admin_head) =
            issue_test_invoice(&fixture, &admin, admin_client, admin_invoice).await;
        assert_eq!(admin_head.status, InvoiceStatus::Issued);

        let member = Keys::generate();
        let member_client = private_stream(&fixture, "money-member-client", &member).await;
        add_community_role(&fixture, &member, "member").await;
        let member_invoice = Uuid::new_v4();
        seed_draft_invoice(
            &fixture,
            &member,
            member_client,
            member_invoice,
            1_200,
            Some(1_800_000_000),
        )
        .await;
        let (member_head_event, member_head) = current_invoice_head(
            &fixture.state,
            fixture.tenant.community(),
            member_client,
            member_client,
            member_invoice,
        )
        .await
        .expect("load member invoice");
        let member_issue = InvoiceVersion {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id: member_client,
            invoice_id: member_invoice,
            version: member_head.version + 1,
            previous_version_event_id: Some(member_head.current_version_event_id.clone()),
            proposal_version_event_id: Some(member_head.proposal_version_event_id.clone()),
            expected_head_event_id: Some(member_head_event.event.id.to_hex()),
            action: InvoiceVersionAction::Issue,
            currency: member_head.currency.clone(),
            lines: member_head.lines.clone(),
            tax_lines: member_head.tax_lines.clone(),
            seller_tax_number: member_head.seller_tax_number.clone(),
            customer_tax_number: member_head.customer_tax_number.clone(),
            total_minor: member_head.total_minor,
            status: InvoiceStatus::Issued,
            due_at: member_head.due_at,
            void_reason: None,
        };
        let member_issue_d =
            invoice_version_d_tag(member_client, member_invoice, member_issue.version);
        let member_issue_event = signed_command(
            &member,
            KIND_INVOICE_VERSION,
            member_client,
            &member_issue_d,
            &member_issue,
        );
        let member_result = handle(
            &fixture.tenant,
            &fixture.state,
            member_issue_event.clone(),
            auth(&member),
        )
        .await;
        assert!(
            matches!(member_result, Err(IngestError::AuthFailed(message)) if message.contains("community owner or admin role"))
        );
        expect_event_missing(&fixture, &member_issue_event).await;

        let other_client = private_stream(&fixture, "money-other-client", &owner).await;
        let forged_payment = payment_evidence(
            owner_client,
            owner_invoice,
            &owner_head_event.event.id.to_hex(),
            1_000,
            "ZAR",
        );
        let forged_payment_d = payment_d_tag(owner_client, forged_payment.payment_id);
        let forged_payment_event = signed_command(
            &owner,
            KIND_PAYMENT,
            other_client,
            &forged_payment_d,
            &forged_payment,
        );
        let cross_client_result = handle(
            &fixture.tenant,
            &fixture.state,
            forged_payment_event.clone(),
            auth(&owner),
        )
        .await;
        assert!(
            matches!(cross_client_result, Err(IngestError::Rejected(message)) if message.contains("client id does not match channel scope"))
        );
        expect_event_missing(&fixture, &forged_payment_event).await;
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn invoice_tax_is_per_line_validated_and_preserved_when_issued() {
        let fixture = fixture().await;
        let owner = Keys::generate();
        business_stream(&fixture, &owner).await;
        let client_id = private_stream(&fixture, "money-tax-client", &owner).await;
        let invoice_id = Uuid::new_v4();
        seed_draft_invoice(
            &fixture,
            &owner,
            client_id,
            invoice_id,
            10_000,
            Some(1_800_000_000),
        )
        .await;
        let (head_event, draft) = current_invoice_head(
            &fixture.state,
            fixture.tenant.community(),
            client_id,
            client_id,
            invoice_id,
        )
        .await
        .expect("load draft invoice");
        let lines = vec![
            ProposalLine {
                service_id: None,
                description: "First minor-unit line".into(),
                quantity_hundredths: 100,
                unit_amount_minor: 1,
            },
            ProposalLine {
                service_id: None,
                description: "Second minor-unit line".into(),
                quantity_hundredths: 100,
                unit_amount_minor: 1,
            },
        ];
        let tax_lines = vec![InvoiceTaxLine {
            label: Some("Illustrative tax".into()),
            rate_basis_points: 5_000,
        }];
        let mut edit = InvoiceVersion {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            version: draft.version + 1,
            previous_version_event_id: Some(draft.current_version_event_id.clone()),
            proposal_version_event_id: Some(draft.proposal_version_event_id.clone()),
            expected_head_event_id: Some(head_event.event.id.to_hex()),
            action: InvoiceVersionAction::DraftEdit,
            currency: draft.currency.clone(),
            lines,
            tax_lines,
            seller_tax_number: Some("SELLER-TEST-REG".into()),
            customer_tax_number: Some("CUSTOMER-TEST-REG".into()),
            total_minor: 3,
            status: InvoiceStatus::Draft,
            due_at: draft.due_at,
            void_reason: None,
        };
        let edit_d = invoice_version_d_tag(client_id, invoice_id, edit.version);
        let invalid_event = signed_command(&owner, KIND_INVOICE_VERSION, client_id, &edit_d, &edit);
        let invalid_result = handle(
            &fixture.tenant,
            &fixture.state,
            invalid_event.clone(),
            auth(&owner),
        )
        .await;
        assert!(
            matches!(invalid_result, Err(IngestError::Rejected(message)) if message.contains("line items and tax"))
        );
        expect_event_missing(&fixture, &invalid_event).await;

        edit.total_minor = 4;
        let edit_event = signed_command(&owner, KIND_INVOICE_VERSION, client_id, &edit_d, &edit);
        let edit_result = handle(&fixture.tenant, &fixture.state, edit_event, auth(&owner))
            .await
            .expect("save optional tax on the draft invoice");
        assert!(edit_result.accepted);
        let (_, taxed_draft) = current_invoice_head(
            &fixture.state,
            fixture.tenant.community(),
            client_id,
            client_id,
            invoice_id,
        )
        .await
        .expect("load tax draft");
        assert_eq!(taxed_draft.total_minor, 4);
        assert_eq!(taxed_draft.tax_lines, edit.tax_lines);
        assert_eq!(
            taxed_draft.seller_tax_number.as_deref(),
            Some("SELLER-TEST-REG")
        );
        assert_eq!(
            taxed_draft.customer_tax_number.as_deref(),
            Some("CUSTOMER-TEST-REG")
        );

        let (_, issued) = issue_test_invoice(&fixture, &owner, client_id, invoice_id).await;
        assert_eq!(issued.status, InvoiceStatus::Issued);
        assert_eq!(issued.total_minor, 4);
        assert_eq!(issued.outstanding_minor, 4);
        assert_eq!(issued.tax_lines, edit.tax_lines);
        assert_eq!(issued.seller_tax_number.as_deref(), Some("SELLER-TEST-REG"));
        assert_eq!(
            issued.customer_tax_number.as_deref(),
            Some("CUSTOMER-TEST-REG")
        );
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn payment_evidence_rejects_currency_and_overpayment_and_updates_partial_and_full_totals()
    {
        let fixture = fixture().await;
        let owner = Keys::generate();
        business_stream(&fixture, &owner).await;
        let client_id = private_stream(&fixture, "money-payment-client", &owner).await;
        let invoice_id = Uuid::new_v4();
        seed_draft_invoice(
            &fixture,
            &owner,
            client_id,
            invoice_id,
            10_000,
            Some(1_800_000_000),
        )
        .await;
        let (head_event, issued) =
            issue_test_invoice(&fixture, &owner, client_id, invoice_id).await;
        assert_eq!(issued.outstanding_minor, 10_000);

        for (amount, currency, expected_message) in [
            (10_001, "ZAR", "payment amount exceeds"),
            (100, "USD", "payment currency must match"),
        ] {
            let payment = payment_evidence(
                client_id,
                invoice_id,
                &head_event.event.id.to_hex(),
                amount,
                currency,
            );
            let payment_d = payment_d_tag(client_id, payment.payment_id);
            let payment_event =
                signed_command(&owner, KIND_PAYMENT, client_id, &payment_d, &payment);
            let result = handle(
                &fixture.tenant,
                &fixture.state,
                payment_event.clone(),
                auth(&owner),
            )
            .await;
            match result {
                Err(IngestError::Rejected(message)) => assert!(
                    message.contains(expected_message),
                    "unexpected payment rejection: {message}"
                ),
                Err(_) => panic!("invalid payment evidence returned a non-rejection error"),
                Ok(_) => panic!("invalid payment evidence was accepted"),
            }
            expect_event_missing(&fixture, &payment_event).await;
        }

        let partial = payment_evidence(
            client_id,
            invoice_id,
            &head_event.event.id.to_hex(),
            4_000,
            "ZAR",
        );
        let partial_d = payment_d_tag(client_id, partial.payment_id);
        let partial_event = signed_command(&owner, KIND_PAYMENT, client_id, &partial_d, &partial);
        handle(&fixture.tenant, &fixture.state, partial_event, auth(&owner))
            .await
            .expect("record partial payment evidence");
        let (partial_head_event, partial_head) = current_invoice_head(
            &fixture.state,
            fixture.tenant.community(),
            client_id,
            client_id,
            invoice_id,
        )
        .await
        .expect("load partial payment totals");
        assert_eq!(partial_head.collected_minor, 4_000);
        assert_eq!(partial_head.outstanding_minor, 6_000);
        assert_eq!(partial_head.payment_evidence_count, 1);

        let full = payment_evidence(
            client_id,
            invoice_id,
            &partial_head_event.event.id.to_hex(),
            6_000,
            "ZAR",
        );
        let full_d = payment_d_tag(client_id, full.payment_id);
        let full_event = signed_command(&owner, KIND_PAYMENT, client_id, &full_d, &full);
        handle(&fixture.tenant, &fixture.state, full_event, auth(&owner))
            .await
            .expect("record final payment evidence");
        let (_, fully_paid) = current_invoice_head(
            &fixture.state,
            fixture.tenant.community(),
            client_id,
            client_id,
            invoice_id,
        )
        .await
        .expect("load fully paid invoice totals");
        assert_eq!(fully_paid.collected_minor, 10_000);
        assert_eq!(fully_paid.outstanding_minor, 0);
        assert_eq!(fully_paid.payment_evidence_count, 2);
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn adjustments_update_balances_and_void_after_any_payment_is_rejected() {
        let fixture = fixture().await;
        let owner = Keys::generate();
        business_stream(&fixture, &owner).await;
        let client_id = private_stream(&fixture, "money-adjustment-client", &owner).await;
        let invoice_id = Uuid::new_v4();
        seed_draft_invoice(
            &fixture,
            &owner,
            client_id,
            invoice_id,
            10_000,
            Some(1_800_000_000),
        )
        .await;
        issue_test_invoice(&fixture, &owner, client_id, invoice_id).await;

        let (head_event, _head) = current_invoice_head(
            &fixture.state,
            fixture.tenant.community(),
            client_id,
            client_id,
            invoice_id,
        )
        .await
        .expect("load issued invoice");
        let payment = payment_evidence(
            client_id,
            invoice_id,
            &head_event.event.id.to_hex(),
            4_000,
            "ZAR",
        );
        let payment_d = payment_d_tag(client_id, payment.payment_id);
        let payment_event = signed_command(&owner, KIND_PAYMENT, client_id, &payment_d, &payment);
        handle(&fixture.tenant, &fixture.state, payment_event, auth(&owner))
            .await
            .expect("record partial payment before credit note");

        let (head_event, head) = current_invoice_head(
            &fixture.state,
            fixture.tenant.community(),
            client_id,
            client_id,
            invoice_id,
        )
        .await
        .expect("load invoice before credit note");
        let credit = money_adjustment(
            client_id,
            invoice_id,
            &head_event.event.id.to_hex(),
            MoneyAdjustmentType::CreditNote,
            7_000,
        );
        let credit_d = money_adjustment_d_tag(client_id, credit.adjustment_id);
        let credit_event =
            signed_command(&owner, KIND_MONEY_ADJUSTMENT, client_id, &credit_d, &credit);
        handle(&fixture.tenant, &fixture.state, credit_event, auth(&owner))
            .await
            .expect("record credit note");
        let (head_event, credited) = current_invoice_head(
            &fixture.state,
            fixture.tenant.community(),
            client_id,
            client_id,
            invoice_id,
        )
        .await
        .expect("load credited invoice");
        assert_eq!(credited.credited_minor, 7_000);
        assert_eq!(credited.collected_minor, 4_000);
        assert_eq!(credited.outstanding_minor, 0);

        let refund = money_adjustment(
            client_id,
            invoice_id,
            &head_event.event.id.to_hex(),
            MoneyAdjustmentType::Refund,
            1_000,
        );
        let refund_d = money_adjustment_d_tag(client_id, refund.adjustment_id);
        let refund_event =
            signed_command(&owner, KIND_MONEY_ADJUSTMENT, client_id, &refund_d, &refund);
        handle(&fixture.tenant, &fixture.state, refund_event, auth(&owner))
            .await
            .expect("record external refund evidence");
        let (head_event, refunded) = current_invoice_head(
            &fixture.state,
            fixture.tenant.community(),
            client_id,
            client_id,
            invoice_id,
        )
        .await
        .expect("load refunded invoice");
        assert_eq!(refunded.collected_minor, 3_000);
        assert_eq!(refunded.outstanding_minor, 0);
        assert_eq!(refunded.payment_evidence_count, 1);

        let void = InvoiceVersion {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            version: refunded.version + 1,
            previous_version_event_id: Some(refunded.current_version_event_id.clone()),
            proposal_version_event_id: Some(refunded.proposal_version_event_id.clone()),
            expected_head_event_id: Some(head_event.event.id.to_hex()),
            action: InvoiceVersionAction::Void,
            currency: refunded.currency.clone(),
            lines: refunded.lines.clone(),
            tax_lines: refunded.tax_lines.clone(),
            seller_tax_number: refunded.seller_tax_number.clone(),
            customer_tax_number: refunded.customer_tax_number.clone(),
            total_minor: refunded.total_minor,
            status: InvoiceStatus::Void,
            due_at: refunded.due_at,
            void_reason: Some("Client paid portion already recorded".into()),
        };
        let void_d = invoice_version_d_tag(client_id, invoice_id, void.version);
        let void_event = signed_command(&owner, KIND_INVOICE_VERSION, client_id, &void_d, &void);
        let void_result = handle(
            &fixture.tenant,
            &fixture.state,
            void_event.clone(),
            auth(&owner),
        )
        .await;
        assert!(
            matches!(void_result, Err(IngestError::Rejected(message)) if message.contains("payment evidence cannot be voided"))
        );
        expect_event_missing(&fixture, &void_event).await;

        let writeoff_invoice = Uuid::new_v4();
        seed_draft_invoice(
            &fixture,
            &owner,
            client_id,
            writeoff_invoice,
            10_000,
            Some(1_800_000_000),
        )
        .await;
        issue_test_invoice(&fixture, &owner, client_id, writeoff_invoice).await;
        let (writeoff_head_event, _writeoff_head) = current_invoice_head(
            &fixture.state,
            fixture.tenant.community(),
            client_id,
            client_id,
            writeoff_invoice,
        )
        .await
        .expect("load write-off invoice");
        let writeoff = money_adjustment(
            client_id,
            writeoff_invoice,
            &writeoff_head_event.event.id.to_hex(),
            MoneyAdjustmentType::WriteOff,
            2_500,
        );
        let writeoff_d = money_adjustment_d_tag(client_id, writeoff.adjustment_id);
        let writeoff_event = signed_command(
            &owner,
            KIND_MONEY_ADJUSTMENT,
            client_id,
            &writeoff_d,
            &writeoff,
        );
        handle(
            &fixture.tenant,
            &fixture.state,
            writeoff_event,
            auth(&owner),
        )
        .await
        .expect("record write-off evidence");
        let (_, written_off) = current_invoice_head(
            &fixture.state,
            fixture.tenant.community(),
            client_id,
            client_id,
            writeoff_invoice,
        )
        .await
        .expect("load written-off invoice");
        assert_eq!(written_off.written_off_minor, 2_500);
        assert_eq!(written_off.outstanding_minor, 7_500);
        assert_eq!(head.status, InvoiceStatus::Issued);
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn follow_up_draft_review_and_approval_record_intent_only() {
        let fixture = fixture().await;
        let owner = Keys::generate();
        business_stream(&fixture, &owner).await;
        let client_id = private_stream(&fixture, "money-follow-up-client", &owner).await;
        let invoice_id = Uuid::new_v4();
        let invoice_due_at = chrono::Utc::now().timestamp() - 3_600;
        seed_draft_invoice(
            &fixture,
            &owner,
            client_id,
            invoice_id,
            10_000,
            Some(invoice_due_at),
        )
        .await;
        let (invoice_event, invoice) =
            issue_test_invoice(&fixture, &owner, client_id, invoice_id).await;
        let follow_up_id = Uuid::new_v4();
        let draft = MoneyFollowUpAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            follow_up_id,
            action: MoneyFollowUpActionKind::Draft,
            expected_head_event_id: None,
            expected_invoice_head_event_id: invoice_event.event.id.to_hex(),
            due_at: Some(chrono::Utc::now().timestamp() + 86_400),
            draft_content: "Please review the outstanding project invoice.".into(),
        };
        let follow_up_d = money_follow_up_d_tag(client_id, follow_up_id);
        let draft_event = signed_command(
            &owner,
            KIND_MONEY_FOLLOW_UP,
            client_id,
            &follow_up_d,
            &draft,
        );
        handle(&fixture.tenant, &fixture.state, draft_event, auth(&owner))
            .await
            .expect("create follow-up draft");
        let (follow_head_event, follow_head) = current_head::<MoneyFollowUpHead>(
            &fixture.state,
            fixture.tenant.community(),
            KIND_MONEY_FOLLOW_UP_HEAD,
            &follow_up_d,
        )
        .await
        .expect("load follow-up draft")
        .map(|event| {
            let head: MoneyFollowUpHead =
                parse_content(&event.event).expect("parse follow-up draft head");
            (event, head)
        })
        .expect("follow-up draft exists");
        assert_eq!(follow_head.status, MoneyFollowUpStatus::Draft);
        assert!(!follow_head.approval_intent_only);

        let review = MoneyFollowUpAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            follow_up_id,
            action: MoneyFollowUpActionKind::Review,
            expected_head_event_id: Some(follow_head_event.event.id.to_hex()),
            expected_invoice_head_event_id: invoice_event.event.id.to_hex(),
            due_at: follow_head.due_at,
            draft_content: follow_head.draft_content.clone(),
        };
        let review_event = signed_command(
            &owner,
            KIND_MONEY_FOLLOW_UP,
            client_id,
            &follow_up_d,
            &review,
        );
        handle(&fixture.tenant, &fixture.state, review_event, auth(&owner))
            .await
            .expect("move follow-up into review");
        let (follow_head_event, follow_head) = current_head::<MoneyFollowUpHead>(
            &fixture.state,
            fixture.tenant.community(),
            KIND_MONEY_FOLLOW_UP_HEAD,
            &follow_up_d,
        )
        .await
        .expect("load follow-up under review")
        .map(|event| {
            let head: MoneyFollowUpHead =
                parse_content(&event.event).expect("parse follow-up review head");
            (event, head)
        })
        .expect("review head exists");
        assert_eq!(follow_head.status, MoneyFollowUpStatus::InReview);

        let approval = MoneyFollowUpAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            follow_up_id,
            action: MoneyFollowUpActionKind::Approve,
            expected_head_event_id: Some(follow_head_event.event.id.to_hex()),
            expected_invoice_head_event_id: invoice_event.event.id.to_hex(),
            due_at: follow_head.due_at,
            draft_content: follow_head.draft_content,
        };
        let approval_event = signed_command(
            &owner,
            KIND_MONEY_FOLLOW_UP,
            client_id,
            &follow_up_d,
            &approval,
        );
        handle(
            &fixture.tenant,
            &fixture.state,
            approval_event,
            auth(&owner),
        )
        .await
        .expect("approve follow-up intent");
        let (_, approved) = current_head::<MoneyFollowUpHead>(
            &fixture.state,
            fixture.tenant.community(),
            KIND_MONEY_FOLLOW_UP_HEAD,
            &follow_up_d,
        )
        .await
        .expect("load approved follow-up")
        .map(|event| {
            let head: MoneyFollowUpHead =
                parse_content(&event.event).expect("parse approved follow-up head");
            (event, head)
        })
        .expect("approved head exists");
        assert_eq!(approved.status, MoneyFollowUpStatus::Approved);
        assert!(approved.approval_intent_only);
        assert_eq!(
            approved.approved_by_pubkey,
            Some(owner.public_key().to_hex())
        );
        assert_eq!(invoice.outstanding_minor, 10_000);
    }

    #[tokio::test]
    #[ignore = "requires disposable Postgres and Redis"]
    async fn concurrent_payments_with_one_invoice_head_commit_only_one() {
        let fixture = fixture().await;
        let owner = Keys::generate();
        business_stream(&fixture, &owner).await;
        let client_id = private_stream(&fixture, "money-race-client", &owner).await;
        let invoice_id = Uuid::new_v4();
        seed_draft_invoice(
            &fixture,
            &owner,
            client_id,
            invoice_id,
            10_000,
            Some(1_800_000_000),
        )
        .await;
        let (head_event, _) = issue_test_invoice(&fixture, &owner, client_id, invoice_id).await;
        let expected_head = head_event.event.id.to_hex();
        let first = payment_evidence(client_id, invoice_id, &expected_head, 7_000, "ZAR");
        let second = payment_evidence(client_id, invoice_id, &expected_head, 7_000, "ZAR");
        let first_d = payment_d_tag(client_id, first.payment_id);
        let second_d = payment_d_tag(client_id, second.payment_id);
        let first_event = signed_command(&owner, KIND_PAYMENT, client_id, &first_d, &first);
        let second_event = signed_command(&owner, KIND_PAYMENT, client_id, &second_d, &second);

        let paused = Arc::new(Notify::new());
        let resume = Arc::new(Notify::new());
        let other_lock_attempt = Arc::new(Notify::new());
        let other_first_lock = Arc::new(Notify::new());
        install_expected_head_lock_test_hook(
            first_event.id.to_bytes(),
            Arc::clone(&paused),
            Arc::clone(&resume),
            Arc::clone(&other_lock_attempt),
            Arc::clone(&other_first_lock),
        );
        let hook_guard = ExpectedHeadLockTestHookGuard {
            resume: Arc::clone(&resume),
        };
        let first_tenant = fixture.tenant.clone();
        let first_state = Arc::clone(&fixture.state);
        let first_actor = owner.clone();
        let mut first_task = tokio::spawn(async move {
            handle(&first_tenant, &first_state, first_event, auth(&first_actor)).await
        });
        if tokio::time::timeout(Duration::from_secs(5), paused.notified())
            .await
            .is_err()
        {
            resume.notify_one();
            first_task.abort();
            let _ = first_task.await;
            drop(hook_guard);
            panic!("first payment did not acquire the invoice head lock");
        }

        let second_tenant = fixture.tenant.clone();
        let second_state = Arc::clone(&fixture.state);
        let second_actor = owner.clone();
        let mut second_task = tokio::spawn(async move {
            handle(
                &second_tenant,
                &second_state,
                second_event,
                auth(&second_actor),
            )
            .await
        });
        if tokio::time::timeout(Duration::from_secs(5), other_lock_attempt.notified())
            .await
            .is_err()
        {
            resume.notify_one();
            first_task.abort();
            second_task.abort();
            let _ = first_task.await;
            let _ = second_task.await;
            drop(hook_guard);
            panic!("second payment did not reach the invoice head lock");
        }
        let second_acquired_stale_lock =
            tokio::time::timeout(Duration::from_millis(150), other_first_lock.notified())
                .await
                .is_ok();
        resume.notify_one();

        let first_result = tokio::time::timeout(Duration::from_secs(5), &mut first_task)
            .await
            .ok();
        let second_result = tokio::time::timeout(Duration::from_secs(5), &mut second_task)
            .await
            .ok();
        if first_result.is_none() || second_result.is_none() {
            first_task.abort();
            second_task.abort();
            let _ = first_task.await;
            let _ = second_task.await;
        }
        drop(hook_guard);

        assert!(
            !second_acquired_stale_lock,
            "second payment acquired the invoice lock before the first commit"
        );
        let first_result = first_result
            .expect("first payment handler completed")
            .expect("first payment handler task succeeded")
            .expect("first payment commits");
        assert!(first_result.accepted);
        let second_result = second_result
            .expect("second payment handler completed")
            .expect("second payment handler task succeeded");
        assert!(matches!(
            second_result,
            Err(IngestError::Rejected(message))
                if message.contains("referenced business record changed before the command committed")
        ));
        let (_, final_head) = current_invoice_head(
            &fixture.state,
            fixture.tenant.community(),
            client_id,
            client_id,
            invoice_id,
        )
        .await
        .expect("load final raced invoice head");
        assert_eq!(final_head.collected_minor, 7_000);
        assert_eq!(final_head.outstanding_minor, 3_000);
        assert_eq!(final_head.payment_evidence_count, 1);
    }
}
