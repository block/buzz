//! The role the relay credits a key with in a channel-bound repository.
//!
//! One function, shared by the read gate, the push policy and repository
//! management, so the three can never disagree about who holds what:
//!
//! ```text
//!   key ──► its role in the repo's bound channel ─┐
//!                                                 ├─► min ──► credited role
//!   agent ──► its recorded owner's role there ────┘
//! ```
//!
//! Authority comes only from the bound channel. Announcing the repository,
//! owning the agent that announced it, or being named in its event grants
//! nothing. An agent with a recorded owner acts only under a live Git grant
//! from that owner, presented with the request, with the lesser of its own
//! role and its owner's, so it never holds more than its owner currently
//! does, and loses access when its owner does or the grant ends. Bans are
//! enforced before this runs, on every Git request, and cascade from owner
//! to agent.

use buzz_core::channel::MemberRole;
use buzz_core::governance::{GitGrant, GrantAction, RepoCoordinate};
use tracing::warn;
use uuid::Uuid;

/// Why a key is credited with nothing.
pub(crate) const NOT_A_MEMBER: &str = "not a channel member";
/// Why an agent is credited with nothing although it and its owner are members.
pub(crate) const NO_GRANT: &str = "no live grant from the agent's owner";

/// The role `key` is credited with in `channel`, with its recorded owner when
/// it is an agent, or why it may not touch the repository at all. An agent's
/// grant is not judged here: see [`credited_role`].
pub(crate) async fn channel_role(
    db: &buzz_db::Db,
    community: buzz_core::CommunityId,
    channel: Uuid,
    key: &[u8],
) -> buzz_db::Result<Result<(MemberRole, Option<Vec<u8>>), &'static str>> {
    let own = db.get_member_role(community, channel, key).await?;
    let owner = match db.get_agent_channel_policy(community, key).await? {
        Some((_, Some(owner))) => Some((
            owner.clone(),
            db.get_member_role(community, channel, &owner).await?,
        )),
        _ => None,
    };
    Ok(
        match credit(own.as_deref(), owner.as_ref().map(|(_, r)| r.as_deref())) {
            Some(role) => Ok((role, owner.map(|(owner, _)| owner))),
            None => Err(NOT_A_MEMBER),
        },
    )
}

/// The role `key` is credited with in `repo`, bound to `channel`, for
/// `action`: its [`channel_role`], and for an agent only under `grant`, the
/// Git grant it presented, signed by its recorded owner.
pub(crate) async fn credited_role(
    db: &buzz_db::Db,
    community: buzz_core::CommunityId,
    channel: Uuid,
    repo: &RepoCoordinate,
    key: &[u8],
    grant: Option<&GitGrant>,
    action: GrantAction,
) -> buzz_db::Result<Result<MemberRole, &'static str>> {
    let (role, owner) = match channel_role(db, community, channel, key).await? {
        Ok(credited) => credited,
        Err(reason) => return Ok(Err(reason)),
    };
    if let Some(owner) = owner {
        if !db
            .git_live_grant(community, grant, repo, &owner, action)
            .await?
        {
            return Ok(Err(NO_GRANT));
        }
    }
    Ok(Ok(role))
}

/// The credited role from stored channel roles: the key's own, and for an
/// agent with a recorded owner, the owner's (`Some(None)` when the owner holds
/// none). The governed land credits with this too, from rows it has locked.
///
/// A bot is credited as a member: bot is what a key is, not what it may do.
/// An unrecognized stored role credits nothing (fail closed).
pub(crate) fn credit(own: Option<&str>, owner: Option<Option<&str>>) -> Option<MemberRole> {
    let own = git_role(own?)?;
    match owner {
        None => Some(own),
        Some(theirs) => Some(lesser(own, git_role(theirs?)?)),
    }
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
    fn credit_caps_an_agent_by_its_owner_and_needs_both() {
        use MemberRole::*;
        for (own, owner, want) in [
            (Some("admin"), None, Some(Admin)),
            (None, None, None),
            (Some("admin"), Some(Some("member")), Some(Member)),
            (Some("member"), Some(Some("owner")), Some(Member)),
            (Some("admin"), Some(None), None),
            (None, Some(Some("owner")), None),
            (Some("bot"), Some(Some("admin")), Some(Member)),
            (Some("admin"), Some(Some("superuser")), None),
        ] {
            assert_eq!(credit(own, owner), want, "{own:?} under {owner:?}");
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
