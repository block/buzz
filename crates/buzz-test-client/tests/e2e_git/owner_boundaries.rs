//! Members-only live checks for the three Git owner boundaries:
//! the read door admits an agent only while its owner is admitted, a manage
//! grant cannot consent to a rebind's destination, and an agent's approval
//! carrying its owner tag is an agent's, not a person's. Each case has a
//! control that must succeed on the same relay.
//!
//! Run against an isolated relay with `BUZZ_REQUIRE_RELAY_MEMBERSHIP=true`,
//! NIP-OA enabled, `RELAY_OWNER_PUBKEY` set to the key in `BUZZ_E2E_ROOT_SECRET`,
//! and its Postgres/S3 (same env as membership_or_grant):
//! `cargo test -p buzz-test-client --test e2e_git git_owner_boundaries -- --ignored --nocapture`.

use buzz_core::governance::{command_builder, Evidence, EvidenceBody, ExpectedHead};
use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp, ToBech32};

use super::{create_test_channel, git, git_status, post_event, relay_http_url, tempdir_named};

fn ws_url() -> String {
    let mut url = url::Url::parse(&relay_http_url()).unwrap();
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme).unwrap();
    url.to_string()
}

async fn submit(client: &mut BuzzTestClient, event: nostr::Event) -> Result<(), String> {
    let ok = client.send_event(event).await.map_err(|e| e.to_string())?;
    if ok.accepted {
        Ok(())
    } else {
        Err(ok.message)
    }
}

fn signed(keys: &Keys, kind: u16, tags: Vec<Tag>) -> nostr::Event {
    EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags)
        .sign_with_keys(keys)
        .unwrap()
}

fn add(channel: &str, who: &Keys, role: &str) -> Vec<Tag> {
    vec![
        Tag::parse(["h", channel]).unwrap(),
        Tag::parse(["p", &who.public_key().to_hex()]).unwrap(),
        Tag::parse(["role", role]).unwrap(),
    ]
}

fn auth_tag(owner: &Keys, agent: &Keys) -> Tag {
    let json = buzz_sdk::nip_oa::compute_auth_tag(owner, &agent.public_key(), "").unwrap();
    Tag::parse(serde_json::from_str::<Vec<String>>(&json).unwrap()).unwrap()
}

fn grant_tag(owner: &Keys, agent: &Keys, coordinate: &str, level: &str) -> Tag {
    let until = Timestamp::now().as_secs() + 1_800;
    let json = buzz_sdk::nip_oa::compute_git_grant(
        owner,
        &agent.public_key(),
        &format!("repo={coordinate}&git={level}&created_at<{until}"),
    )
    .unwrap();
    Tag::parse(serde_json::from_str::<Vec<String>>(&json).unwrap()).unwrap()
}

fn bind(
    community: uuid::Uuid,
    repo: &str,
    next: &str,
    prev: &ExpectedHead,
    signer: &Keys,
    tag: Option<Tag>,
) -> nostr::Event {
    let body = serde_json::json!({"version":1,"body":{"type":"bind","channel":next}}).to_string();
    let builder = command_builder(
        community,
        &format!("bind:{repo}").parse().unwrap(),
        prev,
        &body,
    )
    .unwrap();
    builder.tags(tag).sign_with_keys(signer).unwrap()
}

async fn pinned(pool: &sqlx::PgPool, community: uuid::Uuid, name: &str) -> Option<uuid::Uuid> {
    sqlx::query_scalar("SELECT channel_id FROM git_repo_names WHERE community_id=$1 AND repo_id=$2")
        .bind(community)
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Announce a repo bound to `channel` and seed `main`; returns its checkout
/// and base commit.
async fn announce(
    root: &Keys,
    channel: &str,
    temp: &std::path::Path,
) -> (String, String, std::path::PathBuf, String) {
    let root_nsec = root.secret_key().to_bech32().unwrap();
    let name = format!("bound-{}", uuid::Uuid::new_v4().simple());
    post_event(&signed(
        root,
        30617,
        vec![
            Tag::parse(["d", &name]).unwrap(),
            Tag::parse(["buzz-channel", channel]).unwrap(),
        ],
    ))
    .await;
    let url = format!(
        "{}/git/{}/{name}",
        relay_http_url(),
        root.public_key().to_hex()
    );
    git(&["clone", "--quiet", &url, &name], temp, &root_nsec);
    let seed = temp.join(&name);
    git(
        &["commit", "--quiet", "--allow-empty", "-m", "base"],
        &seed,
        &root_nsec,
    );
    git(
        &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
        &seed,
        &root_nsec,
    );
    let base = git(&["rev-parse", "HEAD"], &seed, &root_nsec)
        .trim()
        .to_string();
    (name, url, seed, base)
}

#[tokio::test]
#[ignore = "requires an isolated members-only live relay + MinIO + git"]
async fn git_owner_boundaries() {
    let community: uuid::Uuid = std::env::var("BUZZ_E2E_GIT_COMMUNITY_ID")
        .expect("BUZZ_E2E_GIT_COMMUNITY_ID required")
        .parse()
        .unwrap();
    let pool =
        sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL required"))
            .await
            .unwrap();

    // The relay owner (RELAY_OWNER_PUBKEY), also the repo announcer.
    let root =
        Keys::parse(&std::env::var("BUZZ_E2E_ROOT_SECRET").expect("BUZZ_E2E_ROOT_SECRET required"))
            .unwrap();
    let owner = Keys::generate(); // the agents' owner
    let agent = Keys::generate(); // member agent, for the read door
    let manager = Keys::generate(); // agent with a manage grant, for the rebind
    let reviewer = Keys::generate(); // agent that first presents its owner on an approval
    let person = Keys::generate(); // a person reviewer, the control
    for (keys, role) in [
        (&root, "owner"),
        (&owner, "member"),
        (&agent, "member"),
        (&manager, "member"),
        (&reviewer, "member"),
        (&person, "member"),
    ] {
        sqlx::query(
            "INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1,$2,$3) \
             ON CONFLICT (community_id, pubkey) DO UPDATE SET role=EXCLUDED.role",
        )
        .bind(community)
        .bind(keys.public_key().to_hex())
        .bind(role)
        .execute(&pool)
        .await
        .unwrap();
    }
    let ws = ws_url();
    let mut rc = BuzzTestClient::connect(&ws, &root).await.unwrap();
    // The reviewer is deliberately not linked: its approval is the first
    // place it presents its owner.
    for a in [&agent, &manager] {
        let tag = buzz_sdk::nip_oa::compute_auth_tag(&owner, &a.public_key(), "").unwrap();
        BuzzTestClient::connect_unauthenticated(&ws)
            .await
            .unwrap()
            .authenticate_with_nip_oa(a, &buzz_sdk::nip_oa::parse_auth_tag(&tag).unwrap())
            .await
            .expect("link agent to owner over NIP-42");
    }

    // The repo is bound to ch1; ch2 is the rebind destination. The owner
    // admins both; the agents are plain members of ch1 only.
    let ch1 = create_test_channel(&root).await;
    let ch2 = create_test_channel(&root).await;
    for (who, role) in [
        (&owner, "admin"),
        (&agent, "member"),
        (&manager, "member"),
        (&reviewer, "member"),
        (&person, "member"),
    ] {
        submit(&mut rc, signed(&root, 9000, add(&ch1, who, role)))
            .await
            .unwrap();
    }
    submit(&mut rc, signed(&root, 9000, add(&ch2, &owner, "admin")))
        .await
        .unwrap();

    let root_hex = root.public_key().to_hex();
    let root_nsec = root.secret_key().to_bech32().unwrap();
    let temp = tempdir_named("buzz-e2e-owner-boundaries");
    let (name, _, seed, base) = announce(&root, &ch1, temp.path()).await;
    let coordinate = format!("{root_hex}:{name}");
    let repo = format!("30617:{coordinate}");
    let mut failures: Vec<String> = Vec::new();
    let mut verdict = |label: &str, ok: bool, detail: String| {
        if ok {
            eprintln!("PASS {label}");
        } else {
            eprintln!("FAIL {label}");
            failures.push(format!("{label}: {detail}"));
        }
    };

    // 3. Review: main needs one independent person approval.
    let rules = r#"{"version":1,"body":{"type":"rules","rules":[{"id":"review","refs":["refs/heads/main"],"labels":{},"require":[{"type":"review","count":1,"group":null}]}]}}"#;
    let command = command_builder(
        community,
        &format!("rules:{repo}").parse().unwrap(),
        &ExpectedHead::Absent,
        rules,
    )
    .unwrap()
    .sign_with_keys(&root)
    .unwrap();
    submit(&mut rc, command)
        .await
        .expect("root sets the review rule");
    git(
        &["commit", "--quiet", "--allow-empty", "-m", "candidate"],
        &seed,
        &root_nsec,
    );
    let tip = git(&["rev-parse", "HEAD"], &seed, &root_nsec)
        .trim()
        .to_string();
    let approval = |by: &Keys, tag: Option<Tag>| {
        Evidence {
            community,
            repo: repo.clone(),
            git_ref: "refs/heads/main".into(),
            head: tip.clone(),
            base: base.clone(),
            body: EvidenceBody::Approval { approved: true },
        }
        .builder()
        .unwrap()
        .tags(tag)
        .sign_with_keys(by)
        .unwrap()
    };
    let push = || {
        git_status(
            &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
            &seed,
            &root_nsec,
        )
    };
    // Over HTTP: recording a first owner closes the agent's unowned
    // sockets, including one that carried this approval.
    post_event(&approval(&reviewer, Some(auth_tag(&owner, &reviewer)))).await;
    let linked: Option<Vec<u8>> = sqlx::query_scalar::<_, Option<Vec<u8>>>(
        "SELECT agent_owner_pubkey FROM users WHERE community_id=$1 AND pubkey=$2",
    )
    .bind(community)
    .bind(reviewer.public_key().to_bytes().to_vec())
    .fetch_optional(&pool)
    .await
    .unwrap()
    .flatten();
    let filed: Result<(), String> = if linked == Some(owner.public_key().to_bytes().to_vec()) {
        Ok(())
    } else {
        Err(format!("owner not recorded from the approval: {linked:?}"))
    };
    let out = push();
    verdict(
        "review: agent approval with owner tag, land refused",
        filed.is_ok() && !out.status.success(),
        format!("{filed:?}, push status {}", out.status),
    );
    post_event(&approval(&person, None)).await;
    let out = push();
    verdict(
        "review: person approval, land succeeds",
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into(),
    );

    // 2. Rebind: the owner admins both channels and grants manage; the agent
    // admins neither. The source may use the grant; the destination must not.
    // The control uses a second repo so the cases cannot see each other.
    let mut mc = BuzzTestClient::connect(&ws, &manager).await.unwrap();
    let manage = |coordinate: &str| Some(grant_tag(&owner, &manager, coordinate, "manage"));
    let before = pinned(&pool, community, &name).await;
    let refused = submit(
        &mut mc,
        bind(
            community,
            &repo,
            &ch2,
            &ExpectedHead::Absent,
            &manager,
            manage(&coordinate),
        ),
    )
    .await;
    let after = pinned(&pool, community, &name).await;
    verdict(
        "rebind: manage grant, owner admins destination, refused",
        refused.is_err() && after == before,
        format!("{refused:?}, pin {before:?} -> {after:?}"),
    );
    let (other, _, _, _) = announce(&root, &ch1, temp.path()).await;
    let other_coordinate = format!("{root_hex}:{other}");
    let other_repo = format!("30617:{other_coordinate}");
    submit(&mut rc, signed(&root, 9000, add(&ch2, &manager, "admin")))
        .await
        .unwrap();
    let moved = submit(
        &mut mc,
        bind(
            community,
            &other_repo,
            &ch2,
            &ExpectedHead::Absent,
            &manager,
            manage(&other_coordinate),
        ),
    )
    .await;
    let now = pinned(&pool, community, &other).await;
    verdict(
        "rebind: agent admins destination, moves",
        moved.is_ok() && now.map(|c| c.to_string()) == Some(ch2.clone()),
        format!("{moved:?}, pin {now:?}"),
    );

    // 1. Read door, last, on a repo bound to ch1, where the agent is a
    // plain member. Removing the owner from the community also removes its
    // agents; the operator then re-admits the agent itself, so the only fact
    // missing is the owner's admission.
    // Its own repo, so an earlier case's rebind cannot change its channel.
    let (_, read_url, _, _) = announce(&root, &ch1, temp.path()).await;
    let agent_nsec = agent.secret_key().to_bech32().unwrap();
    let clone = |dir: &str| {
        git_status(
            &["clone", "--quiet", &read_url, dir],
            temp.path(),
            &agent_nsec,
        )
    };
    let out = clone("read-owner-admitted");
    verdict(
        "read: owner admitted, member agent clones",
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into(),
    );
    submit(
        &mut rc,
        signed(
            &root,
            9031,
            vec![Tag::parse(["p", &owner.public_key().to_hex()]).unwrap()],
        ),
    )
    .await
    .expect("root removes the owner from the community");
    submit(
        &mut rc,
        signed(
            &root,
            9030,
            vec![
                Tag::parse(["p", &agent.public_key().to_hex()]).unwrap(),
                Tag::parse(["role", "member"]).unwrap(),
            ],
        ),
    )
    .await
    .expect("root re-admits the agent itself");
    submit(&mut rc, signed(&root, 9000, add(&ch1, &agent, "member")))
        .await
        .unwrap();
    let admitted: (bool, bool, Option<Vec<u8>>) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM relay_members WHERE community_id=$1 AND pubkey=$2), \
                EXISTS (SELECT 1 FROM relay_members WHERE community_id=$1 AND pubkey=$3), \
                (SELECT agent_owner_pubkey FROM users WHERE community_id=$1 AND pubkey=decode($2,'hex'))",
    )
    .bind(community)
    .bind(agent.public_key().to_hex())
    .bind(owner.public_key().to_hex())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        admitted,
        (true, false, Some(owner.public_key().to_bytes().to_vec())),
        "precondition: the agent is admitted and still linked; its owner is not admitted"
    );
    // Outlast the relay's membership cache (10 s) for the re-admitted agent.
    tokio::time::sleep(std::time::Duration::from_secs(11)).await;
    let out = clone("read-owner-removed");
    verdict(
        "read: agent admitted, owner not admitted, clone denied",
        !out.status.success(),
        "clone succeeded".into(),
    );

    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
