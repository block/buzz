//! Real Git journeys for channel membership and delegated access.
//!
//! Run against an isolated open relay (NIP-OA enabled) and its Postgres/S3:
//! `DATABASE_URL=… BUZZ_E2E_GIT_COMMUNITY_ID=… RELAY_HTTP_URL=… GIT_CREDENTIAL_NOSTR_BIN=…
//! cargo test -p buzz-test-client --test e2e_git git_membership_or_grant -- --ignored --nocapture`.
//! As with the other Git e2es, configure the relay's `BUZZ_S3_*` settings.

use std::path::Path;
use std::process::Output;

use buzz_test_client::BuzzTestClient;
use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp, ToBech32};

use super::{
    create_test_channel, git, git_command, post_event, relay_http_url, tempdir_named, GitS3Probe,
};

async fn send(client: &mut BuzzTestClient, signer: &Keys, kind: u16, tags: Vec<Tag>) {
    let event = EventBuilder::new(Kind::Custom(kind), "")
        .tags(tags)
        .sign_with_keys(signer)
        .unwrap();
    let result = client.send_event(event).await.unwrap();
    assert!(result.accepted, "kind {kind}: {}", result.message);
}

async fn membership(
    client: &mut BuzzTestClient,
    owner: &Keys,
    channel: &str,
    member: &Keys,
    add: bool,
) {
    send(
        client,
        owner,
        if add { 9000 } else { 9001 },
        vec![
            Tag::parse(["h", channel]).unwrap(),
            Tag::parse(["p", &member.public_key().to_hex()]).unwrap(),
        ],
    )
    .await;
}

async fn link_agent(ws_url: &str, owner: &Keys, agent: &Keys) {
    let tag = buzz_sdk::nip_oa::compute_auth_tag(owner, &agent.public_key(), "").unwrap();
    let tag = buzz_sdk::nip_oa::parse_auth_tag(&tag).unwrap();
    BuzzTestClient::connect_unauthenticated(ws_url)
        .await
        .unwrap()
        .authenticate_with_nip_oa(agent, &tag)
        .await
        .expect("link agent to owner over NIP-42");
}

fn grant(owner: &Keys, agent: &Keys, coordinate: &str, level: &str, expires: u64) -> String {
    buzz_sdk::nip_oa::compute_git_grant(
        owner,
        &agent.public_key(),
        &format!("repo={coordinate}&git={level}&created_at<{expires}"),
    )
    .unwrap()
}

fn run(args: &[&str], cwd: &Path, actor: &Keys, tag: Option<&str>) -> Output {
    let mut command = git_command(args, cwd, &actor.secret_key().to_bech32().unwrap());
    if let Some(tag) = tag {
        command.env("BUZZ_AUTH_TAG", tag);
    }
    command.output().expect("spawn git")
}

#[derive(Clone, Copy, Debug)]
enum Expected {
    Success,
    Hidden,
    Banned,
    HookRejected,
}

impl Expected {
    fn matches(self, out: &Output) -> bool {
        let stderr = String::from_utf8_lossy(&out.stderr);
        match self {
            Self::Success => out.status.success(),
            // The Git advertisement hides an unauthorized repo behind a 404.
            Self::Hidden => !out.status.success() && stderr.contains("not found"),
            // The ban check runs before repository authorization and returns 403.
            Self::Banned => !out.status.success() && stderr.contains("403"),
            Self::HookRejected => {
                !out.status.success() && stderr.contains("pre-receive hook declined")
            }
        }
    }
}

fn check(failures: &mut Vec<String>, label: &str, expected: Expected, out: &Output) -> bool {
    let matched = expected.matches(out);
    if matched {
        eprintln!("PASS {label}");
    } else {
        failures.push(format!(
            "{label}: expected {expected:?}, got {}\n{}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    matched
}

struct Repo<'a> {
    url: &'a str,
    seed: &'a Path,
    owner_nsec: &'a str,
    seed_oid: &'a str,
}

impl Repo<'_> {
    fn clone_as(
        &self,
        failures: &mut Vec<String>,
        label: &str,
        cwd: &Path,
        actor: &Keys,
        tag: Option<&str>,
        expected: Expected,
    ) -> bool {
        check(
            failures,
            label,
            expected,
            &run(
                &["clone", "--quiet", "-b", "main", self.url, label],
                cwd,
                actor,
                tag,
            ),
        )
    }

    fn push_as(
        &self,
        failures: &mut Vec<String>,
        branch: &str,
        actor: &Keys,
        tag: Option<&str>,
        expected: Expected,
    ) {
        let remote_ref = format!("refs/heads/{branch}");
        let refspec = format!("HEAD:{remote_ref}");
        let output = run(
            &["push", "--quiet", self.url, &refspec],
            self.seed,
            actor,
            tag,
        );
        check(failures, &format!("{branch} push"), expected, &output);
        let tip = git(
            &["ls-remote", "--refs", self.url, &remote_ref],
            self.seed,
            self.owner_nsec,
        );
        let wanted = if matches!(expected, Expected::Success) {
            format!("{}\t{remote_ref}", self.seed_oid)
        } else {
            String::new()
        };
        if tip.trim() != wanted {
            failures.push(format!(
                "{branch}: remote ref was {:?}, expected {wanted:?}",
                tip.trim()
            ));
        }
    }
}

/// Eight cases exercise actual Git requests and verify whether each push landed.
#[tokio::test]
#[ignore = "requires an isolated live relay with Postgres, Redis and S3"]
async fn git_membership_or_grant() {
    use Expected::{Banned, Hidden, HookRejected, Success};

    let owner = Keys::generate();
    let outside_owner = Keys::generate();
    let member_agent = Keys::generate();
    let outsider_agent = Keys::generate();
    let grant_agent = Keys::generate();
    let person = Keys::generate();

    // The owner is also the moderator for this fixture. All role changes
    // inside the repo's channel and the ban itself use signed live events.
    let community: uuid::Uuid = std::env::var("BUZZ_E2E_GIT_COMMUNITY_ID")
        .expect("BUZZ_E2E_GIT_COMMUNITY_ID required")
        .parse()
        .unwrap();
    let pool =
        sqlx::PgPool::connect(&std::env::var("DATABASE_URL").expect("DATABASE_URL required"))
            .await
            .unwrap();
    sqlx::query("INSERT INTO relay_members (community_id, pubkey, role) VALUES ($1, $2, 'owner')")
        .bind(community)
        .bind(owner.public_key().to_hex())
        .execute(&pool)
        .await
        .unwrap();

    let base = relay_http_url();
    let mut ws_url = url::Url::parse(&base).unwrap();
    let scheme = if ws_url.scheme() == "https" {
        "wss"
    } else {
        "ws"
    };
    ws_url.set_scheme(scheme).unwrap();
    let mut owner_client = BuzzTestClient::connect(ws_url.as_str(), &owner)
        .await
        .unwrap();
    let channel = create_test_channel(&owner).await;
    membership(&mut owner_client, &owner, &channel, &member_agent, true).await;
    membership(&mut owner_client, &owner, &channel, &person, true).await;
    link_agent(ws_url.as_str(), &owner, &member_agent).await;
    link_agent(ws_url.as_str(), &owner, &grant_agent).await;
    link_agent(ws_url.as_str(), &outside_owner, &outsider_agent).await;

    let owner_hex = owner.public_key().to_hex();
    let owner_nsec = owner.secret_key().to_bech32().unwrap();
    let name = format!("mog-{}", uuid::Uuid::new_v4());
    let announce = EventBuilder::new(Kind::Custom(30617), "")
        .tags([
            Tag::parse(["d", &name]).unwrap(),
            Tag::parse(["buzz-channel", &channel]).unwrap(),
            Tag::parse(["buzz-protect", "refs/heads/admin-only", "push:admin"]).unwrap(),
        ])
        .sign_with_keys(&owner)
        .unwrap();
    post_event(&announce).await;
    GitS3Probe::from_env()
        .require_pointer(&owner_hex, &name)
        .await;

    let temp = tempdir_named("buzz-e2e-git-membership-or-grant");
    let url = format!("{base}/git/{owner_hex}/{name}");
    git(
        &["clone", "--quiet", &url, "seed"],
        temp.path(),
        &owner_nsec,
    );
    let seed = temp.path().join("seed");
    git(
        &["commit", "--quiet", "--allow-empty", "-m", "seed"],
        &seed,
        &owner_nsec,
    );
    git(
        &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
        &seed,
        &owner_nsec,
    );
    let seed_oid = git(&["rev-parse", "HEAD"], &seed, &owner_nsec);
    let repo = Repo {
        url: &url,
        seed: &seed,
        owner_nsec: &owner_nsec,
        seed_oid: seed_oid.trim(),
    };
    let coordinate = format!("{owner_hex}:{name}");
    let now = Timestamp::now().as_secs();
    let push_tag = grant(&owner, &grant_agent, &coordinate, "push", now + 1_800);
    let read_tag = grant(&owner, &grant_agent, &coordinate, "read", now + 1_800);
    let outside_tag = grant(
        &outside_owner,
        &outsider_agent,
        &coordinate,
        "push",
        now + 1_800,
    );
    let expired_tag = grant(
        &outside_owner,
        &outsider_agent,
        &coordinate,
        "push",
        now.saturating_sub(60),
    );

    let mut failures = Vec::new();
    let cloned = repo.clone_as(
        &mut failures,
        "member-no-grant",
        temp.path(),
        &member_agent,
        None,
        Success,
    );
    // Use the seeded checkout if this assertion failed so the baseline run
    // still reaches the other independent cases.
    let member_checkout = if cloned {
        temp.path().join("member-no-grant")
    } else {
        seed.clone()
    };
    // Add a ref the member's checkout has never seen, then fetch and
    // verify it. A fetch with nothing new could stop at the advertisement.
    git(
        &["commit", "--quiet", "--allow-empty", "-m", "fetch target"],
        &seed,
        &owner_nsec,
    );
    git(
        &["push", "--quiet", "origin", "HEAD:refs/heads/fetch-target"],
        &seed,
        &owner_nsec,
    );
    let fetched = check(
        &mut failures,
        "member without grant fetch",
        Success,
        &run(
            &["fetch", "--quiet", "origin"],
            &member_checkout,
            &member_agent,
            None,
        ),
    );
    if fetched {
        let fetched_oid = git(
            &["rev-parse", "refs/remotes/origin/fetch-target"],
            &member_checkout,
            &owner_nsec,
        );
        let target_oid = git(&["rev-parse", "HEAD"], &seed, &owner_nsec);
        assert_eq!(fetched_oid, target_oid);
    }
    git(
        &["reset", "--quiet", "--hard", repo.seed_oid],
        &seed,
        &owner_nsec,
    );
    repo.push_as(
        &mut failures,
        "member-no-grant",
        &member_agent,
        None,
        Success,
    );

    let member_read_tag = grant(&owner, &member_agent, &coordinate, "read", now + 1_800);
    repo.push_as(
        &mut failures,
        "member-with-read-grant",
        &member_agent,
        Some(&member_read_tag),
        Success,
    );

    membership(&mut owner_client, &owner, &channel, &member_agent, false).await;
    check(
        &mut failures,
        "removed member fetch",
        Hidden,
        &run(
            &["fetch", "--quiet", "origin"],
            &member_checkout,
            &member_agent,
            None,
        ),
    );
    repo.push_as(&mut failures, "removed-member", &member_agent, None, Hidden);

    for (label, agent, tag, read, push) in [
        (
            "grant-only-push",
            &grant_agent,
            Some(push_tag.as_str()),
            Success,
            Success,
        ),
        (
            "grant-only-read",
            &grant_agent,
            Some(read_tag.as_str()),
            Success,
            HookRejected,
        ),
        (
            "outside-owner",
            &outsider_agent,
            Some(outside_tag.as_str()),
            Hidden,
            Hidden,
        ),
        ("neither-route", &grant_agent, None, Hidden, Hidden),
    ] {
        repo.clone_as(&mut failures, label, temp.path(), agent, tag, read);
        repo.push_as(&mut failures, label, agent, tag, push);
    }

    // The hook must receive the grant's level to enforce the protection.
    // These requests differ only in grant level and target the same ref.
    let manage_tag = grant(&owner, &grant_agent, &coordinate, "manage", now + 1_800);
    repo.push_as(
        &mut failures,
        "admin-only",
        &grant_agent,
        Some(&push_tag),
        HookRejected,
    );
    repo.push_as(
        &mut failures,
        "admin-only",
        &grant_agent,
        Some(&manage_tag),
        Success,
    );

    // The expired tag leaves the agent's direct membership as its source of authority.
    // This agent's owner never joined the channel.
    membership(&mut owner_client, &owner, &channel, &outsider_agent, true).await;
    repo.push_as(
        &mut failures,
        "expired-grant-member",
        &outsider_agent,
        Some(&expired_tag),
        Success,
    );

    send(
        &mut owner_client,
        &owner,
        9040,
        vec![Tag::parse(["p", &outside_owner.public_key().to_hex()]).unwrap()],
    )
    .await;
    repo.clone_as(
        &mut failures,
        "banned-owner",
        temp.path(),
        &outsider_agent,
        None,
        Banned,
    );
    repo.push_as(&mut failures, "banned-owner", &outsider_agent, None, Banned);

    repo.clone_as(
        &mut failures,
        "person-member",
        temp.path(),
        &person,
        None,
        Success,
    );
    repo.push_as(&mut failures, "person-member", &person, None, Success);
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}
