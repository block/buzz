//! Typed builders for Colony Phase 2 business-record commands.
//!
//! Kind constants are re-exported through [`crate::kind`], which is the same
//! authoritative registry used by `buzz-core` and the relay.

use buzz_core::business_records::{
    business_d_tag, client_d_tag, company_work_d_tag, deliverable_approval_d_tag,
    deliverable_version_d_tag, invoice_version_d_tag, money_adjustment_d_tag,
    money_follow_up_d_tag, payment_d_tag, proposal_version_d_tag, prospect_d_tag,
    validate_company_work_item_action,
};
pub use buzz_core::business_records::{
    ApprovalDecision, ClientAction, ClientHead, ClientHeadInput, CompanyWorkItemAction,
    CompanyWorkItemActionKind, CompanyWorkItemHead, CompanyWorkItemInput, CompanyWorkStatus,
    CompanyWorkVerdict, CompanyWorkVerification, CompanyWorkVerificationInput, DeliverableApproval,
    DeliverablePointer, DeliverableVersion, DraftInvoiceHead, InvoiceHead, InvoiceStatus,
    InvoiceVersion, InvoiceVersionAction, MoneyAdjustment, MoneyAdjustmentType,
    MoneyFollowUpAction, MoneyFollowUpActionKind, MoneyFollowUpHead, MoneyFollowUpStatus,
    PartyAction, PartyRecord, PaymentEvidence, ProposalAcceptance, ProposalConversionReceipt,
    ProposalHead, ProposalLine, ProposalVersion, ProspectAction, ProspectActivity,
    ProspectActivityInput, ProspectActivityKind, ProspectEvidence, ProspectHead,
    ProspectQualification, ProspectRecordInput, ProspectStage, RecordAction, WorkItemAction,
    WorkItemHead, WorkItemHeadInput,
};
use buzz_core::kind::*;
use nostr::{EventBuilder, Kind, Tag};
use serde::Serialize;
use uuid::Uuid;

use crate::SdkError;

fn build<T: Serialize>(
    kind: u32,
    channel_id: Uuid,
    d_tag: String,
    content: &T,
) -> Result<EventBuilder, SdkError> {
    let content = serde_json::to_string(content).map_err(|error| {
        SdkError::InvalidInput(format!("business record serialization failed: {error}"))
    })?;
    let tags = vec![
        Tag::parse(["h", channel_id.to_string().as_str()])
            .map_err(|error| SdkError::InvalidTag(error.to_string()))?,
        Tag::parse(["d", d_tag.as_str()])
            .map_err(|error| SdkError::InvalidTag(error.to_string()))?,
    ];
    Ok(EventBuilder::new(Kind::Custom(kind as u16), content).tags(tags))
}

/// Build a member-signed party action in the internal business channel.
pub fn build_party_action(
    community_id: Uuid,
    business_channel_id: Uuid,
    action: &PartyAction,
) -> Result<EventBuilder, SdkError> {
    build(
        KIND_PARTY_ACTION,
        business_channel_id,
        business_d_tag(community_id, "party", action.party_id),
        action,
    )
}

/// Build a member-signed prospect (lead) action in the internal business channel.
pub fn build_prospect_action(
    community_id: Uuid,
    business_channel_id: Uuid,
    action: &ProspectAction,
) -> Result<EventBuilder, SdkError> {
    build(
        KIND_PROSPECT_ACTION,
        business_channel_id,
        prospect_d_tag(community_id, action.prospect_id),
        action,
    )
}

/// Build a member-signed client action in the matching client channel.
pub fn build_client_action(action: &ClientAction) -> Result<EventBuilder, SdkError> {
    build(
        KIND_CLIENT_ACTION,
        action.client_id,
        client_d_tag(action.client_id, "client", action.client_id),
        action,
    )
}

/// Build a member-signed work-item action in its client channel.
pub fn build_work_item_action(action: &WorkItemAction) -> Result<EventBuilder, SdkError> {
    build(
        KIND_WORK_ITEM_ACTION,
        action.client_id,
        client_d_tag(action.client_id, "work", action.work_item_id),
        action,
    )
}

/// Build a member-signed company work-item action in its conversation channel.
pub fn build_company_work_item_action(
    channel_id: Uuid,
    action: &CompanyWorkItemAction,
) -> Result<EventBuilder, SdkError> {
    validate_company_work_item_action(action).map_err(|error| {
        SdkError::InvalidInput(format!("company work item action is invalid: {error}"))
    })?;
    build(
        KIND_WORK_ITEM_ACTION,
        channel_id,
        company_work_d_tag(action.work_item_id),
        action,
    )
}

/// Build a member-signed immutable proposal revision in the business channel.
pub fn build_proposal_version(
    community_id: Uuid,
    business_channel_id: Uuid,
    version: &ProposalVersion,
) -> Result<EventBuilder, SdkError> {
    let d_tag = proposal_version_d_tag(community_id, version.proposal_id, version.revision);
    build(KIND_PROPOSAL_VERSION, business_channel_id, d_tag, version)
}

/// Build a member-signed exact-version proposal acceptance and conversion request.
pub fn build_proposal_acceptance(
    community_id: Uuid,
    business_channel_id: Uuid,
    acceptance: &ProposalAcceptance,
) -> Result<EventBuilder, SdkError> {
    build(
        KIND_PROPOSAL_ACCEPTANCE,
        business_channel_id,
        business_d_tag(community_id, "conversion", acceptance.conversion_id),
        acceptance,
    )
}

/// Build a member-signed immutable deliverable version in its client channel.
pub fn build_deliverable_version(version: &DeliverableVersion) -> Result<EventBuilder, SdkError> {
    let d_tag =
        deliverable_version_d_tag(version.client_id, version.deliverable_id, version.version);
    build(KIND_DELIVERABLE_VERSION, version.client_id, d_tag, version)
}

/// Build a member-signed approval decision for an exact deliverable version.
pub fn build_deliverable_approval(
    approval: &DeliverableApproval,
) -> Result<EventBuilder, SdkError> {
    let d_tag = deliverable_approval_d_tag(approval.client_id, &approval.version_event_id);
    build(
        KIND_DELIVERABLE_APPROVAL,
        approval.client_id,
        d_tag,
        approval,
    )
}

/// Build a member-signed invoice revision or lifecycle transition.
pub fn build_invoice_version(version: &InvoiceVersion) -> Result<EventBuilder, SdkError> {
    let d_tag = invoice_version_d_tag(version.client_id, version.invoice_id, version.version);
    build(KIND_INVOICE_VERSION, version.client_id, d_tag, version)
}

/// Build member-signed evidence that an invoice payment already happened.
pub fn build_payment_evidence(payment: &PaymentEvidence) -> Result<EventBuilder, SdkError> {
    build(
        KIND_PAYMENT,
        payment.client_id,
        payment_d_tag(payment.client_id, payment.payment_id),
        payment,
    )
}

/// Build member-signed evidence for a credit note, external refund, or write-off.
pub fn build_money_adjustment(adjustment: &MoneyAdjustment) -> Result<EventBuilder, SdkError> {
    build(
        KIND_MONEY_ADJUSTMENT,
        adjustment.client_id,
        money_adjustment_d_tag(adjustment.client_id, adjustment.adjustment_id),
        adjustment,
    )
}

/// Build a member-signed follow-up draft, review, or intent approval.
pub fn build_money_follow_up(action: &MoneyFollowUpAction) -> Result<EventBuilder, SdkError> {
    build(
        KIND_MONEY_FOLLOW_UP,
        action.client_id,
        money_follow_up_d_tag(action.client_id, action.follow_up_id),
        action,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use buzz_core::business_records::BUSINESS_RECORD_SCHEMA_VERSION;

    #[test]
    fn client_action_builder_binds_h_and_namespaced_d_tags() {
        let id = Uuid::from_u128(1);
        let action = ClientAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id: id,
            action: buzz_core::business_records::RecordAction::Create,
            expected_head_event_id: None,
            head: buzz_core::business_records::ClientHeadInput {
                schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
                client_id: id,
                party_id: Uuid::from_u128(2),
                display_name: "Example Client".into(),
                approver_pubkeys: Vec::new(),
                status: "active".into(),
            },
        };
        let event = build_client_action(&action)
            .expect("builder")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("signed event");
        assert_eq!(event.kind, Kind::Custom(KIND_CLIENT_ACTION as u16));
        assert_eq!(event.tags.len(), 2);
        let tags = event.tags.iter().collect::<Vec<_>>();
        assert_eq!(tags[0].kind().to_string(), "h");
        let expected_channel = id.to_string();
        assert_eq!(tags[0].content(), Some(expected_channel.as_str()));
        assert_eq!(tags[1].kind().to_string(), "d");
        let expected_d_tag = client_d_tag(id, "client", id);
        assert_eq!(tags[1].content(), Some(expected_d_tag.as_str()));
    }

    #[test]
    fn prospect_action_builder_binds_business_channel_and_prospect_coordinate() {
        use buzz_core::business_records::{
            parse_business_command, validate_business_command_scope, BusinessCommand,
            BUSINESS_RECORD_SCHEMA_VERSION,
        };

        let community_id = Uuid::from_u128(1);
        let channel_id = Uuid::from_u128(2);
        let prospect_id = Uuid::from_u128(3);
        let action = ProspectAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            prospect_id,
            action: RecordAction::Update,
            expected_head_event_id: Some("a".repeat(64)),
            prospect: ProspectRecordInput {
                prospect_id,
                party: PartyRecord {
                    party_id: prospect_id,
                    party_type: "organization".into(),
                    display_name: "Form & Field".into(),
                    external_ids: Vec::new(),
                },
                industry: "Home & Living".into(),
                vertical: "Home Decor & Gift Shops".into(),
                fit_score: Some(91),
                origin: Some(buzz_core::business_records::ProspectOrigin::Discovery),
                discovery_tags: Vec::new(),
                potential_monthly_value_minor: None,
                website: None,
                contact_name: None,
                location: None,
                email: None,
                phone: None,
                evidence: Vec::new(),
                last_verified_at: None,
                qualification: ProspectQualification::Qualified,
                saved: true,
                stage: ProspectStage::Qualified,
                lost_reason: None,
            },
            activity: Some(ProspectActivityInput {
                activity_id: Uuid::from_u128(4),
                activity_kind: ProspectActivityKind::Note,
                content: "Spoke to the owner".into(),
                proposal_id: None,
                proposal_version_event_id: None,
            }),
        };
        let event = build_prospect_action(community_id, channel_id, &action)
            .expect("builder")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("signed event");

        assert_eq!(event.kind, Kind::Custom(KIND_PROSPECT_ACTION as u16));
        let tags = event.tags.iter().collect::<Vec<_>>();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].kind().to_string(), "h");
        assert_eq!(tags[0].content(), Some(channel_id.to_string().as_str()));
        assert_eq!(tags[1].kind().to_string(), "d");
        let expected_d_tag = prospect_d_tag(community_id, prospect_id);
        assert_eq!(tags[1].content(), Some(expected_d_tag.as_str()));
        let command = parse_business_command(KIND_PROSPECT_ACTION, &event.content)
            .expect("parse built prospect command");
        assert!(matches!(command, BusinessCommand::ProspectAction(_)));
        assert_eq!(
            validate_business_command_scope(community_id, channel_id, &expected_d_tag, &command),
            Ok(())
        );
    }

    #[test]
    fn money_builders_bind_kind_client_scope_and_record_coordinate() {
        use buzz_core::business_records::{BusinessCommand, BUSINESS_RECORD_SCHEMA_VERSION};

        let client_id = Uuid::from_u128(9);
        let invoice_id = Uuid::from_u128(10);
        let version = InvoiceVersion {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            version: 2,
            previous_version_event_id: Some("a".repeat(64)),
            proposal_version_event_id: Some("b".repeat(64)),
            expected_head_event_id: Some("c".repeat(64)),
            action: InvoiceVersionAction::Issue,
            currency: "ZAR".into(),
            lines: vec![ProposalLine {
                service_id: None,
                description: "Project milestone".into(),
                quantity_hundredths: 100,
                unit_amount_minor: 25_000,
            }],
            tax_lines: Vec::new(),
            seller_tax_number: None,
            customer_tax_number: None,
            total_minor: 25_000,
            status: InvoiceStatus::Issued,
            due_at: Some(1_800_000_000),
            void_reason: None,
        };
        let payment = PaymentEvidence {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            payment_id: Uuid::from_u128(11),
            provider: "manual".into(),
            provider_reference: None,
            amount_minor: 5_000,
            currency: "ZAR".into(),
            occurred_at: 1_800_000_001,
            evidence_ref: "receipt:local-1".into(),
            expected_invoice_head_event_id: "d".repeat(64),
        };
        let adjustment = MoneyAdjustment {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            adjustment_id: Uuid::from_u128(12),
            adjustment_type: MoneyAdjustmentType::CreditNote,
            amount_minor: 1_000,
            currency: "ZAR".into(),
            occurred_at: 1_800_000_001,
            reason: "Scope reduced".into(),
            evidence_ref: "credit-note:1".into(),
            expected_invoice_head_event_id: "e".repeat(64),
        };
        let follow_up = MoneyFollowUpAction {
            schema_version: BUSINESS_RECORD_SCHEMA_VERSION,
            client_id,
            invoice_id,
            follow_up_id: Uuid::from_u128(13),
            action: MoneyFollowUpActionKind::Draft,
            expected_head_event_id: None,
            expected_invoice_head_event_id: "f".repeat(64),
            due_at: None,
            draft_content: "Please review the outstanding invoice.".into(),
        };
        let cases = [
            (
                build_invoice_version(&version).expect("invoice builder"),
                KIND_INVOICE_VERSION,
                invoice_version_d_tag(client_id, invoice_id, 2),
            ),
            (
                build_payment_evidence(&payment).expect("payment builder"),
                KIND_PAYMENT,
                payment_d_tag(client_id, payment.payment_id),
            ),
            (
                build_money_adjustment(&adjustment).expect("adjustment builder"),
                KIND_MONEY_ADJUSTMENT,
                money_adjustment_d_tag(client_id, adjustment.adjustment_id),
            ),
            (
                build_money_follow_up(&follow_up).expect("follow-up builder"),
                KIND_MONEY_FOLLOW_UP,
                money_follow_up_d_tag(client_id, follow_up.follow_up_id),
            ),
        ];

        for (builder, kind, expected_d_tag) in cases {
            let event = builder
                .sign_with_keys(&nostr::Keys::generate())
                .expect("sign event");
            assert_eq!(event.kind, Kind::Custom(kind as u16));
            let tags = event.tags.iter().collect::<Vec<_>>();
            assert_eq!(tags.len(), 2);
            assert_eq!(tags[0].kind().to_string(), "h");
            assert_eq!(tags[0].content(), Some(client_id.to_string().as_str()));
            assert_eq!(tags[1].kind().to_string(), "d");
            assert_eq!(tags[1].content(), Some(expected_d_tag.as_str()));
            let _: BusinessCommand =
                buzz_core::business_records::parse_business_command(kind, &event.content)
                    .expect("parse built money command");
        }
    }

    #[test]
    fn company_work_action_builder_rejects_invalid_payloads() {
        let action = CompanyWorkItemAction {
            schema_version: buzz_core::business_records::BUSINESS_RECORD_SCHEMA_VERSION,
            work_item_id: Uuid::from_u128(8),
            action: CompanyWorkItemActionKind::Create,
            expected_head_event_id: Some("01".repeat(32)),
            head: None,
            status: None,
            reason: None,
            verification: None,
            due_at: None,
        };

        assert!(build_company_work_item_action(Uuid::from_u128(7), &action).is_err());
    }

    #[test]
    fn company_work_action_builder_uses_shared_kind_and_host_scoped_coordinate() {
        let channel_id = Uuid::from_u128(7);
        let work_item_id = Uuid::from_u128(8);
        let action = CompanyWorkItemAction {
            schema_version: buzz_core::business_records::BUSINESS_RECORD_SCHEMA_VERSION,
            work_item_id,
            action: CompanyWorkItemActionKind::Create,
            expected_head_event_id: None,
            head: Some(CompanyWorkItemInput {
                schema_version: buzz_core::business_records::BUSINESS_RECORD_SCHEMA_VERSION,
                work_item_id,
                title: "Prepare the launch checklist".into(),
                status: CompanyWorkStatus::Active,
                assigned_pubkeys: vec!["ab".repeat(32)],
                approver_pubkeys: Vec::new(),
                deliverables: Vec::new(),
                requester_pubkey: "cd".repeat(32),
                done_condition: "Every launch task has an owner".into(),
                goal_id: None,
                source_event_id: None,
                thread_root_event_id: None,
                evidence: None,
                due_at: None,
            }),
            status: None,
            reason: None,
            verification: None,
            due_at: None,
        };
        let event = build_company_work_item_action(channel_id, &action)
            .expect("builder")
            .sign_with_keys(&nostr::Keys::generate())
            .expect("signed event");

        assert_eq!(event.kind, Kind::Custom(KIND_WORK_ITEM_ACTION as u16));
        let tags = event.tags.iter().collect::<Vec<_>>();
        assert_eq!(tags.len(), 2);
        assert_eq!(tags[0].kind().to_string(), "h");
        assert_eq!(tags[0].content(), Some(channel_id.to_string().as_str()));
        assert_eq!(tags[1].kind().to_string(), "d");
        assert_eq!(
            tags[1].content(),
            Some(company_work_d_tag(work_item_id).as_str())
        );
    }
}
