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

async fn insert(f: &Fixture, event: &Event, channel: Uuid) -> ImportInsertOutcome {
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

fn control_rejection(outcome: Result<ImportControlOutcome>) -> &'static str {
    match outcome.unwrap() {
        ImportControlOutcome::Rejected(reason) => reason,
        other => panic!("expected a rejection, got {other:?}"),
    }
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
