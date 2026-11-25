//! Both Git authorization routes at the database policy boundary.
use super::*;

struct Fixture {
    db: Db,
    community: CommunityId,
    channel: Uuid,
    next: Uuid,
    repo: RepoCoordinate,
    owner: Keys,
    agent: Keys,
}

impl Fixture {
    async fn new() -> Self {
        let db = Db::new(&crate::DbConfig {
            database_url: crate::test_support::database_url(),
            max_connections: 3,
            min_connections: 1,
            ..Default::default()
        })
        .await
        .expect("connect");
        let community = CommunityId::from_uuid(Uuid::new_v4());
        sqlx::query("INSERT INTO communities (id, host) VALUES ($1,$2)")
            .bind(community.as_uuid())
            .bind(format!("git-authority-{}.example", community.as_uuid()))
            .execute(db.pool())
            .await
            .expect("community");
        let owner = Keys::generate();
        let agent = Keys::generate();
        sqlx::query("INSERT INTO users (community_id,pubkey,agent_owner_pubkey) VALUES ($1,$2,$3)")
            .bind(community.as_uuid())
            .bind(agent.public_key().as_bytes())
            .bind(owner.public_key().as_bytes())
            .execute(db.pool())
            .await
            .expect("agent");
        let channel = Uuid::new_v4();
        let next = Uuid::new_v4();
        for ch in [channel, next] {
            sqlx::query("INSERT INTO channels (community_id,id,name,channel_type,visibility,created_by) VALUES ($1,$2,$3,'stream','private',$4)")
                .bind(community.as_uuid()).bind(ch).bind(ch.to_string())
                .bind(owner.public_key().as_bytes()).execute(db.pool()).await.expect("channel");
        }
        let repo = RepoCoordinate {
            owner: owner.public_key().to_hex(),
            name: "authority".into(),
        };
        crate::git_repo::reserve_repo_name(db.pool(), community, &repo.name, &repo.owner)
            .await
            .expect("repo");
        let announcement = EventBuilder::new(Kind::Custom(30617), "")
            .tags([
                Tag::parse(["d", &repo.name]).expect("d tag"),
                Tag::parse(["buzz-channel", &channel.to_string()]).expect("channel tag"),
            ])
            .sign_with_keys(&owner)
            .expect("announcement");
        db.insert_event(community, &announcement, Some(channel))
            .await
            .expect("insert announcement");
        Self {
            db,
            community,
            channel,
            next,
            repo,
            owner,
            agent,
        }
    }

    async fn role(&self, channel: Uuid, key: &Keys, role: Option<&str>) {
        sqlx::query(
            "DELETE FROM channel_members WHERE community_id=$1 AND channel_id=$2 AND pubkey=$3",
        )
        .bind(self.community.as_uuid())
        .bind(channel)
        .bind(key.public_key().as_bytes())
        .execute(self.db.pool())
        .await
        .expect("remove membership");
        if let Some(role) = role {
            sqlx::query("INSERT INTO channel_members (community_id,channel_id,pubkey,role) VALUES ($1,$2,$3,$4::member_role)")
                .bind(self.community.as_uuid()).bind(channel).bind(key.public_key().as_bytes())
                .bind(role).execute(self.db.pool()).await.expect("membership");
        }
    }

    fn grant(&self, level: GrantAction) -> GitGrant {
        let now = Timestamp::now().as_secs();
        GitGrant {
            owner: self.owner.public_key(),
            repo: Some(self.repo.clone()),
            level,
            after: Some(now - 10),
            before: Some(now + 600),
        }
    }

    async fn check(&self, grant: Option<&GitGrant>, read: bool, manage: bool) {
        let mut tx = self
            .db
            .begin_event_write_transaction(self.community)
            .await
            .expect("tx");
        let id = DocumentId::Rules(self.repo.clone());
        assert_eq!(
            authorize_read(
                &mut tx,
                &id,
                self.agent.public_key().as_bytes(),
                grant,
                true
            )
            .await
            .is_ok(),
            read,
            "read {grant:?}"
        );
        assert_eq!(
            repo_admin(
                &mut tx,
                &self.repo,
                self.agent.public_key().as_bytes(),
                self.channel,
                grant,
                true,
            )
            .await
            .expect("admin"),
            manage,
            "manage {grant:?}"
        );
        tx.rollback().await.expect("rollback");
    }

    async fn admission_outcomes(&self, open: bool) -> [bool; 4] {
        let read = self
            .db
            .governance_head(
                self.community,
                &DocumentId::Rules(self.repo.clone()),
                self.agent.public_key().as_bytes(),
                None,
                open,
            )
            .await
            .is_ok();
        let receipts = self
            .db
            .git_receipts(
                self.community,
                &self.repo,
                self.agent.public_key().as_bytes(),
                None,
                None,
                10,
                open,
            )
            .await
            .is_ok();
        let mut writes = Vec::new();
        for (id, body) in [
            (
                DocumentId::Rules(self.repo.clone()),
                r#"{"version":1,"body":{"type":"rules","rules":[]}}"#.to_owned(),
            ),
            (
                DocumentId::Bind(self.repo.clone()),
                serde_json::json!({"version":1,"body":{"type":"bind","channel":self.next}})
                    .to_string(),
            ),
        ] {
            let event = buzz_core::governance::command_builder(
                *self.community.as_uuid(),
                &id,
                &ExpectedHead::Absent,
                &body,
            )
            .expect("command")
            .sign_with_keys(&self.agent)
            .expect("sign");
            let mut tx = self
                .db
                .begin_event_write_transaction(self.community)
                .await
                .expect("tx");
            fence_authority_write(&mut tx, &id).await.expect("fence");
            writes.push(
                self.db
                    .prepare_governance_command(&mut tx, &event, None, &self.owner, open)
                    .await
                    .is_ok(),
            );
            tx.rollback().await.expect("rollback");
        }
        [read, receipts, writes[0], writes[1]]
    }

    async fn prepare(&self, grant: Option<&GitGrant>, bind: bool) -> Result<Prepared> {
        let (id, body) = if bind {
            (
                DocumentId::Bind(self.repo.clone()),
                serde_json::json!({"version":1,"body":{"type":"bind","channel":self.next}})
                    .to_string(),
            )
        } else {
            (
                DocumentId::Rules(self.repo.clone()),
                r#"{"version":1,"body":{"type":"rules","rules":[]}}"#.into(),
            )
        };
        let event = buzz_core::governance::command_builder(
            *self.community.as_uuid(),
            &id,
            &ExpectedHead::Absent,
            &body,
        )
        .expect("command")
        .sign_with_keys(&self.agent)
        .expect("sign");
        let mut tx = self
            .db
            .begin_event_write_transaction(self.community)
            .await
            .expect("tx");
        fence_authority_write(&mut tx, &id).await.expect("fence");
        let prepared = self
            .db
            .prepare_governance_command(&mut tx, &event, grant, &self.owner, true)
            .await;
        tx.rollback().await.expect("rollback");
        prepared
    }
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn membership_and_delegation_independently_authorize_repo_documents() {
    let f = Fixture::new().await;
    // An agent's own role works without its owner being in the channel.
    for (role, read, manage) in [
        (None, false, false),
        (Some("guest"), true, false),
        (Some("bot"), true, false),
        (Some("admin"), true, true),
    ] {
        f.role(f.channel, &f.agent, role).await;
        f.check(None, read, manage).await;
    }
    let manage = f.grant(GrantAction::Manage);
    let read = f.grant(GrantAction::Read);
    let mut expired = manage.clone();
    expired.before = Some(Timestamp::now().as_secs() - 1);
    let mut wrong_repo = manage.clone();
    wrong_repo.repo.as_mut().expect("repo").name = "elsewhere".into();
    // A narrower, expired or unrelated grant cannot revoke independent admin.
    for grant in [&read, &expired, &wrong_repo] {
        f.check(Some(grant), true, true).await;
        assert_eq!(
            f.prepare(Some(grant), false)
                .await
                .expect("rules")
                .valid_until,
            None
        );
    }
    // Removing membership leaves only delegation, capped by the owner.
    f.role(f.channel, &f.agent, None).await;
    f.check(Some(&manage), false, false).await;
    f.role(f.channel, &f.owner, Some("guest")).await;
    f.check(Some(&manage), true, false).await;
    f.role(f.channel, &f.owner, Some("admin")).await;
    f.check(Some(&read), true, false).await;
    f.check(Some(&f.grant(GrantAction::Push)), true, false)
        .await;
    f.check(Some(&manage), true, true).await;
    assert_eq!(
        f.prepare(Some(&manage), false)
            .await
            .expect("delegated rules")
            .valid_until,
        manage.before.map(|at| at as i64)
    );
    for grant in [&expired, &wrong_repo] {
        f.check(Some(grant), false, false).await;
    }
    let mut wrong_owner = manage.clone();
    wrong_owner.owner = Keys::generate().public_key();
    f.check(Some(&wrong_owner), false, false).await;
    // A wildcard still needs the recorded owner's channel role.
    let mut wildcard = manage.clone();
    wildcard.repo = None;
    f.check(Some(&wildcard), true, true).await;
    f.role(f.channel, &f.owner, None).await;
    f.check(Some(&wildcard), false, false).await;

    // Owner bans override both routes, including the agent's independent role.
    f.role(f.channel, &f.agent, Some("admin")).await;
    sqlx::query("INSERT INTO community_bans (community_id,pubkey,banned,actor_pubkey) VALUES ($1,$2,true,$2)")
        .bind(f.community.as_uuid()).bind(f.owner.public_key().as_bytes())
        .execute(f.db.pool()).await.expect("ban");
    f.check(Some(&manage), false, false).await;
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn rebind_requires_direct_destination_admin_and_allows_delegated_source_admin() {
    let f = Fixture::new().await;
    f.role(f.channel, &f.agent, Some("admin")).await;
    f.role(f.next, &f.owner, Some("admin")).await;
    let manage = f.grant(GrantAction::Manage);
    let mut wildcard = manage.clone();
    wildcard.repo = None;
    for grant in [&manage, &wildcard] {
        assert!(
            matches!(
                f.prepare(Some(grant), true).await,
                Err(DbError::AccessDenied(_))
            ),
            "destination admin must be held directly with {grant:?}"
        );
    }

    f.role(f.channel, &f.agent, None).await;
    f.role(f.channel, &f.owner, Some("admin")).await;
    f.role(f.next, &f.agent, Some("admin")).await;
    assert!(matches!(
        f.prepare(None, true).await,
        Err(DbError::AccessDenied(_))
    ));
    assert_eq!(
        f.prepare(Some(&manage), true)
            .await
            .expect("delegated source with direct destination")
            .valid_until,
        manage.before.map(|at| at as i64)
    );

    f.role(f.channel, &f.agent, Some("admin")).await;
    let mut expired = manage;
    expired.before = Some(Timestamp::now().as_secs() - 1);
    assert_eq!(
        f.prepare(Some(&expired), true)
            .await
            .expect("direct admin of both channels")
            .valid_until,
        None
    );
}

#[tokio::test]
#[ignore = "requires Postgres"]
async fn closed_git_governance_requires_the_recorded_owners_admission() {
    let f = Fixture::new().await;
    // The agent is admitted itself and independently administers both channels.
    // Its unbanned owner is intentionally absent from relay_members.
    sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'member')")
        .bind(f.community.as_uuid())
        .bind(f.agent.public_key().to_hex())
        .execute(f.db.pool())
        .await
        .expect("admit agent");
    for ch in [f.channel, f.next] {
        f.role(ch, &f.agent, Some("admin")).await;
    }
    assert_eq!(
        f.admission_outcomes(false).await,
        [false; 4],
        "closed: owner not admitted (read, receipts, rules, bind)"
    );
    assert_eq!(
        f.admission_outcomes(true).await,
        [true; 4],
        "open community admits the unbanned owner"
    );
    sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'member')")
        .bind(f.community.as_uuid())
        .bind(f.owner.public_key().to_hex())
        .execute(f.db.pool())
        .await
        .expect("admit owner");
    assert_eq!(
        f.admission_outcomes(false).await,
        [true; 4],
        "closed community now admits the owner"
    );
    sqlx::query("DELETE FROM relay_members WHERE community_id=$1 AND pubkey=$2")
        .bind(f.community.as_uuid())
        .bind(f.owner.public_key().to_hex())
        .execute(f.db.pool())
        .await
        .expect("remove owner");
    assert_eq!(
        f.admission_outcomes(false).await,
        [false; 4],
        "removing owner admission revokes all Git policy routes"
    );
}
