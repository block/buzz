//! Test-only J3C relay-authenticated client-binding status composition.
//!
//! This harness deliberately composes only public production contracts. It
//! binds a real loopback WebSocket, carries a real NIP-42 `AUTH` frame through
//! the Buzz parser and verifier, creates verification-only evidence through the
//! authorization finalizer, and asks the production issuer and exact-connection
//! transport to deliver. The production J1 native session source consumes the
//! resulting bootstrap/status frames.

extern crate buzz_core as buzz_core_pkg;

#[path = "../../../desktop/src-tauri/src/client_binding_status_session.rs"]
mod client_binding_status_session;

use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use buzz_auth::context::BindingExpiry;
use buzz_auth::evidence_adapter::{ActiveBindingResolution, VerifiedEvidenceAdapter};
use buzz_auth::{
    resolve_authorization, resolve_current_federated_policy, ApplicationLeaseLimit,
    AssertionTransport, AuthContextInput, AuthTransport, AuthorityAdapterError,
    AuthorityAdapterFuture, AuthorizationCapability, AuthorizationClock, AuthorizationClockError,
    AuthorizationClockSkew, AuthorizationFinalizer, AuthorizationOutcome, AuthorizationProfileId,
    AuthorizationProvider, AuthorizationProviderFuture, AuthorizationRequest, AuthorizationTime,
    AuthorizedCommunityAccess, BindingLeaseBound, BindingResolutionRequest, BindingSource,
    BindingVersion, CapabilitySet, CurrentPolicyRequest, CurrentPolicyResolutionSink,
    DirectBindingResolutionSink, EnrollmentMode, ExistingBindingResolutionSink,
    FederatedAuthorityAdapter, FederatedAuthorization, FederatedIdentityRequirement, PolicyVersion,
    ProviderAllow, ProviderAuthorizationClock, ProviderDecision, ProviderTimeout, Scope,
    VerificationOnlyDisposition, VerificationStatusPolicy, VerifiedNostrProof,
};
use buzz_core::client_binding_bootstrap::{
    ClientBindingBootstrapInputV1, ClientBindingEpoch, ClientBindingScopeV1,
    CLIENT_BINDING_BOOTSTRAP_SUB_ID, CLIENT_BINDING_SCOPE_TAG, CLIENT_BINDING_STATUS_SUB_ID,
};
use buzz_core::client_binding_status::{
    ClientBindingStatusError, ClientBindingStatusFoldError, ClientBindingStatusInputV1,
    ClientBindingStatusTracker, ClientBindingStatusUpdate,
};
use buzz_core::kind::{KIND_CLIENT_BINDING_STATUS, KIND_USER_TRUSTED_ASSERTION};
use buzz_core::CommunityId;
use buzz_relay::authorization_runtime::status::{
    AuthoritativeClientStatusEvidence, ClientStatusPresentationGateError,
    ClientStatusPresentationPermit, ClientStatusPrivacyKey, ClientStatusRevisionScope,
    CompleteClientStatusPresentationApproval, ConnectionManagerClientStatusTransport,
    DurableClientStatusRevision, DurableClientStatusRevisionSource, ProviderNeutralPolicyRevision,
    RelayClientBindingStatusIssuer,
};
use buzz_relay::connection::OutboundData;
use buzz_relay::protocol::{ClientMessage, RelayMessage};
use buzz_relay::state::ConnectionManager;
use client_binding_status_session::{
    ClientBindingStatusSession, CurrentProjection, ProjectionUpdate,
};
use futures::{SinkExt, StreamExt};
use nostr::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag, Timestamp};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, oneshot, Mutex as AsyncMutex};
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
struct FixedClock(u64);

impl AuthorizationClock for FixedClock {
    fn now(&self) -> Result<AuthorizationTime, AuthorizationClockError> {
        Ok(AuthorizationTime::from_unix_seconds(self.0))
    }
}

impl ProviderAuthorizationClock for FixedClock {
    fn now_unix_seconds(&self) -> Option<u64> {
        Some(self.0)
    }
}

struct SyntheticAuthority {
    policy_id: Uuid,
    binding_id: Uuid,
    binding_version: BindingVersion,
    valid_until: u64,
}

impl SyntheticAuthority {
    fn resolve_binding(
        &self,
        request: BindingResolutionRequest,
        sink: DirectBindingResolutionSink,
    ) -> Result<buzz_auth::context::AuthoritativeBindingResolution, AuthorityAdapterError<Infallible>>
    {
        Ok(sink.existing_active(
            request.authorization_domain(),
            self.binding_id,
            request.principal().clone(),
            request.bound_pubkey(),
            self.binding_version,
            Some(BindingExpiry::new(self.valid_until)?),
            BindingSource::AttestedKey,
        )?)
    }
}

impl FederatedAuthorityAdapter for SyntheticAuthority {
    type Error = Infallible;

    fn resolve_current_policy<'a>(
        &'a self,
        request: CurrentPolicyRequest,
        sink: CurrentPolicyResolutionSink,
    ) -> AuthorityAdapterFuture<
        'a,
        Result<buzz_auth::ResolvedFederatedPolicy, AuthorityAdapterError<Self::Error>>,
    > {
        Box::pin(async move {
            Ok(sink.resolved(
                request.authorization_domain(),
                self.policy_id,
                1,
                FederatedIdentityRequirement::Required(EnrollmentMode::AttestedKey),
                request.observed_at().saturating_sub(1),
                self.valid_until,
            )?)
        })
    }

    fn resolve_direct_binding<'a>(
        &'a self,
        request: BindingResolutionRequest,
        sink: DirectBindingResolutionSink,
    ) -> AuthorityAdapterFuture<
        'a,
        Result<
            buzz_auth::context::AuthoritativeBindingResolution,
            AuthorityAdapterError<Self::Error>,
        >,
    > {
        Box::pin(async move { self.resolve_binding(request, sink) })
    }

    fn resolve_existing_binding<'a>(
        &'a self,
        request: BindingResolutionRequest,
        sink: ExistingBindingResolutionSink,
    ) -> AuthorityAdapterFuture<
        'a,
        Result<
            buzz_auth::context::AuthoritativeBindingResolution,
            AuthorityAdapterError<Self::Error>,
        >,
    > {
        Box::pin(async move {
            Ok(sink.existing_active(
                request.authorization_domain(),
                self.binding_id,
                request.principal().clone(),
                request.bound_pubkey(),
                self.binding_version,
                Some(BindingExpiry::new(self.valid_until)?),
                BindingSource::AttestedKey,
            )?)
        })
    }
}

struct SyntheticProvider {
    profile: AuthorizationProfileId,
    policy: PolicyVersion,
    issued_at: u64,
    fresh_until: u64,
}

impl AuthorizationProvider for SyntheticProvider {
    fn profile_id(&self) -> AuthorizationProfileId {
        self.profile.clone()
    }

    fn authorize<'a>(
        &'a self,
        request: &'a AuthorizationRequest,
    ) -> AuthorizationProviderFuture<'a> {
        let decision = ProviderAllow::new(
            request.authorization_domain(),
            request.principal().clone(),
            self.profile.clone(),
            request.requested_capabilities().clone(),
            self.policy.clone(),
            self.issued_at,
            self.fresh_until,
        )
        .expect("synthetic provider output must satisfy the production contract");
        Box::pin(std::future::ready(ProviderDecision::Allow(decision)))
    }
}

struct CompleteSyntheticApproval {
    reviewed_revision: String,
}

impl CompleteClientStatusPresentationApproval for CompleteSyntheticApproval {
    fn reviewed_implementation_revision(&self) -> &str {
        &self.reviewed_revision
    }

    fn presentation_gate_passed(&self) -> bool {
        true
    }

    fn dedicated_client_contract_passed(&self) -> bool {
        true
    }
}

struct SyntheticRevisions {
    revision: AtomicU64,
    current_reads: AtomicUsize,
    withdrawal_reads: AtomicUsize,
    scopes: Mutex<Vec<ClientStatusRevisionScope>>,
}

impl SyntheticRevisions {
    fn new(revision: u64) -> Self {
        Self {
            revision: AtomicU64::new(revision),
            current_reads: AtomicUsize::new(0),
            withdrawal_reads: AtomicUsize::new(0),
            scopes: Mutex::new(Vec::new()),
        }
    }

    fn set(&self, revision: u64) {
        self.revision.store(revision, Ordering::SeqCst);
    }

    fn durable(&self) -> Option<DurableClientStatusRevision> {
        let revision = self.revision.load(Ordering::SeqCst);
        DurableClientStatusRevision::from_durable_state(revision, revision).ok()
    }
}

#[async_trait]
impl DurableClientStatusRevisionSource for SyntheticRevisions {
    async fn current_revision_for(
        &self,
        requirement: &buzz_relay::authorization_runtime::status::ClientStatusCurrentRequirement<'_>,
        _issuance_fingerprint: [u8; 32],
    ) -> Option<DurableClientStatusRevision> {
        self.current_reads.fetch_add(1, Ordering::SeqCst);
        self.scopes
            .lock()
            .expect("synthetic revision scope lock")
            .push(requirement.scope());
        self.durable()
    }

    async fn withdrawal_revision_for(
        &self,
        receipt: &buzz_relay::authorization_runtime::status::ClientStatusIssuanceReceipt,
        _withdrawal_fingerprint: [u8; 32],
    ) -> Option<DurableClientStatusRevision> {
        self.withdrawal_reads.fetch_add(1, Ordering::SeqCst);
        assert!(!receipt.connection_id().is_nil());
        self.durable()
    }
}

async fn verification_only_disposition(
    domain: CommunityId,
    author: &Keys,
    now: u64,
    proof: VerifiedNostrProof,
) -> (VerificationOnlyDisposition, ClientStatusPrivacyKey) {
    let adapter = VerifiedEvidenceAdapter::new();
    let issuer = format!("https://{}.invalid", Uuid::new_v4());
    let subject = Uuid::new_v4().to_string();
    let assertion = adapter
        .federated_assertion_from_validated_claims(
            domain,
            AuthTransport::RelayWebSocket,
            &issuer,
            &subject,
            Some(author.public_key()),
            AssertionTransport::TrustedProxy,
            Some(now.saturating_sub(1)),
            now + 240,
            now,
        )
        .expect("synthetic validated claims seal exact assertion evidence");
    let correlation_id = Uuid::new_v4();
    let authority = SyntheticAuthority {
        policy_id: Uuid::new_v4(),
        binding_id: Uuid::new_v4(),
        binding_version: BindingVersion::new(7).expect("positive synthetic binding version"),
        valid_until: now + 240,
    };
    let policy = resolve_current_federated_policy(&authority, domain, correlation_id, now)
        .await
        .expect("test-only authoritative policy resolves");
    let profile = AuthorizationProfileId::from_server_configuration(format!(
        "synthetic-profile-{}",
        Uuid::new_v4()
    ))
    .expect("synthetic profile is valid");
    let provider_policy = PolicyVersion::new(format!("private-policy-{}", Uuid::new_v4()))
        .expect("synthetic provider policy is valid");
    let capabilities = CapabilitySet::single(AuthorizationCapability::CommunityRead);
    let request = AuthorizationRequest::direct(
        &proof,
        &assertion,
        policy,
        capabilities,
        correlation_id,
        now,
    )
    .expect("exact direct provider request is valid");
    let provider = SyntheticProvider {
        profile: profile.clone(),
        policy: provider_policy,
        issued_at: now,
        fresh_until: now + 180,
    };
    let snapshot = match resolve_authorization(
        &provider,
        &request,
        &FixedClock(now),
        ProviderTimeout::new(Duration::from_secs(1)).expect("bounded provider timeout"),
        Uuid::new_v4(),
    )
    .await
    {
        AuthorizationOutcome::Allow(snapshot) => snapshot,
        other => panic!("synthetic exact provider request must allow: {other:?}"),
    };
    let binding = adapter
        .active_binding_from_store(
            domain,
            domain,
            authority.binding_id,
            &issuer,
            &subject,
            author.public_key(),
            authority.binding_version.get(),
            Some(authority.valid_until),
            BindingSource::AttestedKey,
            ActiveBindingResolution::Existing,
            Some(&assertion),
        )
        .expect("typed current binding store output seals");
    let binding_bound = BindingLeaseBound::new(&binding, authority.valid_until)
        .expect("synthetic binding bound is current");
    let tenant =
        buzz_core::tenant::TenantContext::resolved(domain, format!("{}.invalid", Uuid::new_v4()));
    let admission: AuthorizedCommunityAccess = adapter
        .community_access_from_policy(&tenant, domain, vec![Scope::MessagesRead], None)
        .expect("server-resolved community admission seals");
    let input = AuthContextInput::new(tenant, correlation_id, proof, admission);
    let policy = resolve_current_federated_policy(&authority, domain, correlation_id, now)
        .await
        .expect("same current policy resolves at finalization");
    let finalizer = AuthorizationFinalizer::new(Arc::new(FixedClock(now)));
    let disposition = finalizer
        .finalize_verification_only(
            input,
            policy,
            FederatedAuthorization::Direct { binding, assertion },
            snapshot,
            &profile,
            binding_bound,
            VerificationStatusPolicy::new(
                ApplicationLeaseLimit::from_seconds(120).expect("short display lifetime is valid"),
                AuthorizationClockSkew::from_seconds(0).expect("zero skew is valid"),
            ),
        )
        .expect("production finalizer yields display-only evidence");
    let privacy_key = ClientStatusPrivacyKey::from_secret(rand::random());
    (disposition, privacy_key)
}

fn register_connection(
    connections: &ConnectionManager,
    connection_id: Uuid,
    domain: CommunityId,
) -> (
    mpsc::Receiver<OutboundData>,
    mpsc::Receiver<axum::extract::ws::Message>,
) {
    let (tx, rx) = mpsc::channel(16);
    let (ctrl_tx, ctrl_rx) = mpsc::channel(4);
    connections.register(
        connection_id,
        tx,
        ctrl_tx,
        CancellationToken::new(),
        domain,
        Arc::new(AtomicU8::new(0)),
        Arc::new(AsyncMutex::new(HashMap::new())),
        3,
    );
    (rx, ctrl_rx)
}

fn authoritative_evidence(
    disposition: &VerificationOnlyDisposition,
    privacy_key: &ClientStatusPrivacyKey,
) -> AuthoritativeClientStatusEvidence {
    let policy_revision = ProviderNeutralPolicyRevision::derive(
        privacy_key,
        disposition.profile_id(),
        disposition.policy_version(),
    )
    .expect("ephemeral privacy key derives provider-neutral policy revision");
    AuthoritativeClientStatusEvidence::from_authoritative_runtime(
        disposition.authorization_domain(),
        disposition.actor_pubkey(),
        disposition.binding_id(),
        disposition.binding_version(),
        disposition.profile_id().clone(),
        disposition.policy_version().clone(),
        policy_revision,
        disposition.correlation_id(),
        1,
        disposition.issued_at(),
        disposition.expires_at(),
    )
}

fn raw_signed_event(keys: &Keys, kind: u32, content: String, issued_at: u64) -> Event {
    EventBuilder::new(Kind::Custom(kind as u16), content)
        .custom_created_at(Timestamp::from(issued_at))
        .sign_with_keys(keys)
        .expect("ephemeral synthetic event signs")
}

async fn receive_status(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> (String, Event) {
    let message = timeout(Duration::from_secs(2), socket.next())
        .await
        .expect("loopback status frame must not time out")
        .expect("loopback relay remains connected")
        .expect("loopback WebSocket frame is valid");
    let Message::Text(text) = message else {
        panic!("client status must use a text WebSocket frame");
    };
    let envelope: Value = serde_json::from_str(&text).expect("relay frame is JSON");
    assert_eq!(envelope[0], "EVENT");
    let event =
        Event::from_json(envelope[2].to_string()).expect("relay frame carries a Nostr event");
    (text.to_string(), event)
}

async fn receive_transport_event(
    expected_frames: &mpsc::UnboundedSender<String>,
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    subscription: &str,
    event: &Event,
) -> (String, Event) {
    expected_frames
        .send(RelayMessage::event(subscription, event))
        .expect("queue-drain adapter remains live");
    let (text, received) = receive_status(socket).await;
    assert_eq!(received.id, event.id, "wire event must be issuer-produced");
    (text, received)
}

async fn enqueue_and_receive_event(
    connections: &ConnectionManager,
    connection_id: Uuid,
    expected_frames: &mpsc::UnboundedSender<String>,
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    subscription: &str,
    event: &Event,
) -> (String, Event) {
    let frame = RelayMessage::event(subscription, event);
    expected_frames
        .send(frame.clone())
        .expect("queue-drain adapter remains live");
    assert!(
        connections.send_to(connection_id, frame),
        "production ConnectionManager must enqueue the exact-connection frame"
    );
    let (text, received) = receive_status(socket).await;
    assert_eq!(received.id, event.id, "wire event must be sender-produced");
    (text, received)
}

fn assert_current_projection(
    update: Option<ProjectionUpdate>,
    author: &Keys,
    epoch: &ClientBindingEpoch,
    fresh_until: u64,
) {
    let Some(ProjectionUpdate::Current(CurrentProjection {
        event_author_pubkey,
        fresh_until: projected_fresh_until,
        connection_epoch,
    })) = update
    else {
        panic!("production J1 session must project current status");
    };
    assert_eq!(event_author_pubkey, author.public_key().to_hex());
    assert_eq!(projected_fresh_until, fresh_until);
    assert_eq!(connection_epoch, epoch.as_str());
}

fn assert_clear(update: Option<ProjectionUpdate>) {
    assert!(matches!(update, Some(ProjectionUpdate::Clear)));
}

#[tokio::test]
async fn relay_authenticated_status_uses_real_loopback_and_exact_connection_scope() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("ephemeral loopback listener binds");
    let address = listener
        .local_addr()
        .expect("loopback listener has an address");
    assert_ne!(address.port(), 0, "OS must allocate a real ephemeral port");
    let relay_url = format!("ws://{address}");
    let now = Timestamp::now().as_secs();
    let relay = Keys::generate();
    let author = Keys::generate();
    let spoof = Keys::generate();
    let wrong_relay = Keys::generate();
    let domain = CommunityId::from_uuid(Uuid::new_v4());
    let wrong_domain = CommunityId::from_uuid(Uuid::new_v4());
    let connection_id = Uuid::new_v4();
    let epoch = ClientBindingEpoch::new_v4();
    let challenge = Uuid::new_v4().to_string();
    let auth_event = EventBuilder::new(Kind::Custom(22242), "")
        .tags([
            Tag::parse(vec!["relay", relay_url.as_str()]).expect("loopback relay tag is valid"),
            Tag::parse(vec!["challenge", challenge.as_str()])
                .expect("ephemeral challenge tag is valid"),
            Tag::parse(vec![
                CLIENT_BINDING_SCOPE_TAG.to_string(),
                "1".to_string(),
                epoch.as_str().to_string(),
                relay.public_key().to_hex(),
            ])
            .expect("native status scope tag is valid"),
        ])
        .sign_with_keys(&author)
        .expect("ephemeral author signs scoped NIP-42 proof");

    let connections = Arc::new(ConnectionManager::new());
    let (mut outbound_rx, _ctrl_rx) = register_connection(&connections, connection_id, domain);
    let (expected_frame_tx, mut expected_frame_rx) = mpsc::unbounded_channel::<String>();
    let (auth_proof_tx, auth_proof_rx) =
        oneshot::channel::<(VerifiedNostrProof, ClientBindingScopeV1)>();
    let server_relay_url = relay_url.clone();
    let server_challenge = challenge.clone();
    let expected_relay_signer = relay.public_key();
    let server = tokio::spawn(async move {
        let (tcp, peer) = listener.accept().await.expect("loopback client connects");
        assert!(peer.ip().is_loopback(), "harness must remain loopback-only");
        let mut websocket = tokio_tungstenite::accept_async(tcp)
            .await
            .expect("loopback WebSocket upgrades");

        let auth_text = match websocket.next().await {
            Some(Ok(Message::Text(text))) => text,
            _ => panic!("first loopback client frame must be text AUTH"),
        };
        let ClientMessage::Auth(event) =
            ClientMessage::parse(&auth_text).expect("Buzz parser accepts real AUTH frame")
        else {
            panic!("first loopback client frame must parse as AUTH");
        };
        let proof = VerifiedEvidenceAdapter::new()
            .verify_nip42(
                domain,
                AuthTransport::RelayWebSocket,
                &event,
                &server_challenge,
                &server_relay_url,
                None,
            )
            .expect("Buzz verifier seals AUTH received over loopback");
        let status_scope = ClientBindingScopeV1::from_verified_auth_event(&event)
            .expect("verified AUTH carries one exact signed status scope");
        assert_eq!(status_scope.relay_signer(), expected_relay_signer);
        auth_proof_tx
            .send((proof, status_scope))
            .expect("test driver awaits verified AUTH evidence");

        let mut sent = 0usize;
        while let Some(queued) = outbound_rx.recv().await {
            let frame = expected_frame_rx
                .recv()
                .await
                .expect("each production queue item has one test-visible oracle frame");
            // `OutboundData::release` is intentionally crate-private. Receiving
            // and consuming this value proves the production manager/transport
            // emitted before the test-only adapter sends its matching oracle.
            drop(queued);
            websocket
                .send(Message::Text(frame.into()))
                .await
                .expect("queue-gated loopback frame sends");
            sent += 1;
        }
        let _ = websocket.close(None).await;
        sent
    });

    let (mut socket, _) = tokio_tungstenite::connect_async(&relay_url)
        .await
        .expect("real loopback WebSocket client connects");
    socket
        .send(Message::Text(
            json!([
                "AUTH",
                serde_json::to_value(&auth_event).expect("AUTH serializes")
            ])
            .to_string()
            .into(),
        ))
        .await
        .expect("real AUTH frame crosses loopback WebSocket");
    let (proof, status_scope) = auth_proof_rx
        .await
        .expect("server returns exact verified AUTH evidence");
    assert_eq!(proof.actor_pubkey(), author.public_key());
    assert_eq!(status_scope.connection_epoch(), &epoch);
    assert_eq!(status_scope.relay_signer(), relay.public_key());
    connections.set_authenticated_pubkey(connection_id, proof.actor_pubkey().to_bytes().to_vec());

    let (disposition, privacy_key) =
        verification_only_disposition(domain, &author, now, proof).await;
    let evidence = authoritative_evidence(&disposition, &privacy_key);
    let revisions = SyntheticRevisions::new(10);
    let issuer = RelayClientBindingStatusIssuer::new(&relay, &revisions, &privacy_key);
    let permit = ClientStatusPresentationPermit::from_complete_stack(&CompleteSyntheticApproval {
        reviewed_revision: "a".repeat(40),
    })
    .expect("test-only complete approval constructs the production gate");

    let transport = ConnectionManagerClientStatusTransport::new(Arc::clone(&connections));

    let authenticated_epoch = status_scope.connection_epoch().clone();
    let bootstrap = ClientBindingBootstrapInputV1::new(
        domain,
        author.public_key(),
        authenticated_epoch.clone(),
        now,
    )
    .expect("authenticated connection scope creates bootstrap input")
    .sign_with_relay_keys(&relay)
    .expect("relay signs exact connection bootstrap");
    let mut session = ClientBindingStatusSession::new(
        relay.public_key(),
        author.public_key(),
        authenticated_epoch,
    );
    let (bootstrap_text, received_bootstrap) = enqueue_and_receive_event(
        &connections,
        connection_id,
        &expected_frame_tx,
        &mut socket,
        CLIENT_BINDING_BOOTSTRAP_SUB_ID,
        &bootstrap,
    )
    .await;
    assert_eq!(received_bootstrap.id, bootstrap.id);
    assert!(matches!(
        session.consume_text(&bootstrap_text, now),
        Some(ProjectionUpdate::Unchanged)
    ));

    let current_attempt = issuer
        .deliver_verification_only(&permit, &disposition, 1, None, connection_id, &transport)
        .await
        .expect("production issuer creates current status");
    assert_eq!(current_attempt.delivery_error(), None);
    assert_eq!(current_attempt.receipt().connection_id(), connection_id);
    assert_eq!(current_attempt.receipt().revision(), 10);
    assert_eq!(current_attempt.event().pubkey, relay.public_key());
    assert!(current_attempt.event().verify().is_ok());
    assert!(!current_attempt
        .event()
        .content
        .contains(disposition.profile_id().as_str()));
    assert!(!current_attempt
        .event()
        .content
        .contains(disposition.policy_version().as_str()));

    // The same issuer cannot target a connection authenticated as another key
    // or resolved for another authorization domain.
    let wrong_author_connection = Uuid::new_v4();
    let (_wrong_author_rx, _wrong_author_ctrl_rx) =
        register_connection(&connections, wrong_author_connection, domain);
    connections.set_authenticated_pubkey(
        wrong_author_connection,
        spoof.public_key().to_bytes().to_vec(),
    );
    let wrong_author_attempt = issuer
        .deliver_verification_only(
            &permit,
            &disposition,
            1,
            None,
            wrong_author_connection,
            &transport,
        )
        .await
        .expect("issuance succeeds independently of exact delivery");
    assert!(wrong_author_attempt.delivery_error().is_some());

    let wrong_domain_connection = Uuid::new_v4();
    let (_wrong_domain_rx, _wrong_domain_ctrl_rx) =
        register_connection(&connections, wrong_domain_connection, wrong_domain);
    connections.set_authenticated_pubkey(
        wrong_domain_connection,
        author.public_key().to_bytes().to_vec(),
    );
    let wrong_domain_attempt = issuer
        .deliver_verification_only(
            &permit,
            &disposition,
            1,
            None,
            wrong_domain_connection,
            &transport,
        )
        .await
        .expect("issuance succeeds independently of exact delivery");
    assert!(wrong_domain_attempt.delivery_error().is_some());

    revisions.set(10);
    let (current_text, current) = receive_transport_event(
        &expected_frame_tx,
        &mut socket,
        CLIENT_BINDING_STATUS_SUB_ID,
        current_attempt.event(),
    )
    .await;
    assert_current_projection(
        session.consume_text(&current_text, now),
        &author,
        &epoch,
        disposition.expires_at(),
    );
    let mut tracker =
        ClientBindingStatusTracker::new(relay.public_key(), domain, author.public_key());
    assert_eq!(
        tracker.accept(&current, now),
        Ok(ClientBindingStatusUpdate::Accepted)
    );
    assert!(tracker.current_presentation(now).is_some());
    assert_eq!(tracker.high_water_revision(), Some(10));

    let malformed = raw_signed_event(&relay, KIND_CLIENT_BINDING_STATUS, "{".to_string(), now);
    let (malformed_text, malformed) = enqueue_and_receive_event(
        &connections,
        connection_id,
        &expected_frame_tx,
        &mut socket,
        CLIENT_BINDING_STATUS_SUB_ID,
        &malformed,
    )
    .await;
    assert_clear(session.consume_text(&malformed_text, now));
    assert_eq!(
        tracker.accept(&malformed, now),
        Err(ClientBindingStatusFoldError::InvalidStatus(
            ClientBindingStatusError::MalformedPayload
        ))
    );

    let mut unsupported_content: Value =
        serde_json::from_str(&current.content).expect("current status content is JSON");
    unsupported_content["version"] = json!(2);
    let unsupported = raw_signed_event(
        &relay,
        KIND_CLIENT_BINDING_STATUS,
        unsupported_content.to_string(),
        now,
    );
    let (unsupported_text, unsupported) = enqueue_and_receive_event(
        &connections,
        connection_id,
        &expected_frame_tx,
        &mut socket,
        CLIENT_BINDING_STATUS_SUB_ID,
        &unsupported,
    )
    .await;
    assert_clear(session.consume_text(&unsupported_text, now));
    assert_eq!(
        tracker.accept(&unsupported, now),
        Err(ClientBindingStatusFoldError::InvalidStatus(
            ClientBindingStatusError::UnsupportedVersion
        ))
    );

    let wrong_signer = ClientBindingStatusInputV1::current(
        domain,
        author.public_key(),
        7,
        "opaque-wrong-relay",
        11,
        now,
        now + 120,
        None,
    )
    .expect("bounded wrong-relay status input")
    .sign_with_relay_keys(&wrong_relay)
    .expect("wrong relay still produces an authenticated Nostr event");
    let (wrong_signer_text, wrong_signer) = enqueue_and_receive_event(
        &connections,
        connection_id,
        &expected_frame_tx,
        &mut socket,
        CLIENT_BINDING_STATUS_SUB_ID,
        &wrong_signer,
    )
    .await;
    assert!(matches!(
        session.consume_text(&wrong_signer_text, now),
        Some(ProjectionUpdate::Unchanged)
    ));
    assert_eq!(
        tracker.accept(&wrong_signer, now),
        Err(ClientBindingStatusFoldError::InvalidStatus(
            ClientBindingStatusError::UnexpectedRelay
        ))
    );

    let author_mismatch = ClientBindingStatusInputV1::current(
        domain,
        spoof.public_key(),
        7,
        "opaque-author-spoof",
        11,
        now,
        now + 120,
        None,
    )
    .expect("bounded mismatched-author status input")
    .sign_with_relay_keys(&relay)
    .expect("relay signs explicit mismatched-author test event");
    let (author_mismatch_text, author_mismatch) = enqueue_and_receive_event(
        &connections,
        connection_id,
        &expected_frame_tx,
        &mut socket,
        CLIENT_BINDING_STATUS_SUB_ID,
        &author_mismatch,
    )
    .await;
    assert_clear(session.consume_text(&author_mismatch_text, now));
    assert_eq!(
        tracker.accept(&author_mismatch, now),
        Err(ClientBindingStatusFoldError::InvalidStatus(
            ClientBindingStatusError::EventAuthorMismatch
        ))
    );

    let domain_mismatch = ClientBindingStatusInputV1::current(
        wrong_domain,
        author.public_key(),
        7,
        "opaque-domain-spoof",
        11,
        now,
        now + 120,
        None,
    )
    .expect("bounded mismatched-domain status input")
    .sign_with_relay_keys(&relay)
    .expect("relay signs explicit mismatched-domain test event");
    let (domain_mismatch_text, domain_mismatch) = enqueue_and_receive_event(
        &connections,
        connection_id,
        &expected_frame_tx,
        &mut socket,
        CLIENT_BINDING_STATUS_SUB_ID,
        &domain_mismatch,
    )
    .await;
    assert_clear(session.consume_text(&domain_mismatch_text, now));
    assert_eq!(
        tracker.accept(&domain_mismatch, now),
        Err(ClientBindingStatusFoldError::InvalidStatus(
            ClientBindingStatusError::AuthorizationDomainMismatch
        ))
    );

    // Neither mutable profile metadata nor a legacy NIP-85 assertion can
    // restore or rename the relay-authenticated status presentation.
    for (legacy, expected_clear) in [
        (
            raw_signed_event(
                &spoof,
                Kind::Metadata.as_u16() as u32,
                json!({"name": format!("spoof-{}", Uuid::new_v4())}).to_string(),
                now,
            ),
            false,
        ),
        (
            raw_signed_event(
                &relay,
                KIND_USER_TRUSTED_ASSERTION,
                json!({"active": true, "label": format!("legacy-{}", Uuid::new_v4())}).to_string(),
                now,
            ),
            true,
        ),
    ] {
        let (legacy_text, legacy) = enqueue_and_receive_event(
            &connections,
            connection_id,
            &expected_frame_tx,
            &mut socket,
            CLIENT_BINDING_STATUS_SUB_ID,
            &legacy,
        )
        .await;
        let update = session.consume_text(&legacy_text, now);
        if expected_clear {
            assert_clear(update);
        } else {
            assert!(matches!(update, Some(ProjectionUpdate::Unchanged)));
        }
        assert_eq!(
            tracker.accept(&legacy, now),
            Err(ClientBindingStatusFoldError::InvalidStatus(
                ClientBindingStatusError::WrongKind
            ))
        );
        assert_eq!(tracker.high_water_revision(), Some(10));
        assert_eq!(
            tracker
                .current_presentation(now)
                .and_then(|status| status.display_label()),
            None
        );
    }

    revisions.set(11);
    let withdrawal = issuer
        .deliver_withdrawn_after_invalidation(
            &permit,
            &evidence,
            current_attempt.receipt(),
            &transport,
        )
        .await
        .expect("production issuer delivers a strictly newer withdrawal");
    let (withdrawal_text, withdrawal) = receive_transport_event(
        &expected_frame_tx,
        &mut socket,
        CLIENT_BINDING_STATUS_SUB_ID,
        &withdrawal,
    )
    .await;
    assert_clear(session.consume_text(&withdrawal_text, now));
    assert_eq!(
        tracker.accept(&withdrawal, now),
        Ok(ClientBindingStatusUpdate::Accepted)
    );
    assert!(tracker.current_presentation(now).is_none());
    assert_eq!(
        tracker.accept(&withdrawal, now),
        Ok(ClientBindingStatusUpdate::Duplicate)
    );
    assert_eq!(
        tracker.accept(&current, now),
        Err(ClientBindingStatusFoldError::LowerRevisionReplay)
    );

    let equal_conflict = ClientBindingStatusInputV1::current(
        domain,
        author.public_key(),
        8,
        "opaque-equal-conflict",
        11,
        now,
        now + 120,
        None,
    )
    .expect("equal-revision conflict input is structurally valid")
    .sign_with_relay_keys(&relay)
    .expect("relay signs explicit conflict event");
    let (equal_conflict_text, equal_conflict) = enqueue_and_receive_event(
        &connections,
        connection_id,
        &expected_frame_tx,
        &mut socket,
        CLIENT_BINDING_STATUS_SUB_ID,
        &equal_conflict,
    )
    .await;
    assert_clear(session.consume_text(&equal_conflict_text, now));
    assert_eq!(
        tracker.accept(&equal_conflict, now),
        Err(ClientBindingStatusFoldError::ConflictingEqualRevision)
    );

    // A trusted relay event in a malformed reserved outer frame must clear and
    // consume its revision. Replaying the same event in an exact frame cannot
    // restore; only a strictly newer issuer event may do so (J1 fail-closed
    // high-water latch).
    let trusted_invalid_current = ClientBindingStatusInputV1::current(
        domain,
        author.public_key(),
        8,
        "opaque-trusted-invalid",
        12,
        now,
        disposition.expires_at(),
        None,
    )
    .expect("trusted-invalid inner status is structurally valid")
    .sign_with_relay_keys(&relay)
    .expect("trusted relay signs inner status");
    let trusted_invalid_frame = json!([
        "EVENT",
        CLIENT_BINDING_STATUS_SUB_ID,
        serde_json::to_value(&trusted_invalid_current).expect("status serializes"),
        {"unexpected": true}
    ])
    .to_string();
    expected_frame_tx
        .send(trusted_invalid_frame.clone())
        .expect("queue-drain adapter remains live");
    assert!(connections.send_to(connection_id, trusted_invalid_frame));
    let (trusted_invalid_text, received_trusted_invalid) = receive_status(&mut socket).await;
    assert_eq!(received_trusted_invalid.id, trusted_invalid_current.id);
    assert_clear(session.consume_text(&trusted_invalid_text, now));
    assert_eq!(session.projected_fresh_until(), None);

    let (equal_replay_text, equal_replay) = enqueue_and_receive_event(
        &connections,
        connection_id,
        &expected_frame_tx,
        &mut socket,
        CLIENT_BINDING_STATUS_SUB_ID,
        &trusted_invalid_current,
    )
    .await;
    assert_eq!(equal_replay.id, trusted_invalid_current.id);
    assert!(matches!(
        session.consume_text(&equal_replay_text, now),
        Some(ProjectionUpdate::Unchanged)
    ));
    assert_eq!(session.projected_fresh_until(), None);

    revisions.set(13);
    let restored_attempt = issuer
        .deliver_verification_only(&permit, &disposition, 1, None, connection_id, &transport)
        .await
        .expect("production issuer creates strictly newer restoration");
    assert_eq!(restored_attempt.delivery_error(), None);
    let (restored_text, restored) = receive_transport_event(
        &expected_frame_tx,
        &mut socket,
        CLIENT_BINDING_STATUS_SUB_ID,
        restored_attempt.event(),
    )
    .await;
    assert_current_projection(
        session.consume_text(&restored_text, now),
        &author,
        &epoch,
        disposition.expires_at(),
    );
    assert_eq!(
        tracker.accept(&restored, now),
        Ok(ClientBindingStatusUpdate::Accepted)
    );
    assert!(tracker.current_presentation(now).is_some());

    tracker.on_disconnect();
    assert!(tracker.current_presentation(now).is_none());
    assert_eq!(tracker.high_water_revision(), Some(13));
    assert_eq!(
        tracker.accept(&restored, now),
        Ok(ClientBindingStatusUpdate::Duplicate),
        "reconnect must not restore presentation from a duplicate"
    );
    assert!(tracker.current_presentation(now).is_none());

    assert_clear(Some(session.disconnect()));
    assert_eq!(session.projected_fresh_until(), None);
    assert!(matches!(
        session.consume_text(&restored_text, now),
        Some(ProjectionUpdate::Unchanged)
    ));
    assert_eq!(session.projected_fresh_until(), None);

    revisions.set(14);
    let reconnect_attempt = issuer
        .deliver_verification_only(&permit, &disposition, 2, None, connection_id, &transport)
        .await
        .expect("reconnect obtains a newer production issuance");
    let (reconnect_text, reconnect) = receive_transport_event(
        &expected_frame_tx,
        &mut socket,
        CLIENT_BINDING_STATUS_SUB_ID,
        reconnect_attempt.event(),
    )
    .await;
    assert_current_projection(
        session.consume_text(&reconnect_text, now),
        &author,
        &epoch,
        disposition.expires_at(),
    );
    assert_eq!(
        tracker.accept(&reconnect, now),
        Ok(ClientBindingStatusUpdate::Accepted)
    );
    assert!(tracker.current_presentation(now).is_some());
    assert!(tracker
        .current_presentation(disposition.expires_at())
        .is_none());
    assert_eq!(tracker.high_water_revision(), Some(14));
    assert_clear(Some(session.expire(disposition.expires_at())));
    assert_eq!(session.projected_fresh_until(), None);

    tracker.change_scope(relay.public_key(), wrong_domain, author.public_key());
    assert_eq!(tracker.high_water_revision(), None);
    assert!(tracker.current_presentation(now).is_none());
    assert_eq!(
        tracker.accept(&reconnect, now),
        Err(ClientBindingStatusFoldError::InvalidStatus(
            ClientBindingStatusError::AuthorizationDomainMismatch
        ))
    );

    // Logout/restart starts with no projection. It does not synthesize a
    // profile-derived or NIP-85-derived fallback while awaiting a new status.
    let mut restarted =
        ClientBindingStatusTracker::new(relay.public_key(), domain, author.public_key());
    assert!(restarted.current_presentation(now).is_none());
    assert_eq!(restarted.high_water_revision(), None);
    let restarted_session =
        ClientBindingStatusSession::new(relay.public_key(), author.public_key(), epoch.clone());
    assert_eq!(restarted_session.connection_epoch(), &epoch);
    assert_eq!(restarted_session.projected_fresh_until(), None);

    assert_eq!(revisions.current_reads.load(Ordering::SeqCst), 5);
    assert_eq!(revisions.withdrawal_reads.load(Ordering::SeqCst), 1);
    {
        let observed_scopes = revisions
            .scopes
            .lock()
            .expect("synthetic revision scope lock");
        assert!(
            !observed_scopes.is_empty(),
            "issuer must read durable scope"
        );
        assert!(observed_scopes.iter().all(|scope| {
            scope.authorization_domain() == domain
                && scope.event_author_pubkey() == author.public_key()
        }));
    }

    connections.deregister(connection_id);
    drop(expected_frame_tx);
    let sent = server.await.expect("loopback relay task exits cleanly");
    assert!(
        sent >= 15,
        "non-vacuity: AUTH/bootstrap/status cases crossed queue-gated WebSocket"
    );
}

#[test]
fn presentation_gate_rejects_incomplete_review_evidence() {
    let incomplete = CompleteSyntheticApproval {
        reviewed_revision: "not-a-revision".to_string(),
    };
    assert!(matches!(
        ClientStatusPresentationPermit::from_complete_stack(&incomplete),
        Err(ClientStatusPresentationGateError::Incomplete)
    ));
}
