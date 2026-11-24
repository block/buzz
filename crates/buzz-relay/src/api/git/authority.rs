//! Repository authority is the greater of direct channel membership and a
//! live owner grant capped by the owner's channel role. Admission and owner
//! bans are enforced before these helpers; the land checks them again under
//! its authority locks. Announcing a repository grants no authority.

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

/// Resolve a live grant and combine it with independent membership authority.
pub(crate) async fn credited_role(
    db: &buzz_db::Db,
    community: buzz_core::CommunityId,
    channel: Uuid,
    repo: &RepoCoordinate,
    key: &[u8],
    grant: Option<&GitGrant>,
    action: GrantAction,
) -> buzz_db::Result<Result<MemberRole, &'static str>> {
    let level = match db.get_agent_channel_policy(community, key).await? {
        Some((_, Some(owner))) => {
            db.git_live_grant_level(community, grant, repo, &owner)
                .await?
        }
        _ => None,
    };
    Ok(
        channel_role(db, community, channel, key, level.map(grant_role))
            .await?
            .and_then(|role| {
                if role.has_at_least(grant_role(action)) {
                    Ok(role)
                } else {
                    Err(NOT_A_MEMBER)
                }
            }),
    )
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
