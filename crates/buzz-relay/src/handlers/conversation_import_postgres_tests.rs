//! Postgres-backed tests for NIP-CI control events through the ingest gates.

use buzz_core::import::{
    ImportParticipant, ImportProposalContent, ImportRange, ImportSource, ImportSourceKind,
};
use nostr::{Event, EventBuilder, Keys, Kind, Tag};
use uuid::Uuid;

use crate::handlers::group_state_postgres_tests::{community, create_channel, http_auth, state};
use crate::handlers::ingest::{ingest_event, IngestError};

const MANIFEST: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn sign(keys: &Keys, kind: u16, content: &str, tags: Vec<Vec<String>>) -> Event {
    EventBuilder::new(Kind::Custom(kind), content)
        .tags(tags.into_iter().map(|t| Tag::parse(t).unwrap()))
        .allow_self_tagging()
        .sign_with_keys(keys)
        .unwrap()
}

fn proposal(proposer: &Keys, channel: Uuid, participants: &[(&Keys, &str)]) -> Event {
    let now = chrono::Utc::now().timestamp();
    let mut participants: Vec<ImportParticipant> = participants
        .iter()
        .map(|(keys, source_user)| ImportParticipant {
            pubkey: keys.public_key().to_hex(),
            source_user: (*source_user).to_string(),
            messages: 1,
            reactions: 0,
        })
        .collect();
    participants.sort_by(|a, b| a.pubkey.cmp(&b.pubkey));
    let mut tags = vec![vec!["h".to_string(), channel.to_string()]];
    tags.extend(
        participants
            .iter()
            .map(|p| vec!["p".to_string(), p.pubkey.clone()]),
    );
    tags.push(vec!["manifest".into(), MANIFEST.into()]);
    tags.push(vec!["import-source".into(), "test".into()]);
    let content = ImportProposalContent {
        v: 1,
        source: ImportSource {
            source_type: "test".into(),
            conversation: "2".repeat(64),
            kind: ImportSourceKind::PrivateChannel,
            label: "Channel".into(),
        },
        range: ImportRange {
            from: now - 10_000,
            to: now - 100,
        },
        participants,
        excluded_authors: 0,
        manifest: MANIFEST.into(),
        expires_at: now + 3_600,
    };
    sign(
        proposer,
        42110,
        &serde_json::to_string(&content).unwrap(),
        tags,
    )
}

fn session_tags(proposal: &Event, channel: Uuid) -> Vec<Vec<String>> {
    vec![
        vec!["e".into(), proposal.id.to_hex()],
        vec!["h".into(), channel.to_string()],
    ]
}

fn rejection(result: Result<crate::handlers::ingest::IngestResult, IngestError>) -> String {
    match result {
        Err(IngestError::Rejected(reason)) => reason,
        Err(other) => panic!("expected a rejection, got {other:?}"),
        Ok(result) => panic!("expected a rejection, got OK {}", result.message),
    }
}

/// A participant who has left the channel can still send kind:42113, while
/// other control events keep the membership gate and the session store
/// still requires a participant.
#[tokio::test]
#[ignore = "requires Postgres"]
async fn a_participant_who_left_can_still_complete() {
    let state = state().await;
    let tenant = community(&state, "import-complete").await;
    let (owner, alice, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
    let channel = create_channel(&state, &tenant, &owner, &[&["visibility", "private"]]).await;
    let put = sign(
        &owner,
        9000,
        "",
        vec![
            vec!["h".into(), channel.to_string()],
            vec!["p".into(), alice.public_key().to_hex()],
        ],
    );
    ingest_event(&state, &tenant, put, http_auth(&owner))
        .await
        .expect("add alice");

    let alice_user = "3".repeat(64);
    let proposal = proposal(
        &owner,
        channel,
        &[(&owner, &"4".repeat(64)), (&alice, &alice_user)],
    );
    ingest_event(&state, &tenant, proposal.clone(), http_auth(&owner))
        .await
        .expect("proposal");

    let consent_content =
        serde_json::json!({"v": 1, "decision": "accept", "source_user": alice_user}).to_string();
    let consent_tags = |p: &Keys| {
        let mut tags = session_tags(&proposal, channel);
        tags.push(vec!["manifest".into(), MANIFEST.into()]);
        tags.push(vec!["p".into(), p.public_key().to_hex()]);
        tags
    };
    let wrong_p = sign(&alice, 42111, &consent_content, consent_tags(&alice));
    assert_eq!(
        rejection(ingest_event(&state, &tenant, wrong_p, http_auth(&alice)).await),
        "invalid: import consent does not match proposal"
    );
    let consent = sign(&alice, 42111, &consent_content, consent_tags(&owner));
    ingest_event(&state, &tenant, consent, http_auth(&alice))
        .await
        .expect("consent");

    let leave = sign(
        &alice,
        9022,
        "",
        vec![vec!["h".into(), channel.to_string()]],
    );
    ingest_event(&state, &tenant, leave, http_auth(&alice))
        .await
        .expect("leave");

    let done = r#"{"v":1,"published":{"messages":0,"reactions":0}}"#;
    let stranger_done = sign(&stranger, 42113, done, session_tags(&proposal, channel));
    assert_eq!(
        rejection(ingest_event(&state, &tenant, stranger_done, http_auth(&stranger)).await),
        "restricted: import author not a participant"
    );
    let withdraw = sign(&alice, 42112, "", session_tags(&proposal, channel));
    assert_eq!(
        rejection(ingest_event(&state, &tenant, withdraw, http_auth(&alice)).await),
        "restricted: not a channel member"
    );
    let complete = sign(&alice, 42113, done, session_tags(&proposal, channel));
    let result = ingest_event(&state, &tenant, complete, http_auth(&alice))
        .await
        .expect("a participant who left completes");
    assert!(result.accepted, "{}", result.message);
}

/// Imported history is invisible to agent readers on pull paths as well as
/// live fan-out: REQ, NIP-50 search and HTTP /query return it to humans only.
/// Mutation: drop the gate from the REQ history loop, the search loop or the
/// /query post-filter and this fails.
#[tokio::test]
#[ignore = "requires Postgres and Redis"]
async fn imported_history_is_hidden_from_agent_readers() {
    use super::system_channel_postgres_tests as reads;
    use serde_json::json;

    let f = reads::fixture().await;
    let (owner, alice, bot) = (Keys::generate(), Keys::generate(), Keys::generate());
    let channel = create_channel(&f.state, &f.tenant, &owner, &[]).await;
    for member in [&alice, &bot] {
        let put = sign(
            &owner,
            9000,
            "",
            vec![
                vec!["h".into(), channel.to_string()],
                vec!["p".into(), member.public_key().to_hex()],
            ],
        );
        ingest_event(&f.state, &f.tenant, put, http_auth(&owner))
            .await
            .expect("add member");
    }
    let pool = sqlx::PgPool::connect(&crate::test_support::database_url())
        .await
        .expect("connect");
    sqlx::query(
        "INSERT INTO users (community_id, pubkey) VALUES ($1, $2) \
         ON CONFLICT (community_id, pubkey) DO NOTHING",
    )
    .bind(f.tenant.community().as_uuid())
    .bind(owner.public_key().to_bytes().as_slice())
    .execute(&pool)
    .await
    .expect("owner user row");
    sqlx::query(
        "INSERT INTO users (community_id, pubkey, agent_owner_pubkey) VALUES ($1, $2, $3) \
         ON CONFLICT (community_id, pubkey) DO UPDATE \
         SET agent_owner_pubkey = EXCLUDED.agent_owner_pubkey",
    )
    .bind(f.tenant.community().as_uuid())
    .bind(bot.public_key().to_bytes().as_slice())
    .bind(owner.public_key().to_bytes().as_slice())
    .execute(&pool)
    .await
    .expect("make agent");

    let alice_user = "3".repeat(64);
    let proposal = proposal(
        &owner,
        channel,
        &[(&owner, &"4".repeat(64)), (&alice, &alice_user)],
    );
    ingest_event(&f.state, &f.tenant, proposal.clone(), http_auth(&owner))
        .await
        .expect("proposal");
    let mut consent_tags = session_tags(&proposal, channel);
    consent_tags.push(vec!["manifest".into(), MANIFEST.into()]);
    consent_tags.push(vec!["p".into(), owner.public_key().to_hex()]);
    let consent = sign(
        &alice,
        42111,
        &json!({"v": 1, "decision": "accept", "source_user": alice_user}).to_string(),
        consent_tags,
    );
    ingest_event(&f.state, &f.tenant, consent, http_auth(&alice))
        .await
        .expect("consent");
    sqlx::query("UPDATE import_sessions SET accepts_after = 0 WHERE community_id = $1")
        .bind(f.tenant.community().as_uuid())
        .execute(&pool)
        .await
        .expect("skip warm-up");

    let word = format!("w{}", Uuid::new_v4().simple());
    let imported = EventBuilder::new(Kind::Custom(9), format!("imported {word}"))
        .tags([
            Tag::parse(["h", &channel.to_string()]).unwrap(),
            Tag::parse(["import", &proposal.id.to_hex()]).unwrap(),
            Tag::parse(["source", "test", "ref-1"]).unwrap(),
        ])
        .custom_created_at(nostr::Timestamp::from(
            (chrono::Utc::now().timestamp() - 5_000) as u64,
        ))
        .sign_with_keys(&alice)
        .unwrap();
    let result = ingest_event(&f.state, &f.tenant, imported.clone(), http_auth(&alice))
        .await
        .expect("imported message");
    assert!(result.accepted, "{}", result.message);
    let live = sign(
        &alice,
        9,
        &format!("live {word}"),
        vec![vec!["h".into(), channel.to_string()]],
    );
    ingest_event(&f.state, &f.tenant, live.clone(), http_auth(&alice))
        .await
        .expect("live message");

    let (imported_id, live_id) = (imported.id.to_hex(), live.id.to_hex());
    let history = json!({"kinds": [9], "#h": [channel.to_string()]});
    let search = json!({"kinds": [9], "search": word});
    for (who, reader, sees_import) in [("human", &owner, true), ("agent", &bot, false)] {
        for (path, ids) in [
            ("REQ", reads::req(&f, reader, history.clone()).await),
            ("search", reads::req(&f, reader, search.clone()).await),
            (
                "/query",
                reads::bridge_ids(
                    reads::bridge(&f, reader, "/query", json!([history.clone()])).await,
                ),
            ),
        ] {
            assert!(ids.contains(&live_id), "{who} {path}: live missing {ids:?}");
            assert_eq!(
                ids.contains(&imported_id),
                sees_import,
                "{who} {path}: imported visibility {ids:?}"
            );
        }
    }
}
