use super::*;
use buzz_core::import::{ImportParticipant, ImportSource, ImportSourceKind};
use buzz_core::kind::{KIND_IMPORT_PROPOSAL, KIND_STREAM_MESSAGE};

const MANIFEST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const CONVERSATION: &str = "2222222222222222222222222222222222222222222222222222222222222222";

/// Session caps high enough that only the cap test reaches them.
const LIMITS: import::ImportLimits = import::ImportLimits {
    max_events_per_participant: 50_000,
    max_events_per_session: 200_000,
    max_active_sessions_per_channel: 100,
    max_active_sessions_per_proposer: 100,
};

struct Fixture {
    db: Db,
    community: CommunityId,
    relay: Keys,
}

async fn fixture() -> Fixture {
    let db = Db::new(&crate::DbConfig {
        database_url: crate::test_support::database_url(),
        ..crate::DbConfig::default()
    })
    .await
    .unwrap();
    if std::env::var("BUZZ_TEST_SCHEMA_MODE").as_deref() != Ok("desired") {
        db.migrate().await.unwrap();
    }
    let community = CommunityId::from_uuid(Uuid::new_v4());
    sqlx::query("INSERT INTO communities (id, host) VALUES ($1, $2)")
        .bind(community.as_uuid())
        .bind(format!("import-{}.test", community.as_uuid()))
        .execute(&db.pool)
        .await
        .unwrap();
    Fixture {
        db,
        community,
        relay: Keys::generate(),
    }
}

async fn channel(f: &Fixture, channel_type: &str, members: &[(&Keys, &str)]) -> Uuid {
    let mut tx = crate::begin_community_write_transaction(
        &f.db.pool,
        f.community,
        crate::observability::WriterOperation::EventWrite,
    )
    .await
    .unwrap();
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO channels (id, community_id, name, channel_type, visibility, created_by) \
         VALUES ($1, $2, $3, $4::channel_type, 'private', $5)",
    )
    .bind(id)
    .bind(f.community.as_uuid())
    .bind(format!("c-{}", id.simple()))
    .bind(channel_type)
    .bind(members[0].0.public_key().to_bytes().as_slice())
    .execute(tx.conn())
    .await
    .unwrap();
    for (keys, role) in members {
        sqlx::query(
            "INSERT INTO channel_members (community_id, channel_id, pubkey, role) \
             VALUES ($1, $2, $3, $4::member_role)",
        )
        .bind(f.community.as_uuid())
        .bind(id)
        .bind(keys.public_key().to_bytes().as_slice())
        .bind(*role)
        .execute(tx.conn())
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    id
}

fn source_user(keys: &Keys) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(format!(
        "test:{}",
        keys.public_key().to_hex()
    )))
}

fn proposal_content(participants: &[(&Keys, u64, u64)], now: i64) -> ImportProposalContent {
    let mut participants: Vec<ImportParticipant> = participants
        .iter()
        .map(|(keys, messages, reactions)| ImportParticipant {
            pubkey: keys.public_key().to_hex(),
            source_user: source_user(keys),
            messages: *messages,
            reactions: *reactions,
        })
        .collect();
    participants.sort_by(|a, b| a.pubkey.cmp(&b.pubkey));
    ImportProposalContent {
        v: 1,
        source: ImportSource {
            source_type: "test".into(),
            conversation: CONVERSATION.into(),
            kind: ImportSourceKind::Dm,
            label: "Direct message".into(),
        },
        range: ImportRange {
            from: now - 10_000,
            to: now - 100,
        },
        participants,
        excluded_authors: 0,
        manifest: MANIFEST.into(),
        expires_at: now + 3_600,
    }
}

fn sign(keys: &Keys, kind: u32, content: &str, tags: Vec<Vec<String>>, created_at: i64) -> Event {
    let tags: Vec<Tag> = tags.into_iter().map(|t| Tag::parse(t).unwrap()).collect();
    // A proposal lists its proposer; the builder drops a self p tag unless told not to.
    EventBuilder::new(Kind::Custom(kind as u16), content)
        .tags(tags)
        .allow_self_tagging()
        .custom_created_at(Timestamp::from(created_at as u64))
        .sign_with_keys(keys)
        .unwrap()
}

fn proposal_event(keys: &Keys, channel: Uuid, content: &ImportProposalContent) -> Event {
    let mut tags = vec![vec!["h".to_string(), channel.to_string()]];
    for p in &content.participants {
        tags.push(vec!["p".into(), p.pubkey.clone()]);
    }
    tags.push(vec!["manifest".into(), content.manifest.clone()]);
    tags.push(vec![
        "import-source".into(),
        content.source.source_type.clone(),
    ]);
    sign(
        keys,
        KIND_IMPORT_PROPOSAL,
        &serde_json::to_string(content).unwrap(),
        tags,
        Utc::now().timestamp(),
    )
}

fn control(keys: &Keys, kind: u32, proposal: &Event, channel: Uuid, content: &str) -> Event {
    sign(
        keys,
        kind,
        content,
        vec![
            vec!["e".into(), proposal.id.to_hex()],
            vec!["h".into(), channel.to_string()],
        ],
        Utc::now().timestamp(),
    )
}

fn consent_event(keys: &Keys, proposal: &Event, channel: Uuid, content: &str) -> Event {
    sign(
        keys,
        42111,
        content,
        vec![
            vec!["e".into(), proposal.id.to_hex()],
            vec!["h".into(), channel.to_string()],
            vec!["manifest".into(), MANIFEST.into()],
            vec!["p".into(), proposal.pubkey.to_hex()],
        ],
        Utc::now().timestamp(),
    )
}

fn imported(keys: &Keys, proposal: &Event, channel: Uuid, created_at: i64, n: u32) -> Event {
    sign(
        keys,
        KIND_STREAM_MESSAGE,
        &format!("message {n}"),
        vec![
            vec!["h".into(), channel.to_string()],
            vec!["import".into(), proposal.id.to_hex()],
            vec!["source".into(), "test".into(), format!("ref-{n}")],
        ],
        created_at,
    )
}

fn tags_of(event: &Event) -> ImportTags {
    import::parse_import_tags(event).unwrap().unwrap()
}

fn snapshot_of(stored: &StoredEvent) -> ImportSessionSnapshotContent {
    serde_json::from_str(&stored.event.content).unwrap()
}

async fn applied(outcome: Result<ImportControlOutcome>) -> ImportSessionSnapshotContent {
    match outcome.unwrap() {
        ImportControlOutcome::Applied { snapshot, .. } => snapshot_of(&snapshot),
        other => panic!("expected Applied, got {other:?}"),
    }
}

async fn consent(
    f: &Fixture,
    keys: &Keys,
    proposal: &Event,
    channel: Uuid,
    decision: ImportDecision,
    source_user: &str,
) -> Result<ImportControlOutcome> {
    let content =
        serde_json::json!({"v": 1, "decision": decision.as_str(), "source_user": source_user});
    let event = consent_event(keys, proposal, channel, &content.to_string());
    f.db.record_import_consent(
        f.community,
        &event,
        proposal.id.as_bytes(),
        channel,
        MANIFEST,
        decision,
        source_user,
        &f.relay,
    )
    .await
}

/// Move every active session in the fixture's community past its warm-up, so
/// a test can import right after activation. The warm-up itself is covered by
/// `imports_wait_for_accepts_after`. Rows already moved are not touched, so
/// this never waits on a session row lock.
async fn skip_warmup(f: &Fixture) {
    sqlx::query(
        "UPDATE import_sessions SET accepts_after = 0 \
         WHERE community_id = $1 AND accepts_after > 0",
    )
    .bind(f.community.as_uuid())
    .execute(&f.db.pool)
    .await
    .unwrap();
}

async fn insert(f: &Fixture, event: &Event, channel: Uuid) -> ImportInsertOutcome {
    skip_warmup(f).await;
    f.db.insert_import_event(
        f.community,
        event,
        &tags_of(event),
        channel,
        None,
        None,
        &f.relay,
    )
    .await
    .unwrap()
}

fn rejected_reason(outcome: &ImportInsertOutcome) -> Option<&'static str> {
    match outcome {
        ImportInsertOutcome::Rejected { reason, .. } => Some(reason),
        _ => None,
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn dm_session_lifecycle_enforces_consent_range_and_counts() {
    let f = fixture().await;
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let dm = channel(&f, "dm", &[(&alice, "member"), (&bob, "member")]).await;
    let now = Utc::now().timestamp();
    let content = proposal_content(&[(&alice, 2, 0), (&bob, 1, 0)], now);
    let proposal = proposal_event(&alice, dm, &content);

    let snapshot = applied(
        f.db.create_import_session(
            f.community,
            &proposal,
            &content,
            dm,
            content.expires_at,
            LIMITS,
            &f.relay,
        )
        .await,
    )
    .await;
    assert_eq!(snapshot.status, ImportSessionStatus::Proposed);
    assert_eq!(
        snapshot.consents.get(&alice.public_key().to_hex()),
        Some(&ImportDecision::Accept)
    );

    // Bob has not consented: the session is not active yet.
    let early = imported(&alice, &proposal, dm, now - 5_000, 1);
    assert_eq!(
        rejected_reason(&insert(&f, &early, dm).await),
        Some(reason::SESSION_NOT_ACTIVE)
    );

    // A consent bound to the wrong identity is refused.
    let wrong = consent(
        &f,
        &bob,
        &proposal,
        dm,
        ImportDecision::Accept,
        CONVERSATION,
    )
    .await
    .unwrap();
    assert!(matches!(
        wrong,
        ImportControlOutcome::Rejected(reason::CONSENT_MISMATCH)
    ));

    let snapshot = applied(
        consent(
            &f,
            &bob,
            &proposal,
            dm,
            ImportDecision::Accept,
            &source_user(&bob),
        )
        .await,
    )
    .await;
    assert_eq!(snapshot.status, ImportSessionStatus::Active);

    assert!(matches!(
        insert(&f, &early, dm).await,
        ImportInsertOutcome::Inserted { .. }
    ));
    let outside = imported(&alice, &proposal, dm, now - 20_000, 2);
    assert_eq!(
        rejected_reason(&insert(&f, &outside, dm).await),
        Some(reason::TIMESTAMP_OUT_OF_RANGE)
    );
    let second = imported(&alice, &proposal, dm, now - 4_000, 3);
    assert!(matches!(
        insert(&f, &second, dm).await,
        ImportInsertOutcome::Inserted { .. }
    ));
    let third = imported(&alice, &proposal, dm, now - 3_000, 4);
    assert_eq!(
        rejected_reason(&insert(&f, &third, dm).await),
        Some(reason::COUNT_EXCEEDED)
    );
    // A resend after the count is used up is a duplicate, not a refusal, and
    // the preflight read agrees.
    assert!(matches!(
        insert(&f, &second, dm).await,
        ImportInsertOutcome::Duplicate
    ));
    let preflight =
        f.db.import_preflight(f.community, proposal.id.as_bytes(), &second)
            .await
            .unwrap()
            .expect("session");
    assert!(preflight.already_stored);
    let stranger = Keys::generate();
    let foreign = imported(&stranger, &proposal, dm, now - 3_000, 5);
    assert_eq!(
        rejected_reason(&insert(&f, &foreign, dm).await),
        Some(reason::NOT_CONSENTING)
    );

    // Imported events never create mention rows.
    let mentions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_mentions WHERE community_id = $1 AND event_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(early.id.as_bytes().as_slice())
    .fetch_one(&f.db.pool)
    .await
    .unwrap();
    assert_eq!(mentions, 0);

    // Bob declines after accepting: a DM session ends.
    let snapshot = applied(
        consent(
            &f,
            &bob,
            &proposal,
            dm,
            ImportDecision::Decline,
            &source_user(&bob),
        )
        .await,
    )
    .await;
    assert_eq!(snapshot.status, ImportSessionStatus::Declined);
    let after = imported(&bob, &proposal, dm, now - 2_000, 6);
    assert_eq!(
        rejected_reason(&insert(&f, &after, dm).await),
        Some(reason::SESSION_NOT_ACTIVE)
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn dm_participants_must_equal_members() {
    let f = fixture().await;
    let (alice, bob, carol) = (Keys::generate(), Keys::generate(), Keys::generate());
    let dm = channel(
        &f,
        "dm",
        &[(&alice, "member"), (&bob, "member"), (&carol, "member")],
    )
    .await;
    let now = Utc::now().timestamp();
    let content = proposal_content(&[(&alice, 1, 0), (&bob, 1, 0)], now);
    let proposal = proposal_event(&alice, dm, &content);
    let outcome =
        f.db.create_import_session(
            f.community,
            &proposal,
            &content,
            dm,
            content.expires_at,
            LIMITS,
            &f.relay,
        )
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        ImportControlOutcome::Rejected(reason::DM_PARTICIPANTS_MISMATCH)
    ));

    let outsider = Keys::generate();
    let content = proposal_content(&[(&alice, 1, 0), (&outsider, 1, 0)], now);
    let proposal = proposal_event(&alice, dm, &content);
    let outcome =
        f.db.create_import_session(
            f.community,
            &proposal,
            &content,
            dm,
            content.expires_at,
            LIMITS,
            &f.relay,
        )
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        ImportControlOutcome::Rejected(reason::PARTICIPANT_NOT_MEMBER)
    ));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn channel_session_requires_admin_and_withdraw_stops_acceptance() {
    let f = fixture().await;
    let (owner, alice, bob) = (Keys::generate(), Keys::generate(), Keys::generate());
    let chan = channel(
        &f,
        "stream",
        &[(&owner, "owner"), (&alice, "member"), (&bob, "member")],
    )
    .await;
    let now = Utc::now().timestamp();
    let content = proposal_content(&[(&alice, 5, 1), (&bob, 5, 0)], now);
    let proposal = proposal_event(&alice, chan, &content);
    let snapshot = applied(
        f.db.create_import_session(
            f.community,
            &proposal,
            &content,
            chan,
            content.expires_at,
            LIMITS,
            &f.relay,
        )
        .await,
    )
    .await;
    assert!(snapshot.required.contains(&"admin".to_string()));

    applied(
        consent(
            &f,
            &bob,
            &proposal,
            chan,
            ImportDecision::Accept,
            &source_user(&bob),
        )
        .await,
    )
    .await;
    // A non-admin cannot give the admin consent.
    let refused = consent(&f, &bob, &proposal, chan, ImportDecision::Accept, "admin")
        .await
        .unwrap();
    assert!(matches!(
        refused,
        ImportControlOutcome::Rejected(reason::NOT_PARTICIPANT)
    ));
    let snapshot =
        applied(consent(&f, &owner, &proposal, chan, ImportDecision::Accept, "admin").await).await;
    assert_eq!(snapshot.status, ImportSessionStatus::Active);

    let message = imported(&alice, &proposal, chan, now - 5_000, 1);
    assert!(matches!(
        insert(&f, &message, chan).await,
        ImportInsertOutcome::Inserted { .. }
    ));

    // A reaction on a non-imported target is refused.
    let plain = sign(
        &bob,
        KIND_STREAM_MESSAGE,
        "live",
        vec![vec!["h".into(), chan.to_string()]],
        now,
    );
    crate::event::insert_event_with_thread_metadata(
        &f.db.pool,
        f.community,
        &plain,
        Some(chan),
        None,
    )
    .await
    .unwrap();
    let reaction = sign(
        &alice,
        KIND_REACTION,
        "+",
        vec![
            vec!["h".into(), chan.to_string()],
            vec!["e".into(), plain.id.to_hex()],
            vec!["import".into(), proposal.id.to_hex()],
            vec!["source".into(), "test".into(), "ref-1".into()],
        ],
        now - 4_000,
    );
    let actor = alice.public_key().to_bytes();
    skip_warmup(&f).await;
    let outcome =
        f.db.insert_import_event(
            f.community,
            &reaction,
            &tags_of(&reaction),
            chan,
            None,
            Some(ImportReaction {
                target_event_id: plain.id.as_bytes(),
                actor_pubkey: &actor,
                emoji: "+",
            }),
            &f.relay,
        )
        .await
        .unwrap();
    assert_eq!(
        rejected_reason(&outcome),
        Some(reason::REACTION_TARGET_NOT_IMPORTED)
    );

    let withdraw = control(&bob, 42112, &proposal, chan, r#"{"v":1}"#);
    let snapshot = applied(
        f.db.withdraw_import_session(
            f.community,
            &withdraw,
            proposal.id.as_bytes(),
            chan,
            &f.relay,
        )
        .await,
    )
    .await;
    assert_eq!(snapshot.status, ImportSessionStatus::Withdrawn);
    let late = imported(&alice, &proposal, chan, now - 3_000, 2);
    assert_eq!(
        rejected_reason(&insert(&f, &late, chan).await),
        Some(reason::SESSION_NOT_ACTIVE)
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn completion_and_expiry_move_the_session() {
    let f = fixture().await;
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let dm = channel(&f, "dm", &[(&alice, "member"), (&bob, "member")]).await;
    let now = Utc::now().timestamp();
    let content = proposal_content(&[(&alice, 1, 0), (&bob, 0, 0)], now);
    let proposal = proposal_event(&alice, dm, &content);
    applied(
        f.db.create_import_session(
            f.community,
            &proposal,
            &content,
            dm,
            content.expires_at,
            LIMITS,
            &f.relay,
        )
        .await,
    )
    .await;
    applied(
        consent(
            &f,
            &bob,
            &proposal,
            dm,
            ImportDecision::Accept,
            &source_user(&bob),
        )
        .await,
    )
    .await;
    let done = r#"{"v":1,"published":{"messages":0,"reactions":0}}"#;
    let snapshot = applied(
        f.db.complete_import_participant(
            f.community,
            &control(&alice, 42113, &proposal, dm, done),
            proposal.id.as_bytes(),
            dm,
            &f.relay,
        )
        .await,
    )
    .await;
    assert_eq!(snapshot.status, ImportSessionStatus::Active);
    assert_eq!(snapshot.completed, vec![alice.public_key().to_hex()]);
    let snapshot = applied(
        f.db.complete_import_participant(
            f.community,
            &control(&bob, 42113, &proposal, dm, done),
            proposal.id.as_bytes(),
            dm,
            &f.relay,
        )
        .await,
    )
    .await;
    assert_eq!(snapshot.status, ImportSessionStatus::Completed);

    // A session past its expiry is marked expired on the next read.
    let content = proposal_content(&[(&alice, 1, 0), (&bob, 1, 0)], now);
    let proposal = proposal_event(&alice, dm, &content);
    applied(
        f.db.create_import_session(
            f.community,
            &proposal,
            &content,
            dm,
            now + 1,
            LIMITS,
            &f.relay,
        )
        .await,
    )
    .await;
    sqlx::query(
        "UPDATE import_sessions SET expires_at = $3 WHERE community_id = $1 AND proposal_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(proposal.id.as_bytes().as_slice())
    .bind(now - 1)
    .execute(&f.db.pool)
    .await
    .unwrap();
    let outcome = insert(&f, &imported(&alice, &proposal, dm, now - 5_000, 1), dm).await;
    match outcome {
        ImportInsertOutcome::Rejected {
            reason,
            snapshot: Some(snapshot),
        } => {
            assert_eq!(reason, reason::SESSION_NOT_ACTIVE);
            assert_eq!(snapshot_of(&snapshot).status, ImportSessionStatus::Expired);
        }
        other => panic!("expected expiry rejection, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn import_writes_are_refused_once_deletion_starts() {
    let f = fixture().await;
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let dm = channel(&f, "dm", &[(&alice, "member"), (&bob, "member")]).await;
    let now = Utc::now().timestamp();
    let content = proposal_content(&[(&alice, 1, 0), (&bob, 1, 0)], now);
    let proposal = proposal_event(&alice, dm, &content);
    crate::test_support::quiesce_community_for_tests(&f.db.pool, f.community).await;
    let error =
        f.db.create_import_session(
            f.community,
            &proposal,
            &content,
            dm,
            content.expires_at,
            LIMITS,
            &f.relay,
        )
        .await
        .expect_err("quiescing community must refuse import writes");
    assert!(
        crate::test_support::is_admission_rejection(&error),
        "{error}"
    );
}

async fn make_agent(f: &Fixture, agent: &Keys, owner: &Keys) {
    for (pubkey, owner) in [
        (owner.public_key().to_bytes(), None),
        (
            agent.public_key().to_bytes(),
            Some(owner.public_key().to_bytes()),
        ),
    ] {
        sqlx::query(
            "INSERT INTO users (community_id, pubkey, agent_owner_pubkey) VALUES ($1, $2, $3) \
             ON CONFLICT (community_id, pubkey) DO UPDATE \
             SET agent_owner_pubkey = EXCLUDED.agent_owner_pubkey",
        )
        .bind(f.community.as_uuid())
        .bind(pubkey.as_slice())
        .bind(owner.as_ref().map(|o| o.as_slice()))
        .execute(&f.db.pool)
        .await
        .unwrap();
    }
}

async fn add_member(f: &Fixture, channel: Uuid, keys: &Keys) {
    sqlx::query(
        "INSERT INTO channel_members (community_id, channel_id, pubkey, role) \
         VALUES ($1, $2, $3, 'member')",
    )
    .bind(f.community.as_uuid())
    .bind(channel)
    .bind(keys.public_key().to_bytes().as_slice())
    .execute(&f.db.pool)
    .await
    .unwrap();
}

async fn propose(
    f: &Fixture,
    proposer: &Keys,
    channel: Uuid,
    participants: &[(&Keys, u64, u64)],
) -> (Event, Result<ImportControlOutcome>) {
    let content = proposal_content(participants, Utc::now().timestamp());
    let proposal = proposal_event(proposer, channel, &content);
    let outcome =
        f.db.create_import_session(
            f.community,
            &proposal,
            &content,
            channel,
            content.expires_at,
            LIMITS,
            &f.relay,
        )
        .await;
    (proposal, outcome)
}

async fn accept(f: &Fixture, keys: &Keys, proposal: &Event, channel: Uuid) {
    applied(
        consent(
            f,
            keys,
            proposal,
            channel,
            ImportDecision::Accept,
            &source_user(keys),
        )
        .await,
    )
    .await;
}

fn imported_with_source(
    keys: &Keys,
    proposal: &Event,
    channel: Uuid,
    created_at: i64,
    source_type: &str,
) -> Event {
    sign(
        keys,
        KIND_STREAM_MESSAGE,
        "message",
        vec![
            vec!["h".into(), channel.to_string()],
            vec!["import".into(), proposal.id.to_hex()],
            vec!["source".into(), source_type.into(), "ref".into()],
        ],
        created_at,
    )
}

fn imported_reaction(
    keys: &Keys,
    proposal: &Event,
    channel: Uuid,
    target: &Event,
    created_at: i64,
) -> Event {
    sign(
        keys,
        KIND_REACTION,
        "+",
        vec![
            vec!["h".into(), channel.to_string()],
            vec!["e".into(), target.id.to_hex()],
            vec!["import".into(), proposal.id.to_hex()],
            vec!["source".into(), "test".into(), "target".into()],
        ],
        created_at,
    )
}

async fn insert_reaction(
    f: &Fixture,
    event: &Event,
    channel: Uuid,
    target: &Event,
) -> ImportInsertOutcome {
    let actor = event.pubkey.to_bytes();
    skip_warmup(f).await;
    f.db.insert_import_event(
        f.community,
        event,
        &tags_of(event),
        channel,
        None,
        Some(ImportReaction {
            target_event_id: target.id.as_bytes(),
            actor_pubkey: &actor,
            emoji: "+",
        }),
        &f.relay,
    )
    .await
    .unwrap()
}

fn is_inserted(outcome: &ImportInsertOutcome) -> bool {
    matches!(outcome, ImportInsertOutcome::Inserted { .. })
}

fn control_rejection(outcome: Result<ImportControlOutcome>) -> &'static str {
    match outcome.unwrap() {
        ImportControlOutcome::Rejected(reason) => reason,
        other => panic!("expected a rejection, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn imported_history_never_reaches_agents() {
    let f = fixture().await;
    let (owner, alice, bot) = (Keys::generate(), Keys::generate(), Keys::generate());
    make_agent(&f, &bot, &owner).await;

    // An agent cannot be a participant, in a channel or in a DM.
    let private = channel(
        &f,
        "stream",
        &[(&owner, "owner"), (&alice, "member"), (&bot, "member")],
    )
    .await;
    let (_, outcome) = propose(&f, &owner, private, &[(&owner, 1, 0), (&bot, 1, 0)]).await;
    assert_eq!(
        control_rejection(outcome),
        reason::PARTICIPANTS_INCLUDE_AGENTS
    );
    let dm = channel(&f, "dm", &[(&owner, "member"), (&bot, "member")]).await;
    let (_, outcome) = propose(&f, &owner, dm, &[(&owner, 1, 0), (&bot, 1, 0)]).await;
    assert_eq!(
        control_rejection(outcome),
        reason::PARTICIPANTS_INCLUDE_AGENTS
    );

    // A private destination with an agent member is refused; an open one is not.
    let (_, outcome) = propose(&f, &owner, private, &[(&owner, 1, 0), (&alice, 1, 0)]).await;
    assert_eq!(control_rejection(outcome), reason::DESTINATION_HAS_AGENTS);
    sqlx::query("UPDATE channels SET visibility = 'open' WHERE community_id = $1 AND id = $2")
        .bind(f.community.as_uuid())
        .bind(private)
        .execute(&f.db.pool)
        .await
        .unwrap();
    let (_, outcome) = propose(&f, &owner, private, &[(&owner, 1, 0), (&alice, 1, 0)]).await;
    assert!(matches!(
        outcome.unwrap(),
        ImportControlOutcome::Applied { .. }
    ));

    // An agent that joins a private destination after activation stops imports.
    let later = channel(&f, "stream", &[(&owner, "owner"), (&alice, "member")]).await;
    let (proposal, outcome) = propose(&f, &owner, later, &[(&owner, 2, 0), (&alice, 1, 0)]).await;
    applied(outcome).await;
    accept(&f, &alice, &proposal, later).await;
    let now = Utc::now().timestamp();
    assert!(is_inserted(
        &insert(
            &f,
            &imported(&owner, &proposal, later, now - 5_000, 1),
            later
        )
        .await
    ));
    add_member(&f, later, &bot).await;
    assert_eq!(
        rejected_reason(
            &insert(
                &f,
                &imported(&owner, &proposal, later, now - 4_000, 2),
                later
            )
            .await
        ),
        Some(reason::DESTINATION_HAS_AGENTS)
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn dm_membership_change_after_activation_stops_imports() {
    let f = fixture().await;
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let dm = channel(&f, "dm", &[(&alice, "member"), (&bob, "member")]).await;
    let (proposal, outcome) = propose(&f, &alice, dm, &[(&alice, 2, 0), (&bob, 0, 0)]).await;
    applied(outcome).await;
    accept(&f, &bob, &proposal, dm).await;
    let now = Utc::now().timestamp();
    assert!(is_inserted(
        &insert(&f, &imported(&alice, &proposal, dm, now - 5_000, 1), dm).await
    ));
    // A member added to the DM (kind:9000 allows it) stops imports.
    let carol = Keys::generate();
    add_member(&f, dm, &carol).await;
    assert_eq!(
        rejected_reason(&insert(&f, &imported(&alice, &proposal, dm, now - 4_500, 3), dm).await),
        Some(reason::DM_PARTICIPANTS_MISMATCH)
    );
    sqlx::query(
        "DELETE FROM channel_members WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
    )
    .bind(f.community.as_uuid())
    .bind(dm)
    .bind(carol.public_key().to_bytes().as_slice())
    .execute(&f.db.pool)
    .await
    .unwrap();
    // So does a participant leaving it.
    sqlx::query(
        "UPDATE channel_members SET removed_at = now() \
         WHERE community_id = $1 AND channel_id = $2 AND pubkey = $3",
    )
    .bind(f.community.as_uuid())
    .bind(dm)
    .bind(bob.public_key().to_bytes().as_slice())
    .execute(&f.db.pool)
    .await
    .unwrap();
    assert_eq!(
        rejected_reason(&insert(&f, &imported(&alice, &proposal, dm, now - 4_000, 2), dm).await),
        Some(reason::DM_PARTICIPANTS_MISMATCH)
    );
    // The participant who left can still finish.
    let done = r#"{"v":1,"published":{"messages":0,"reactions":0}}"#;
    let snapshot = applied(
        f.db.complete_import_participant(
            f.community,
            &control(&bob, 42113, &proposal, dm, done),
            proposal.id.as_bytes(),
            dm,
            &f.relay,
        )
        .await,
    )
    .await;
    assert_eq!(snapshot.completed, vec![bob.public_key().to_hex()]);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn concurrent_inserts_never_exceed_the_declared_count() {
    let f = fixture().await;
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let dm = channel(&f, "dm", &[(&alice, "member"), (&bob, "member")]).await;
    let (proposal, outcome) = propose(&f, &alice, dm, &[(&alice, 2, 0), (&bob, 0, 0)]).await;
    applied(outcome).await;
    accept(&f, &bob, &proposal, dm).await;
    let now = Utc::now().timestamp();
    assert!(is_inserted(
        &insert(&f, &imported(&alice, &proposal, dm, now - 5_000, 1), dm).await
    ));
    // One message remains; two different events race for it.
    let (a, b) = (
        imported(&alice, &proposal, dm, now - 4_000, 2),
        imported(&alice, &proposal, dm, now - 3_000, 3),
    );
    // Hold the session row so both inserts are in flight before either can
    // read the counter.
    let mut blocker = f.db.pool.begin().await.unwrap();
    sqlx::query(
        "SELECT 1 FROM import_sessions WHERE community_id = $1 AND proposal_id = $2 FOR UPDATE",
    )
    .bind(f.community.as_uuid())
    .bind(proposal.id.as_bytes().as_slice())
    .execute(&mut *blocker)
    .await
    .unwrap();
    let release = async {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        blocker.commit().await.unwrap();
    };
    let (first, second, ()) = tokio::join!(insert(&f, &a, dm), insert(&f, &b, dm), release);
    let stored = [&first, &second]
        .into_iter()
        .filter(|o| is_inserted(o))
        .count();
    assert_eq!(stored, 1, "{first:?} / {second:?}");
    let refused = if is_inserted(&first) { &second } else { &first };
    assert_eq!(rejected_reason(refused), Some(reason::COUNT_EXCEEDED));
    let published: i64 = sqlx::query_scalar(
        "SELECT published_messages FROM import_session_participants \
         WHERE community_id = $1 AND proposal_id = $2 AND pubkey = $3",
    )
    .bind(f.community.as_uuid())
    .bind(proposal.id.as_bytes().as_slice())
    .bind(alice.public_key().to_bytes().as_slice())
    .fetch_one(&f.db.pool)
    .await
    .unwrap();
    assert_eq!(published, 2);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn imported_events_must_match_their_session() {
    let f = fixture().await;
    let (owner, alice, carol) = (Keys::generate(), Keys::generate(), Keys::generate());
    let chan = channel(
        &f,
        "stream",
        &[(&owner, "owner"), (&alice, "member"), (&carol, "member")],
    )
    .await;
    let other = channel(&f, "stream", &[(&owner, "owner"), (&alice, "member")]).await;
    let now = Utc::now().timestamp();

    // Session A: owner and alice. Session B in the same channel: owner and carol.
    // Session C in another channel: owner and alice.
    let (a, outcome) = propose(&f, &owner, chan, &[(&owner, 1, 0), (&alice, 2, 3)]).await;
    applied(outcome).await;
    accept(&f, &alice, &a, chan).await;
    let (b, outcome) = propose(&f, &owner, chan, &[(&owner, 1, 0), (&carol, 1, 0)]).await;
    applied(outcome).await;
    accept(&f, &carol, &b, chan).await;
    let (c, outcome) = propose(&f, &owner, other, &[(&owner, 1, 0), (&alice, 1, 0)]).await;
    applied(outcome).await;
    accept(&f, &alice, &c, other).await;

    let in_a = imported(&alice, &a, chan, now - 9_000, 1);
    assert!(is_inserted(&insert(&f, &in_a, chan).await));
    let in_b = imported(&carol, &b, chan, now - 9_000, 2);
    assert!(is_inserted(&insert(&f, &in_b, chan).await));
    let in_c = imported(&alice, &c, other, now - 9_000, 3);
    assert!(is_inserted(&insert(&f, &in_c, other).await));

    // The session's channel, its source type and its participants bind.
    assert_eq!(
        rejected_reason(&insert(&f, &imported(&alice, &a, chan, now - 8_000, 4), other).await),
        Some(reason::CHANNEL_MISMATCH)
    );
    assert_eq!(
        rejected_reason(
            &insert(
                &f,
                &imported_with_source(&alice, &a, chan, now - 8_000, "other"),
                chan
            )
            .await
        ),
        Some(reason::SOURCE_TYPE_MISMATCH)
    );
    assert_eq!(
        rejected_reason(&insert(&f, &imported(&carol, &a, chan, now - 8_000, 5), chan).await),
        Some(reason::NOT_CONSENTING)
    );

    // A reaction target imported under another proposal, or living in
    // another channel, is refused.
    let on_b = imported_reaction(&alice, &a, chan, &in_b, now - 7_000);
    assert_eq!(
        rejected_reason(&insert_reaction(&f, &on_b, chan, &in_b).await),
        Some(reason::REACTION_TARGET_NOT_IMPORTED)
    );
    let on_c = imported_reaction(&alice, &a, chan, &in_c, now - 7_000);
    assert_eq!(
        rejected_reason(&insert_reaction(&f, &on_c, chan, &in_c).await),
        Some(reason::REACTION_TARGET_NOT_IMPORTED)
    );

    // A second imported reaction with the same actor, target and emoji
    // conflicts; it is not reported as a duplicate.
    let first = imported_reaction(&alice, &a, chan, &in_a, now - 7_000);
    assert!(is_inserted(&insert_reaction(&f, &first, chan, &in_a).await));
    assert!(matches!(
        insert_reaction(&f, &first, chan, &in_a).await,
        ImportInsertOutcome::Duplicate
    ));
    let again = imported_reaction(&alice, &a, chan, &in_a, now - 6_000);
    assert_eq!(
        rejected_reason(&insert_reaction(&f, &again, chan, &in_a).await),
        Some(reason::REACTION_CONFLICT)
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn proposer_only_session_activates_and_admin_decline_withdraws() {
    let f = fixture().await;
    let (owner, alice) = (Keys::generate(), Keys::generate());
    let chan = channel(&f, "stream", &[(&owner, "owner"), (&alice, "member")]).await;

    // An owner importing only their own history needs nobody else.
    let (_, outcome) = propose(&f, &owner, chan, &[(&owner, 1, 0)]).await;
    assert_eq!(applied(outcome).await.status, ImportSessionStatus::Active);

    // A member's session needs the admin; an admin decline after activation
    // withdraws it.
    let (proposal, outcome) = propose(&f, &alice, chan, &[(&alice, 2, 0)]).await;
    assert_eq!(applied(outcome).await.status, ImportSessionStatus::Proposed);
    let snapshot =
        applied(consent(&f, &owner, &proposal, chan, ImportDecision::Accept, "admin").await).await;
    assert_eq!(snapshot.status, ImportSessionStatus::Active);
    let now = Utc::now().timestamp();
    assert!(is_inserted(
        &insert(&f, &imported(&alice, &proposal, chan, now - 5_000, 1), chan).await
    ));
    let snapshot = applied(
        consent(
            &f,
            &owner,
            &proposal,
            chan,
            ImportDecision::Decline,
            "admin",
        )
        .await,
    )
    .await;
    assert_eq!(snapshot.status, ImportSessionStatus::Withdrawn);
    assert_eq!(
        rejected_reason(
            &insert(&f, &imported(&alice, &proposal, chan, now - 4_000, 2), chan).await
        ),
        Some(reason::SESSION_NOT_ACTIVE)
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn control_events_mention_participants_and_resend_as_duplicates() {
    let f = fixture().await;
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let dm = channel(&f, "dm", &[(&alice, "member"), (&bob, "member")]).await;
    let (proposal, outcome) = propose(&f, &alice, dm, &[(&alice, 1, 0), (&bob, 1, 0)]).await;
    applied(outcome).await;

    // The proposal is live activity: each participant gets a mention row.
    let mentioned: Vec<String> = sqlx::query_scalar(
        "SELECT pubkey_hex FROM event_mentions \
         WHERE community_id = $1 AND event_id = $2 ORDER BY pubkey_hex",
    )
    .bind(f.community.as_uuid())
    .bind(proposal.id.as_bytes().as_slice())
    .fetch_all(&f.db.pool)
    .await
    .unwrap();
    let mut expected = vec![alice.public_key().to_hex(), bob.public_key().to_hex()];
    expected.sort();
    assert_eq!(mentioned, expected);
    let feed =
        f.db.query_feed_mentions(f.community, &bob.public_key().to_bytes(), &[dm], None, 10)
            .await
            .unwrap();
    assert!(
        feed.iter().any(|row| row.event.id == proposal.id),
        "a waiting proposal must reach the participant's mentions feed"
    );

    // A consent must name the proposer in its single p tag.
    let content = serde_json::json!({
        "v": 1, "decision": "accept", "source_user": source_user(&bob)
    })
    .to_string();
    let record = |event: Event| {
        let db = &f.db;
        let relay = &f.relay;
        let community = f.community;
        let proposal_id = proposal.id;
        let bob_source = source_user(&bob);
        async move {
            db.record_import_consent(
                community,
                &event,
                proposal_id.as_bytes(),
                dm,
                MANIFEST,
                ImportDecision::Accept,
                &bob_source,
                relay,
            )
            .await
        }
    };
    let wrong_p = sign(
        &bob,
        42111,
        &content,
        vec![
            vec!["e".into(), proposal.id.to_hex()],
            vec!["h".into(), dm.to_string()],
            vec!["manifest".into(), MANIFEST.into()],
            vec!["p".into(), bob.public_key().to_hex()],
        ],
        Utc::now().timestamp(),
    );
    assert_eq!(
        control_rejection(record(wrong_p).await),
        reason::CONSENT_MISMATCH
    );

    // A resend after expiry is a duplicate, not a status rejection.
    let good = consent_event(&bob, &proposal, dm, &content);
    applied(record(good.clone()).await).await;
    sqlx::query(
        "UPDATE import_sessions SET expires_at = $3 WHERE community_id = $1 AND proposal_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(proposal.id.as_bytes().as_slice())
    .bind(Utc::now().timestamp() - 1)
    .execute(&f.db.pool)
    .await
    .unwrap();
    assert!(matches!(
        record(good).await.unwrap(),
        ImportControlOutcome::Duplicate
    ));
}

async fn covered(f: &Fixture, scope: CoveredScope<'_>, reason: &'static str) -> bool {
    let tx = f.db.pool.begin().await.unwrap();
    let entry = crate::runtime::replica_fence::TokenEntry {
        token: 1,
        committed_at: std::time::Instant::now(),
        fence_wall: Utc::now(),
    };
    let decision = crate::RouteDecision::Replica(tx, entry, reason);
    matches!(
        f.db.guard_covered_route("test", f.community, scope, decision)
            .await,
        crate::RouteDecision::Replica(..)
    )
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn covered_reads_go_to_the_writer_while_a_channel_imports() {
    let f = fixture().await;
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let dm = channel(&f, "dm", &[(&alice, "member"), (&bob, "member")]).await;
    let quiet = channel(&f, "dm", &[(&alice, "member"), (&bob, "member")]).await;
    let live = sign(
        &alice,
        KIND_STREAM_MESSAGE,
        "live",
        vec![vec!["h".into(), dm.to_string()]],
        Utc::now().timestamp(),
    );
    crate::event::insert_event_with_thread_metadata(&f.db.pool, f.community, &live, Some(dm), None)
        .await
        .unwrap();
    let missing = [7u8; 32];
    let root = CoveredScope::ThreadRoot(live.id.as_bytes());

    // A proposed session does not admit imports yet.
    let (proposal, outcome) = propose(&f, &alice, dm, &[(&alice, 1, 0), (&bob, 0, 0)]).await;
    applied(outcome).await;
    assert!(covered(&f, CoveredScope::Channel(dm), "covered").await);
    assert!(covered(&f, CoveredScope::ThreadRoot(&missing), "covered").await);

    accept(&f, &bob, &proposal, dm).await;
    assert!(!covered(&f, CoveredScope::Channel(dm), "covered").await);
    assert!(!covered(&f, root, "covered").await);
    // A root the snapshot lacks keeps the replica: the thread reader re-runs
    // the short page it gets on the writer.
    assert!(covered(&f, CoveredScope::ThreadRoot(&missing), "covered").await);
    // Other channels, and bounded reads, keep the replica.
    assert!(covered(&f, CoveredScope::Channel(quiet), "covered").await);
    assert!(covered(&f, CoveredScope::Channel(dm), "fresh").await);

    // A terminal session no longer holds the channel.
    let withdraw = control(&bob, 42112, &proposal, dm, "");
    applied(
        f.db.withdraw_import_session(f.community, &withdraw, proposal.id.as_bytes(), dm, &f.relay)
            .await,
    )
    .await;
    assert!(covered(&f, CoveredScope::Channel(dm), "covered").await);
    assert!(covered(&f, CoveredScope::ThreadRoot(live.id.as_bytes()), "covered").await);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn expired_sessions_stop_pinning_their_channel() {
    let f = fixture().await;
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let dm = channel(&f, "dm", &[(&alice, "member"), (&bob, "member")]).await;
    let (proposal, outcome) = propose(&f, &alice, dm, &[(&alice, 1, 0), (&bob, 0, 0)]).await;
    applied(outcome).await;
    accept(&f, &bob, &proposal, dm).await;
    let expire = |at: i64| {
        sqlx::query(
            "UPDATE import_sessions SET expires_at = $3 \
             WHERE community_id = $1 AND proposal_id = $2",
        )
        .bind(f.community.as_uuid())
        .bind(proposal.id.as_bytes().as_slice())
        .bind(at)
        .execute(&f.db.pool)
    };
    let now = Utc::now().timestamp();

    // Just past expiry the status is still stored as active and a lagging
    // snapshot may lack the last imports, so the channel stays on the writer.
    expire(now - 10).await.unwrap();
    assert!(!covered(&f, CoveredScope::Channel(dm), "covered").await);
    // Past expiry plus the warm-up span, it no longer pins the channel.
    expire(now - IMPORT_ACCEPT_DELAY_SECS - 10).await.unwrap();
    assert!(covered(&f, CoveredScope::Channel(dm), "covered").await);
}

/// The snapshot's `accepts_after` tag value.
fn accepts_after_tag(stored: &StoredEvent) -> Option<i64> {
    stored.event.tags.iter().find_map(|tag| {
        let slice = tag.as_slice();
        (slice.first().map(String::as_str) == Some(import::ACCEPTS_AFTER_TAG))
            .then(|| slice.get(1).and_then(|v| v.parse().ok()))
            .flatten()
    })
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn imports_wait_for_accepts_after() {
    let f = fixture().await;
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let dm = channel(&f, "dm", &[(&alice, "member"), (&bob, "member")]).await;
    let (proposal, outcome) = propose(&f, &alice, dm, &[(&alice, 1, 0), (&bob, 0, 0)]).await;
    let ImportControlOutcome::Applied { snapshot, .. } = outcome.unwrap() else {
        panic!("proposal not applied");
    };
    assert_eq!(accepts_after_tag(&snapshot), None, "proposed has no tag");

    // A reader snapshot taken before activation, as a lagging replica holds.
    let mut before = f.db.pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
        .execute(&mut *before)
        .await
        .unwrap();
    sqlx::query("SELECT 1 FROM import_sessions LIMIT 1")
        .execute(&mut *before)
        .await
        .unwrap();

    let content = serde_json::json!({
        "v": 1, "decision": "accept", "source_user": source_user(&bob)
    })
    .to_string();
    let event = consent_event(&bob, &proposal, dm, &content);
    let ImportControlOutcome::Applied { snapshot, .. } =
        f.db.record_import_consent(
            f.community,
            &event,
            proposal.id.as_bytes(),
            dm,
            MANIFEST,
            ImportDecision::Accept,
            &source_user(&bob),
            &f.relay,
        )
        .await
        .unwrap()
    else {
        panic!("consent not applied");
    };
    let (activated_at, stored_accepts_after): (i64, i64) = sqlx::query_as(
        "SELECT activated_at, accepts_after FROM import_sessions \
         WHERE community_id = $1 AND proposal_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(proposal.id.as_bytes().as_slice())
    .fetch_one(&f.db.pool)
    .await
    .unwrap();
    assert_eq!(
        stored_accepts_after,
        activated_at + IMPORT_ACCEPT_DELAY_SECS
    );
    assert_eq!(accepts_after_tag(&snapshot), Some(stored_accepts_after));

    // Inside the warm-up the import is refused, so the writer holds nothing
    // the pre-activation snapshot could be missing.
    let now = Utc::now().timestamp();
    let message = imported(&alice, &proposal, dm, now - 5_000, 1);
    let outcome =
        f.db.insert_import_event(
            f.community,
            &message,
            &tags_of(&message),
            dm,
            None,
            None,
            &f.relay,
        )
        .await
        .unwrap();
    assert_eq!(rejected_reason(&outcome), Some(reason::SESSION_WARMING));
    let imported_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM events WHERE community_id = $1 AND channel_id = $2 AND kind = 9",
    )
    .bind(f.community.as_uuid())
    .bind(dm)
    .fetch_one(&f.db.pool)
    .await
    .unwrap();
    assert_eq!(imported_rows, 0);

    // The pre-activation snapshot sees no active session and keeps the
    // replica; a current snapshot sends the channel to the writer.
    let entry = crate::runtime::replica_fence::TokenEntry {
        token: 1,
        committed_at: std::time::Instant::now(),
        fence_wall: Utc::now(),
    };
    let stale = crate::RouteDecision::Replica(before, entry, "covered");
    assert!(matches!(
        f.db.guard_covered_route("test", f.community, CoveredScope::Channel(dm), stale)
            .await,
        crate::RouteDecision::Replica(..)
    ));
    assert!(!covered(&f, CoveredScope::Channel(dm), "covered").await);

    // Once accepts_after has passed, the same event is accepted.
    sqlx::query(
        "UPDATE import_sessions SET accepts_after = extract(epoch FROM now())::bigint - 1 \
         WHERE community_id = $1 AND proposal_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(proposal.id.as_bytes().as_slice())
    .execute(&f.db.pool)
    .await
    .unwrap();
    let outcome =
        f.db.insert_import_event(
            f.community,
            &message,
            &tags_of(&message),
            dm,
            None,
            None,
            &f.relay,
        )
        .await
        .unwrap();
    assert!(is_inserted(&outcome));
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn active_sessions_are_capped_per_channel_and_proposer() {
    let f = fixture().await;
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let limits = import::ImportLimits {
        max_active_sessions_per_channel: 1,
        max_active_sessions_per_proposer: 2,
        ..LIMITS
    };
    // Distinct declared counts keep proposals signed in the same second from
    // sharing an id, which would be answered as a duplicate.
    let open = |channel: Uuid, messages: u64| {
        let content = proposal_content(
            &[(&alice, messages, 0), (&bob, 0, 0)],
            Utc::now().timestamp(),
        );
        let proposal = proposal_event(&alice, channel, &content);
        let db = &f.db;
        let community = f.community;
        let relay = &f.relay;
        async move {
            let outcome = db
                .create_import_session(
                    community,
                    &proposal,
                    &content,
                    channel,
                    content.expires_at,
                    limits,
                    relay,
                )
                .await
                .unwrap();
            (proposal, outcome)
        }
    };
    let rejection = |outcome: ImportControlOutcome| match outcome {
        ImportControlOutcome::Rejected(reason) => Some(reason),
        _ => None,
    };
    let first = channel(&f, "dm", &[(&alice, "member"), (&bob, "member")]).await;
    let second = channel(&f, "dm", &[(&alice, "member"), (&bob, "member")]).await;
    let third = channel(&f, "dm", &[(&alice, "member"), (&bob, "member")]).await;

    let (held, outcome) = open(first, 1).await;
    assert_eq!(rejection(outcome), None);
    let (_, outcome) = open(first, 2).await;
    assert_eq!(rejection(outcome), Some(reason::CHANNEL_SESSION_LIMIT));
    // A resend of the stored proposal is a duplicate, not a cap rejection.
    let content: ImportProposalContent = serde_json::from_str(&held.content).unwrap();
    let outcome =
        f.db.create_import_session(
            f.community,
            &held,
            &content,
            first,
            content.expires_at,
            limits,
            &f.relay,
        )
        .await
        .unwrap();
    assert!(matches!(outcome, ImportControlOutcome::Duplicate));
    let (_, outcome) = open(second, 1).await;
    assert_eq!(rejection(outcome), None);
    let (_, outcome) = open(third, 1).await;
    assert_eq!(rejection(outcome), Some(reason::PROPOSER_SESSION_LIMIT));

    // An expired session no longer counts, even before its status is stored.
    sqlx::query(
        "UPDATE import_sessions SET expires_at = 1 \
         WHERE community_id = $1 AND proposal_id = $2",
    )
    .bind(f.community.as_uuid())
    .bind(held.id.as_bytes().as_slice())
    .execute(&f.db.pool)
    .await
    .unwrap();
    let (_, outcome) = open(first, 3).await;
    assert_eq!(rejection(outcome), None);
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_participant_linked_to_an_owner_later_cannot_import() {
    let f = fixture().await;
    let (owner, alice) = (Keys::generate(), Keys::generate());
    let room = channel(&f, "stream", &[(&owner, "owner"), (&alice, "member")]).await;
    let (proposal, outcome) = propose(&f, &owner, room, &[(&owner, 0, 0), (&alice, 2, 0)]).await;
    applied(outcome).await;
    accept(&f, &alice, &proposal, room).await;
    let now = Utc::now().timestamp();
    assert!(is_inserted(
        &insert(&f, &imported(&alice, &proposal, room, now - 5_000, 1), room).await
    ));
    make_agent(&f, &alice, &owner).await;
    assert_eq!(
        rejected_reason(
            &insert(&f, &imported(&alice, &proposal, room, now - 4_000, 2), room).await
        ),
        Some(reason::PARTICIPANTS_INCLUDE_AGENTS)
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn the_floor_is_armed_again_after_an_import_transaction() {
    let f = fixture().await;
    // One connection, so the plain write below reuses the import's session.
    let db = Db::new(&crate::DbConfig {
        database_url: crate::test_support::database_url(),
        max_connections: 1,
        min_connections: 0,
        ..crate::DbConfig::default()
    })
    .await
    .unwrap();
    let (alice, bob) = (Keys::generate(), Keys::generate());
    let dm = channel(&f, "dm", &[(&alice, "member"), (&bob, "member")]).await;
    let (proposal, outcome) = propose(&f, &alice, dm, &[(&alice, 1, 0), (&bob, 0, 0)]).await;
    applied(outcome).await;
    accept(&f, &bob, &proposal, dm).await;
    skip_warmup(&f).await;

    let backend = || async {
        sqlx::query_scalar::<_, i32>("SELECT pg_backend_pid()")
            .fetch_one(&db.pool)
            .await
            .unwrap()
    };
    let pid = backend().await;
    let now = Utc::now().timestamp();
    let message = imported(&alice, &proposal, dm, now - 5_000, 1);
    let outcome = db
        .insert_import_event(
            f.community,
            &message,
            &tags_of(&message),
            dm,
            None,
            None,
            &f.relay,
        )
        .await
        .unwrap();
    assert!(is_inserted(&outcome));
    assert_eq!(backend().await, pid, "the pool reused its one connection");
    let floor: String = sqlx::query_scalar("SHOW buzz.created_at_floor")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        floor,
        crate::replica_fence::CREATED_AT_FLOOR_SECS.to_string()
    );

    // A plain channel write with an old created_at is still refused at commit.
    let old = sign(
        &alice,
        KIND_STREAM_MESSAGE,
        "old",
        vec![vec!["h".into(), dm.to_string()]],
        now - 5_000,
    );
    let error = crate::event::insert_event_with_thread_metadata(
        &db.pool,
        f.community,
        &old,
        Some(dm),
        None,
    )
    .await
    .expect_err("an old non-import row must not pass the floor");
    assert!(error.to_string().contains("replica-fence floor"), "{error}");
    assert_eq!(backend().await, pid);
}
