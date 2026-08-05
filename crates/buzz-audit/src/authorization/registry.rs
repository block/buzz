//! Stable closed registries used by authorization evidence.

macro_rules! closed_registry {
    (
        $(#[$meta:meta])*
        $visibility:vis enum $name:ident {
            $($variant:ident = $number:literal => $code:literal,)*
        }
    ) => {
        $(#[$meta])*
        #[allow(missing_docs)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[repr(u16)]
        $visibility enum $name {
            $($variant = $number,)*
        }

        impl $name {
            /// Stable provider-neutral registry code.
            pub const fn code(self) -> &'static str {
                match self {
                    $(Self::$variant => $code,)*
                }
            }

            /// Numeric representation frozen by evidence schema V1.
            pub const fn discriminant(self) -> u16 {
                self as u16
            }

            /// Complete registry in stable order.
            pub const ALL: &'static [Self] = &[$(Self::$variant,)*];
        }
    };
}

closed_registry! {
    /// Semantic event names accepted by evidence schema V1.
    pub enum EventKind {
        AssertionAccepted = 1 => "assertion.accepted",
        AssertionRejected = 2 => "assertion.rejected",
        ProofAccepted = 3 => "proof.accepted",
        ProofRejected = 4 => "proof.rejected",
        AdmissionAllowed = 5 => "admission.allowed",
        AdmissionDenied = 6 => "admission.denied",
        BindingCreated = 7 => "binding.created",
        BindingMatched = 8 => "binding.matched",
        BindingConflict = 9 => "binding.conflict",
        BindingRevoked = 10 => "binding.revoked",
        BindingRotated = 11 => "binding.rotated",
        BindingRecovered = 12 => "binding.recovered",
        BindingArchived = 13 => "binding.archived",
        LeaseIssued = 14 => "lease.issued",
        LeaseRenewed = 15 => "lease.renewed",
        LeaseExpired = 16 => "lease.expired",
        LeaseInvalidated = 17 => "lease.invalidated",
        LeaseRefreshDenied = 18 => "lease.refresh_denied",
        DelegatedAllowed = 19 => "delegated.allowed",
        DelegatedDenied = 20 => "delegated.denied",
        OperatorDenied = 21 => "operator.denied",
        OperatorInspected = 22 => "operator.inspected",
        OperatorListed = 23 => "operator.listed",
        OperatorPreviewed = 24 => "operator.previewed",
        OperatorProvisioned = 25 => "operator.provisioned",
        OperatorRetired = 26 => "operator.retired",
        OperatorPrincipalDisabled = 27 => "operator.principal_disabled",
        OperatorKeyRevoked = 28 => "operator.key_revoked",
        OperatorBindingRevoked = 29 => "operator.binding_revoked",
        OperatorSessionRevoked = 30 => "operator.session_revoked",
        OperatorDomainContained = 31 => "operator.domain_contained",
        OperatorRotated = 32 => "operator.rotated",
        OperatorRecovered = 33 => "operator.recovered",
        OperatorPrincipalEnabled = 34 => "operator.principal_enabled",
        OperatorArchived = 35 => "operator.archived",
        OperatorDelegationRetired = 36 => "operator.delegation_retired",
        OperatorRequestReplayed = 37 => "operator.request_replayed",
        OperatorReconciled = 38 => "operator.reconciled",
        OperatorEffectRepaired = 39 => "operator.effect_repaired",
        OperatorEmergencyRevoked = 40 => "operator.emergency_revoked",
        InvalidationCommitted = 41 => "invalidation.committed",
        InvalidationObserved = 42 => "invalidation.observed",
        InvalidationReconciled = 43 => "invalidation.reconciled",
        InvalidationFailed = 44 => "invalidation.failed",
        PolicyStale = 45 => "policy.stale",
        KeysetUnknownKey = 46 => "keyset.unknown_key",
        KeysetUnavailable = 47 => "keyset.unavailable",
        StorageUnavailable = 48 => "storage.unavailable",
        ProvenanceAccepted = 49 => "provenance.accepted",
        ProvenanceRejected = 50 => "provenance.rejected",
        EvidenceBackpressure = 51 => "evidence.backpressure",
        EvidenceExported = 52 => "evidence.exported",
        EvidenceQuarantined = 53 => "evidence.quarantined",
        EvidenceDeadLettered = 54 => "evidence.dead_lettered",
        EvidenceRestored = 55 => "evidence.restored",
        EvidenceTamperDetected = 56 => "evidence.tamper_detected",
        BindingRepaired = 57 => "binding.repaired",
        LeaseUseAfterDeniedRefresh = 58 => "lease.use_after_denied_refresh",
        AdministrativeAction = 59 => "administrative.action",
        BreakglassAction = 60 => "breakglass.action",
        KeysetRefreshed = 61 => "keyset.refreshed",
        IssuerUnavailable = 62 => "issuer.unavailable",
        DirectOriginRejected = 63 => "direct_origin.rejected",
        ProvenanceFailed = 64 => "provenance.failed",
    }
}

closed_registry! {
    /// Closed evidence result classification.
    pub enum EventResult {
        Allowed = 1 => "allowed",
        Denied = 2 => "denied",
        Unavailable = 3 => "unavailable",
        Applied = 4 => "applied",
        NoChange = 5 => "no_change",
        Previewed = 6 => "previewed",
        Replayed = 7 => "replayed",
        Quarantined = 8 => "quarantined",
    }
}

closed_registry! {
    /// Trusted decision reason classification.
    pub enum DecisionReason {
        Verified = 1 => "verified",
        PolicyAllowed = 2 => "policy_allowed",
        PolicyDenied = 3 => "policy_denied",
        EvidenceInvalid = 4 => "evidence_invalid",
        EvidenceExpired = 5 => "evidence_expired",
        EvidenceReplayed = 6 => "evidence_replayed",
        EvidenceUnavailable = 7 => "evidence_unavailable",
        DomainMismatch = 8 => "domain_mismatch",
        OperationMismatch = 9 => "operation_mismatch",
        TargetMismatch = 10 => "target_mismatch",
        MissingReason = 11 => "missing_reason",
        MissingApproval = 12 => "missing_approval",
        StaleApproval = 13 => "stale_approval",
        SelfApproval = 14 => "self_approval",
        ReplayedApproval = 15 => "replayed_approval",
        StaleExpectedState = 16 => "stale_expected_state",
        IntentConflict = 17 => "intent_conflict",
        LegacyOperationReserved = 18 => "legacy_operation_reserved",
        StorageUnavailable = 19 => "storage_unavailable",
        CapacityExhausted = 20 => "capacity_exhausted",
        SchemaUnsupported = 21 => "schema_unsupported",
        ExportPoisoned = 22 => "export_poisoned",
        IntegrityFailure = 23 => "integrity_failure",
        Applied = 24 => "applied",
        AlreadyApplied = 25 => "already_applied",
        PreviewOnly = 26 => "preview_only",
        RepairNotAuthorized = 27 => "repair_not_authorized",
        EmergencyScopeDenied = 28 => "emergency_scope_denied",
        UnsupportedExactTarget = 29 => "unsupported_exact_target",
        DirectOriginRejected = 30 => "direct_origin_rejected",
        ProvenanceFailed = 31 => "provenance_failed",
        UnknownKey = 32 => "unknown_key",
        IssuerUnavailable = 33 => "issuer_unavailable",
        RefreshUnavailable = 34 => "refresh_unavailable",
        UpstreamUnavailable = 35 => "upstream_unavailable",
        UnauthorizedActor = 36 => "unauthorized_actor",
        CrossDomain = 37 => "cross_domain",
        ApprovalNotIndependent = 38 => "approval_not_independent",
    }
}

closed_registry! {
    /// Closed actor provenance class.
    pub enum ActorClass {
        NotApplicable = 1 => "not_applicable",
        Unresolved = 2 => "unresolved",
        Direct = 3 => "direct",
        Delegated = 4 => "delegated",
        Operator = 5 => "operator",
        ControlPlane = 6 => "control_plane",
    }
}

closed_registry! {
    /// Protected operation class without resource identifiers.
    pub enum OperationClass {
        NotApplicable = 1 => "not_applicable",
        Read = 2 => "read",
        Write = 3 => "write",
        Publish = 4 => "publish",
        Join = 5 => "join",
        Lifecycle = 6 => "lifecycle",
        Inspection = 7 => "inspection",
        Preview = 8 => "preview",
        Repair = 9 => "repair",
        EmergencyContainment = 10 => "emergency_containment",
    }
}

closed_registry! {
    /// Transport class without routes, hosts, or provider names.
    pub enum TransportClass {
        NotApplicable = 1 => "not_applicable",
        WebSocket = 2 => "websocket",
        Http = 3 => "http",
        Media = 4 => "media",
        Repository = 5 => "repository",
        Audio = 6 => "audio",
        Internal = 7 => "internal",
    }
}

closed_registry! {
    /// Evidence source class without deployment identifiers.
    pub enum SourceClass {
        LocalState = 1 => "local_state",
        VerifiedProof = 2 => "verified_proof",
        VerifiedAssertion = 3 => "verified_assertion",
        Policy = 4 => "policy",
        Lifecycle = 5 => "lifecycle",
        Invalidation = 6 => "invalidation",
        Exporter = 7 => "exporter",
        Restore = 8 => "restore",
    }
}

/// Durable evidence lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum EvidenceStreamKind {
    /// Transactional evidence for a committed state transition.
    AuditOutbox = 1,
    /// Durable evidence for a non-mutating decision.
    Decision = 2,
    /// Independent bounded pipeline-control evidence.
    Control = 3,
}

impl EvidenceStreamKind {
    /// Stable storage code.
    pub const fn discriminant(self) -> u8 {
        self as u8
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn assert_unique<T: Copy>(
        values: &[T],
        discriminant: impl Fn(T) -> u16,
        code: impl Fn(T) -> &'static str,
    ) {
        let numeric = values
            .iter()
            .copied()
            .map(&discriminant)
            .collect::<BTreeSet<_>>();
        let names = values.iter().copied().map(&code).collect::<BTreeSet<_>>();
        assert_eq!(numeric.len(), values.len());
        assert_eq!(names.len(), values.len());
    }

    #[test]
    fn registries_have_unique_numeric_and_text_codes() {
        assert_unique(EventKind::ALL, EventKind::discriminant, EventKind::code);
        assert_unique(
            EventResult::ALL,
            EventResult::discriminant,
            EventResult::code,
        );
        assert_unique(
            DecisionReason::ALL,
            DecisionReason::discriminant,
            DecisionReason::code,
        );
        assert_unique(ActorClass::ALL, ActorClass::discriminant, ActorClass::code);
        assert_unique(
            OperationClass::ALL,
            OperationClass::discriminant,
            OperationClass::code,
        );
        assert_unique(
            TransportClass::ALL,
            TransportClass::discriminant,
            TransportClass::code,
        );
        assert_unique(
            SourceClass::ALL,
            SourceClass::discriminant,
            SourceClass::code,
        );
    }
}
