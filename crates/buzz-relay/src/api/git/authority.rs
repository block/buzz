//! Repository authority is the greater of direct channel membership and a
//! live owner grant capped by the owner's channel role. The read door checks
//! community and owner admission; the land rechecks under its authority locks.
//! Announcing a repository grants no authority.

use buzz_core::channel::MemberRole;
use buzz_core::governance::{GitGrant, GrantAction, RepoCoordinate};
use tracing::warn;
use uuid::Uuid;

/// Why a key is credited with nothing.
pub(crate) const NOT_A_MEMBER: &str = "not a channel member";

/// Resolve the hook's channel roles. Its HMAC authenticates the grant level;
/// the final land rechecks the grant's owner, repo, clock and floor.
pub(crate) async fn channel_role(
    db: &buzz_db::Db,
    community: buzz_core::CommunityId,
    channel: Uuid,
    key: &[u8],
    grant: Option<MemberRole>,
) -> buzz_db::Result<Result<MemberRole, &'static str>> {
    let own = db.get_member_role(community, channel, key).await?;
    let owner = match db.get_agent_channel_policy(community, key).await? {
        Some((_, Some(owner))) => Some(db.get_member_role(community, channel, &owner).await?),
        _ => None,
    };
    Ok(credit(own.as_deref(), owner.as_ref().map(|r| r.as_deref()), grant).ok_or(NOT_A_MEMBER))
}

/// Resolve read authority with current community and recorded-owner admission.
pub(crate) async fn credited_role(
    db: &buzz_db::Db,
    community: buzz_core::CommunityId,
    channel: Uuid,
    repo: &RepoCoordinate,
    key: &[u8],
    grant: Option<&GitGrant>,
    open: bool,
) -> buzz_db::Result<Result<MemberRole, &'static str>> {
    let mut tx = db.begin_event_write_transaction(community).await?;
    let facts = buzz_db::git_land::key_facts_for_read(&mut tx, channel, key).await?;
    if !facts.git_admitted(open) {
        return Ok(Err(NOT_A_MEMBER));
    }
    let level = match &facts.owner {
        Some((owner, _)) => {
            buzz_db::governance::live_grant_level(&mut tx, grant, repo, owner).await?
        }
        None => None,
    };
    let role = credit(
        facts.key.channel_role.as_deref(),
        facts
            .owner
            .as_ref()
            .map(|(_, owner)| owner.channel_role.as_deref()),
        level.map(grant_role),
    );
    tx.rollback().await?;
    Ok(role.ok_or(NOT_A_MEMBER))
}

/// Translate the three delegation levels to channel authority.
pub(crate) fn grant_role(level: GrantAction) -> MemberRole {
    match level {
        GrantAction::Read => MemberRole::Guest,
        GrantAction::Push => MemberRole::Member,
        GrantAction::Manage => MemberRole::Admin,
    }
}

/// Combine direct membership with a grant capped by the recorded owner's role.
/// Admission (including owner bans) is checked by the caller.
pub(crate) fn credit(
    own: Option<&str>,
    owner: Option<Option<&str>>,
    grant: Option<MemberRole>,
) -> Option<MemberRole> {
    let own = own.and_then(git_role);
    let delegated = owner
        .flatten()
        .and_then(git_role)
        .zip(grant)
        .map(|(owner, grant)| lesser(owner, grant));
    own.into_iter()
        .chain(delegated)
        .max_by_key(|r| r.permission_level())
}

/// The Git role a stored channel role carries. A bot is credited as a member;
/// an unrecognized role carries nothing.
fn git_role(stored: &str) -> Option<MemberRole> {
    match stored.parse::<MemberRole>() {
        Ok(MemberRole::Bot) => Some(MemberRole::Member),
        Ok(role) => Some(role),
        Err(_) => {
            warn!(role = %stored, "git authority: unrecognized channel role (deny)");
            None
        }
    }
}

fn lesser(a: MemberRole, b: MemberRole) -> MemberRole {
    if a.permission_level() <= b.permission_level() {
        a
    } else {
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_role_credits_bots_as_members_and_denies_unknown_roles() {
        use MemberRole::*;
        for (stored, want) in [
            ("owner", Some(Owner)),
            ("admin", Some(Admin)),
            ("member", Some(Member)),
            ("guest", Some(Guest)),
            ("bot", Some(Member)),
            ("superuser", None),
            ("", None),
        ] {
            assert_eq!(git_role(stored), want, "{stored:?}");
        }
    }

    #[test]
    fn credit_combines_independent_membership_and_capped_delegation() {
        use MemberRole::*;
        let roles = [
            None,
            Some("guest"),
            Some("member"),
            Some("admin"),
            Some("owner"),
            Some("bot"),
            Some("unknown"),
        ];
        for own in roles {
            for owner in std::iter::once(None).chain(roles.into_iter().map(Some)) {
                for grant in [None, Some(Guest), Some(Member), Some(Admin)] {
                    let direct = own.and_then(git_role);
                    let delegated = owner
                        .flatten()
                        .and_then(git_role)
                        .zip(grant)
                        .map(|(r, g)| lesser(r, g));
                    let got = credit(own, owner, grant);
                    for need in [Guest, Member, Admin, Owner] {
                        assert_eq!(
                            got.is_some_and(|r| r.has_at_least(need)),
                            direct.is_some_and(|r| r.has_at_least(need))
                                || delegated.is_some_and(|r| r.has_at_least(need)),
                            "own={own:?} owner={owner:?} grant={grant:?} need={need:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn lesser_takes_the_weaker_role_either_way() {
        use MemberRole::*;
        for (a, b, want) in [
            (Owner, Member, Member),
            (Member, Owner, Member),
            (Admin, Guest, Guest),
            (Admin, Admin, Admin),
        ] {
            assert_eq!(lesser(a, b), want, "{a:?} vs {b:?}");
        }
    }
}

#[cfg(test)]
mod postgres_tests {
    use super::*;
    use nostr::Keys;

    #[tokio::test]
    #[ignore = "requires PostgreSQL"]
    async fn read_role_requires_recorded_owner_admission() {
        let state = crate::handlers::group_state_postgres_tests::state().await;
        let tenant =
            crate::handlers::group_state_postgres_tests::community(&state, "git-read-owner").await;
        let agent = Keys::generate();
        let owner = Keys::generate();
        let channel = crate::handlers::group_state_postgres_tests::create_channel(
            &state,
            &tenant,
            &agent,
            &[],
        )
        .await;
        let cid = tenant.community();
        sqlx::query("INSERT INTO users (community_id,pubkey,agent_owner_pubkey) VALUES ($1,$2,$3) ON CONFLICT (community_id,pubkey) DO UPDATE SET agent_owner_pubkey=EXCLUDED.agent_owner_pubkey")
            .bind(cid.as_uuid())
            .bind(agent.public_key().as_bytes())
            .bind(owner.public_key().as_bytes())
            .execute(state.db.pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'member')")
            .bind(cid.as_uuid())
            .bind(agent.public_key().to_hex())
            .execute(state.db.pool())
            .await
            .unwrap();
        let repo = RepoCoordinate {
            owner: agent.public_key().to_hex(),
            name: "repo".into(),
        };
        let result = credited_role(
            &state.db,
            cid,
            channel,
            &repo,
            agent.public_key().as_bytes(),
            None,
            false,
        )
        .await
        .unwrap();
        assert!(
            result.is_err(),
            "directly admitted agent must not bypass its nonmember owner: {result:?}"
        );
        for (open, owner_member, allowed) in [
            (true, false, true),
            (false, true, true),
            (false, false, false),
        ] {
            sqlx::query("DELETE FROM relay_members WHERE community_id=$1 AND pubkey=$2")
                .bind(cid.as_uuid())
                .bind(owner.public_key().to_hex())
                .execute(state.db.pool())
                .await
                .unwrap();
            if owner_member {
                sqlx::query(
                    "INSERT INTO relay_members (community_id,pubkey,role) VALUES ($1,$2,'member')",
                )
                .bind(cid.as_uuid())
                .bind(owner.public_key().to_hex())
                .execute(state.db.pool())
                .await
                .unwrap();
            }
            let result = credited_role(
                &state.db,
                cid,
                channel,
                &repo,
                agent.public_key().as_bytes(),
                None,
                open,
            )
            .await
            .unwrap();
            assert_eq!(
                result.is_ok(),
                allowed,
                "open={open}, owner_member={owner_member}"
            );
        }
        // The read door is a snapshot, not a land. It must not acquire roster
        // row locks that can invert a membership-removal writer's lock order.
        let mut writer = state.db.pool().begin().await.unwrap();
        sqlx::query("SELECT 1 FROM relay_members WHERE community_id=$1 AND pubkey=$2 FOR UPDATE")
            .bind(cid.as_uuid())
            .bind(agent.public_key().to_hex())
            .fetch_one(&mut *writer)
            .await
            .unwrap();
        let read = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            credited_role(
                &state.db,
                cid,
                channel,
                &repo,
                agent.public_key().as_bytes(),
                None,
                true,
            ),
        )
        .await;
        writer.rollback().await.unwrap();
        assert!(read
            .expect("read authorization must not wait for roster row locks")
            .unwrap()
            .is_ok());
    }
}
