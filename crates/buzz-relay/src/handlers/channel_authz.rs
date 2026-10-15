//! Pure NIP-29 channel membership-authority decisions (kinds 9000/9001/9022).
//!
//! `validate_admin_event` in [`super::side_effects`] keeps every database read;
//! this module holds only the policy those reads feed. Each function takes
//! already-resolved data and returns a typed decision, so the authorization
//! rules are exhaustively unit-testable without Postgres or Redis — the same
//! shell/pure split [`super::moderation_authz`] uses for the moderation
//! capability grid.
//!
//! ## Error strings are the wire contract
//!
//! Every [`ChannelAuthzError`] message is returned to clients verbatim in the
//! NIP-29 `OK` frame. The variants deliberately preserve the two historically
//! distinct last-owner phrasings — [`ChannelAuthzError::LastOwnerRemoval`] for
//! the pre-storage validator and
//! [`ChannelAuthzError::LastOwnerRemovalTransferFirst`] for the side-effect
//! appliers. The *rule* is defined once in [`is_sole_owner`]; only the wording
//! differs per call site.

use buzz_db::channel::{MemberRecord, MemberRole};

/// A membership-authority denial. `Display` is the client-visible reason.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChannelAuthzError {
    /// Actor holds no authority for this action.
    #[error("actor not authorized")]
    ActorNotAuthorized,
    /// Actor tried to grant `owner`/`admin` without holding it.
    #[error("only owners/admins may grant elevated roles")]
    ElevatedRoleGrantDenied,
    /// Actor tried to change an active member's role without being elevated.
    #[error("only owners/admins may change an active member's role")]
    RoleChangeDenied,
    /// Demoting the channel's only owner would orphan it.
    #[error("cannot demote the last owner — transfer ownership first")]
    LastOwnerDemotion,
    /// Actor is not an active member of the channel.
    #[error("actor is not an active member")]
    NotActiveMember,
    /// Removing the channel's only owner would orphan it (validator wording).
    #[error("cannot remove the last owner")]
    LastOwnerRemoval,
    /// Removing the channel's only owner would orphan it (applier wording).
    #[error("cannot remove the last owner — transfer ownership first")]
    LastOwnerRemovalTransferFirst,
    /// `owner_only` policy on an agent with no owner recorded.
    #[error("policy:owner_only — agent has no owner set")]
    PolicyOwnerOnlyNoOwner,
    /// `owner_only` policy and the actor is not the agent's owner.
    #[error("policy:owner_only — only the agent owner can add this agent")]
    PolicyOwnerOnlyDenied,
    /// `nobody` policy — the agent has opted out of third-party adds.
    #[error("policy:nobody — this agent has disabled external channel additions")]
    PolicyNobody,
}

/// Whether `pubkey` is the channel's only remaining `owner`.
///
/// This is the single definition of last-owner protection. Every call site
/// that once restated it — kind:9000 demotion, kind:9001 self-removal,
/// kind:9022 leave, and the `handle_remove_user` / `handle_leave_request`
/// appliers — asks this one question and supplies its own wording.
///
/// `members` must already be filtered to *active* membership; both
/// `get_members` and `get_members_for_event_write` are, so a soft-removed
/// owner row never counts toward the roster.
pub fn is_sole_owner(members: &[MemberRecord], pubkey: &[u8]) -> bool {
    let mut owners = members.iter().filter(|m| m.role == "owner");
    let sole = matches!(owners.next(), Some(first) if first.pubkey == pubkey);
    sole && owners.next().is_none()
}

/// Decide whether `actor` may remove themselves from the channel.
///
/// Shared by kind:9001 self-removal and kind:9022 leave — they enforced
/// character-identical rules and messages before this seam existed.
pub fn decide_self_departure(
    members: &[MemberRecord],
    actor: &[u8],
) -> Result<(), ChannelAuthzError> {
    if !members.iter().any(|m| m.pubkey == actor) {
        return Err(ChannelAuthzError::NotActiveMember);
    }
    if is_sole_owner(members, actor) {
        return Err(ChannelAuthzError::LastOwnerRemoval);
    }
    Ok(())
}

/// The outcome of a kind:9000 membership-authority check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PutUserDecision {
    /// Authorized outright. A self-add bypasses the target's agent
    /// `channel_add_policy` — you may always add yourself.
    Allow,
    /// Authorized so far; the caller must still load and evaluate the target's
    /// agent `channel_add_policy` via [`decide_channel_add_policy`].
    CheckAddPolicy,
}

/// Decide whether `actor` may add `target` to the channel, or change the role
/// `target` already holds (NIP-29 kind:9000 PUT_USER).
///
/// `actor_role` and `members` must come from an *active* membership read;
/// `requested_role` is `None` when the event carries no `role` tag, which
/// means "no role change requested" rather than "demote to member".
///
/// The database read for the target's agent channel-add policy stays with the
/// caller: this returns [`PutUserDecision::CheckAddPolicy`] when that read is
/// still required.
pub fn decide_put_user(
    visibility: &str,
    actor_role: Option<MemberRole>,
    requested_role: Option<MemberRole>,
    members: &[MemberRecord],
    target: &[u8],
    actor: &[u8],
) -> Result<PutUserDecision, ChannelAuthzError> {
    // Open channels allow any authenticated user; private channels require the
    // actor to be an existing active member. Any active member may add an
    // ordinary member, guest, or bot, but only owners/admins may grant an
    // elevated role.
    if visibility == "private" {
        if actor_role.is_none() {
            return Err(ChannelAuthzError::ActorNotAuthorized);
        }

        if requested_role.is_some_and(|role| role.is_elevated())
            && !actor_role.is_some_and(|role| role.is_elevated())
        {
            return Err(ChannelAuthzError::ElevatedRoleGrantDenied);
        }
    }

    // Changing an ACTIVE existing member's role is privileged in both
    // directions, on every visibility. `members` comes from a `removed_at IS
    // NULL` read, so a soft-removed row is deliberately not an "existing
    // member" here: its stored role is history, not live authority, and
    // reactivation is governed by the elevated-granter check above rather than
    // by the role the row remembers.
    //
    // `add_member` is the authority (it also covers the desktop/admin callers
    // that skip this validator); rejecting here too means the client gets a
    // real error instead of an OK for an event whose side effect then fails.
    // Re-adding at the same role stays idempotent — the huddle bot-add path
    // relies on that.
    if let Some((existing, role)) = members
        .iter()
        .find(|m| m.pubkey == target)
        .zip(requested_role)
        .filter(|(m, role)| m.role != role.as_str())
    {
        if !actor_role.is_some_and(|r| r.is_elevated()) {
            return Err(ChannelAuthzError::RoleChangeDenied);
        }
        if existing.role == "owner" && role != MemberRole::Owner && is_sole_owner(members, target) {
            return Err(ChannelAuthzError::LastOwnerDemotion);
        }
    }

    // Self-add: always allowed regardless of the target's agent policy.
    if target == actor {
        return Ok(PutUserDecision::Allow);
    }

    Ok(PutUserDecision::CheckAddPolicy)
}

/// Evaluate a target agent's `channel_add_policy` for a third-party add.
///
/// `policy` and `owner` come from `get_agent_channel_policy`; callers skip
/// this entirely when the target has no policy row, or when the add is a
/// self-add ([`PutUserDecision::Allow`]).
///
/// Unknown policy values allow. The database enum prevents them from being
/// stored, so this is defence in depth rather than reachable behaviour — if a
/// new value is added to the enum, extend this match.
pub fn decide_channel_add_policy(
    policy: &str,
    owner: Option<&[u8]>,
    actor: &[u8],
) -> Result<(), ChannelAuthzError> {
    match policy {
        "owner_only" => {
            let owner = owner.ok_or(ChannelAuthzError::PolicyOwnerOnlyNoOwner)?;
            if actor != owner {
                return Err(ChannelAuthzError::PolicyOwnerOnlyDenied);
            }
            Ok(())
        }
        "nobody" => Err(ChannelAuthzError::PolicyNobody),
        _ => Ok(()),
    }
}

/// What authority `actor` holds to remove *somebody else* (kind:9001).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoveOtherDecision {
    /// Channel owner or admin — authorized for any target.
    Allow,
    /// An active non-elevated member. Authorized only if they own the target
    /// agent, which the caller must confirm with a database read.
    CheckAgentOwner,
    /// Not an active member. Denied without any further read — you must be in
    /// the channel to remove anyone, even your own bot.
    Deny,
}

/// Classify `actor`'s authority to remove another member (NIP-29 kind:9001).
pub fn classify_remove_other(members: &[MemberRecord], actor: &[u8]) -> RemoveOtherDecision {
    match members.iter().find(|m| m.pubkey == actor) {
        Some(m) if m.role == "owner" || m.role == "admin" => RemoveOtherDecision::Allow,
        Some(_) => RemoveOtherDecision::CheckAgentOwner,
        None => RemoveOtherDecision::Deny,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    /// Build a member roster from `(pubkey_byte, role)` pairs.
    fn roster(entries: &[(u8, &str)]) -> Vec<MemberRecord> {
        let channel_id = Uuid::new_v4();
        entries
            .iter()
            .map(|(tag, role)| MemberRecord {
                channel_id,
                pubkey: vec![*tag; 32],
                role: (*role).to_string(),
                joined_at: Utc::now(),
                invited_by: None,
                removed_at: None,
            })
            .collect()
    }

    fn pk(tag: u8) -> Vec<u8> {
        vec![tag; 32]
    }

    /// A roster literal: `(pubkey_tag, role)` pairs.
    type Roster<'a> = &'a [(u8, &'a str)];
    /// `(roster, subject, expected)`.
    type SoleOwnerCase<'a> = (Roster<'a>, u8, bool);
    /// `(roster, actor, expected_error)`.
    type DepartureCase<'a> = (Roster<'a>, u8, Option<ChannelAuthzError>);
    /// `(roster, actor, expected_decision)`.
    type RemoveOtherCase<'a> = (Roster<'a>, u8, RemoveOtherDecision);
    /// `(policy, owner_tag, actor, expected_error)`.
    type AddPolicyCase<'a> = (&'a str, Option<u8>, u8, Option<ChannelAuthzError>);

    /// The single definition of last-owner protection, over the full shape
    /// space the five former call sites covered.
    #[test]
    fn sole_owner_table() {
        let cases: &[SoleOwnerCase] = &[
            // Only owner, and it is the subject.
            (&[(1, "owner")], 1, true),
            (&[(1, "owner"), (2, "member")], 1, true),
            (&[(1, "owner"), (2, "admin"), (3, "bot")], 1, true),
            // Only owner, but the subject is somebody else.
            (&[(1, "owner"), (2, "member")], 2, false),
            (&[(1, "owner")], 2, false),
            // Two owners: neither is sole.
            (&[(1, "owner"), (2, "owner")], 1, false),
            (&[(1, "owner"), (2, "owner")], 2, false),
            (&[(1, "owner"), (2, "owner"), (3, "member")], 3, false),
            // No owners at all.
            (&[(1, "member"), (2, "admin")], 1, false),
            (&[], 1, false),
            // An admin is not an owner.
            (&[(1, "admin")], 1, false),
        ];

        for (entries, subject, expected) in cases {
            let members = roster(entries);
            assert_eq!(
                is_sole_owner(&members, &pk(*subject)),
                *expected,
                "roster {entries:?} subject {subject}"
            );
        }
    }

    /// kind:9001 self-removal and kind:9022 leave share one rule: the actor
    /// must be an active member, and must not be the channel's only owner.
    #[test]
    fn self_departure_table() {
        let cases: &[DepartureCase] = &[
            // Non-member cannot leave.
            (&[(1, "owner")], 9, Some(ChannelAuthzError::NotActiveMember)),
            (&[], 1, Some(ChannelAuthzError::NotActiveMember)),
            // Sole owner is pinned.
            (
                &[(1, "owner"), (2, "member")],
                1,
                Some(ChannelAuthzError::LastOwnerRemoval),
            ),
            (
                &[(1, "owner")],
                1,
                Some(ChannelAuthzError::LastOwnerRemoval),
            ),
            // Co-owner may leave.
            (&[(1, "owner"), (2, "owner")], 1, None),
            // Non-owner roles may always leave.
            (&[(1, "owner"), (2, "member")], 2, None),
            (&[(1, "owner"), (2, "admin")], 2, None),
            (&[(1, "owner"), (2, "bot")], 2, None),
            (&[(1, "owner"), (2, "guest")], 2, None),
        ];

        for (entries, actor, expected) in cases {
            let members = roster(entries);
            assert_eq!(
                decide_self_departure(&members, &pk(*actor)).err(),
                *expected,
                "roster {entries:?} actor {actor}"
            );
        }
    }

    /// kind:9000 authorization, over visibility × actor role × requested role
    /// × existing-target shape. Ordering matters: the private-channel gates
    /// run before the role-change gates, which run before the self-add
    /// shortcut.
    #[test]
    fn put_user_table() {
        use ChannelAuthzError as E;
        use MemberRole::{Admin, Member, Owner};
        use PutUserDecision::{Allow, CheckAddPolicy};

        // (visibility, roster, actor, actor_role, target, requested_role, expected)
        type Case<'a> = (
            &'a str,
            &'a [(u8, &'a str)],
            u8,
            Option<MemberRole>,
            u8,
            Option<MemberRole>,
            Result<PutUserDecision, E>,
        );
        let cases: &[Case] = &[
            // ── Private channels require the actor to be an active member ──
            (
                "private",
                &[(1, "owner")],
                9,
                None,
                5,
                Some(Member),
                Err(E::ActorNotAuthorized),
            ),
            // Even a self-add cannot bootstrap membership into a private channel.
            (
                "private",
                &[(1, "owner")],
                9,
                None,
                9,
                None,
                Err(E::ActorNotAuthorized),
            ),
            // ── Private: only elevated actors may grant elevated roles ──
            (
                "private",
                &[(1, "owner"), (2, "member")],
                2,
                Some(Member),
                5,
                Some(Admin),
                Err(E::ElevatedRoleGrantDenied),
            ),
            (
                "private",
                &[(1, "owner"), (2, "member")],
                2,
                Some(Member),
                5,
                Some(Owner),
                Err(E::ElevatedRoleGrantDenied),
            ),
            // An elevated actor may grant an elevated role.
            (
                "private",
                &[(1, "owner")],
                1,
                Some(Owner),
                5,
                Some(Admin),
                Ok(CheckAddPolicy),
            ),
            (
                "private",
                &[(1, "owner"), (2, "admin")],
                2,
                Some(Admin),
                5,
                Some(Admin),
                Ok(CheckAddPolicy),
            ),
            // A plain member may still add an ordinary member to a private channel.
            (
                "private",
                &[(1, "owner"), (2, "member")],
                2,
                Some(Member),
                5,
                Some(Member),
                Ok(CheckAddPolicy),
            ),
            // ── Open channels skip both private gates entirely ──
            (
                "open",
                &[(1, "owner")],
                9,
                None,
                5,
                Some(Member),
                Ok(CheckAddPolicy),
            ),
            (
                "open",
                &[(1, "owner")],
                9,
                None,
                5,
                Some(Admin),
                Ok(CheckAddPolicy),
            ),
            // ── Changing an ACTIVE member's role is privileged on every visibility ──
            (
                "open",
                &[(1, "owner"), (2, "member")],
                9,
                None,
                2,
                Some(Admin),
                Err(E::RoleChangeDenied),
            ),
            (
                "open",
                &[(1, "owner"), (2, "member"), (3, "member")],
                3,
                Some(Member),
                2,
                Some(Admin),
                Err(E::RoleChangeDenied),
            ),
            // Demotion is privileged in the same way as promotion.
            (
                "open",
                &[(1, "owner"), (2, "admin"), (3, "member")],
                3,
                Some(Member),
                2,
                Some(Member),
                Err(E::RoleChangeDenied),
            ),
            // An elevated actor may change roles.
            (
                "open",
                &[(1, "owner"), (2, "member")],
                1,
                Some(Owner),
                2,
                Some(Admin),
                Ok(CheckAddPolicy),
            ),
            // ── Last-owner demotion guard ──
            // The sole owner demoting themselves.
            (
                "open",
                &[(1, "owner"), (2, "member")],
                1,
                Some(Owner),
                1,
                Some(Member),
                Err(E::LastOwnerDemotion),
            ),
            // Another owner demoting the sole owner is impossible (they'd be an
            // owner too), but an admin demoting the sole owner is not.
            (
                "open",
                &[(1, "owner"), (2, "admin")],
                2,
                Some(Admin),
                1,
                Some(Member),
                Err(E::LastOwnerDemotion),
            ),
            // With a co-owner present the demotion is allowed.
            (
                "open",
                &[(1, "owner"), (2, "owner")],
                1,
                Some(Owner),
                2,
                Some(Member),
                Ok(CheckAddPolicy),
            ),
            // Owner → Owner is not a demotion, so the guard does not fire; it is
            // also not a role change, so it short-circuits as idempotent.
            (
                "open",
                &[(1, "owner")],
                1,
                Some(Owner),
                1,
                Some(Owner),
                Ok(Allow),
            ),
            // ── Re-adding at the same role stays idempotent (huddle bot path) ──
            (
                "open",
                &[(1, "owner"), (2, "bot")],
                9,
                None,
                2,
                Some(MemberRole::Bot),
                Ok(CheckAddPolicy),
            ),
            // An absent role tag requests no change, so the role-change gate
            // never fires even for an unprivileged actor.
            (
                "open",
                &[(1, "owner"), (2, "member")],
                9,
                None,
                2,
                None,
                Ok(CheckAddPolicy),
            ),
            // ── Self-add short-circuits the agent channel-add policy ──
            ("open", &[(1, "owner")], 9, None, 9, Some(Member), Ok(Allow)),
            (
                "open",
                &[(1, "owner"), (2, "member")],
                2,
                Some(Member),
                2,
                Some(Member),
                Ok(Allow),
            ),
            (
                "private",
                &[(1, "owner"), (2, "member")],
                2,
                Some(Member),
                2,
                None,
                Ok(Allow),
            ),
        ];

        for (visibility, entries, actor, actor_role, target, requested_role, expected) in cases {
            let members = roster(entries);
            assert_eq!(
                decide_put_user(
                    visibility,
                    *actor_role,
                    *requested_role,
                    &members,
                    &pk(*target),
                    &pk(*actor),
                ),
                *expected,
                "visibility {visibility} roster {entries:?} actor {actor} target {target} requested {requested_role:?}"
            );
        }
    }

    /// The target's agent `channel_add_policy`, evaluated for a third-party
    /// add. Self-adds never reach here.
    #[test]
    fn channel_add_policy_table() {
        use ChannelAuthzError as E;

        // (policy, owner, actor, expected)
        let cases: &[AddPolicyCase] = &[
            // "anyone" allows any actor.
            ("anyone", None, 7, None),
            ("anyone", Some(1), 7, None),
            // "nobody" blocks every actor, including the agent's own owner.
            ("nobody", Some(7), 7, Some(E::PolicyNobody)),
            ("nobody", None, 7, Some(E::PolicyNobody)),
            // "owner_only" admits exactly the configured owner.
            ("owner_only", Some(7), 7, None),
            ("owner_only", Some(1), 7, Some(E::PolicyOwnerOnlyDenied)),
            // "owner_only" with no owner recorded is a misconfiguration, and
            // fails closed with its own distinct message.
            ("owner_only", None, 7, Some(E::PolicyOwnerOnlyNoOwner)),
            // Unknown values fall through to allow; the DB enum prevents them
            // from being stored, so this is defence in depth.
            ("something_new", None, 7, None),
            ("", None, 7, None),
        ];

        for (policy, owner, actor, expected) in cases {
            let owner_bytes = owner.map(pk);
            assert_eq!(
                decide_channel_add_policy(policy, owner_bytes.as_deref(), &pk(*actor)).err(),
                *expected,
                "policy {policy} owner {owner:?} actor {actor}"
            );
        }
    }

    /// kind:9001 removal of somebody else. A plain member is not denied
    /// outright — they may still own the target agent, which requires a
    /// database read the caller performs.
    #[test]
    fn remove_other_table() {
        use RemoveOtherDecision::{Allow, CheckAgentOwner, Deny};

        let cases: &[RemoveOtherCase] = &[
            // Owners and admins may remove anyone.
            (&[(1, "owner"), (2, "member")], 1, Allow),
            (&[(1, "owner"), (2, "admin")], 2, Allow),
            // A plain member, guest, or bot may only remove an agent they own.
            (&[(1, "owner"), (2, "member")], 2, CheckAgentOwner),
            (&[(1, "owner"), (2, "guest")], 2, CheckAgentOwner),
            (&[(1, "owner"), (2, "bot")], 2, CheckAgentOwner),
            // Non-members are denied without an agent-owner read: you must be
            // in the channel to remove anyone, even your own bot.
            (&[(1, "owner")], 9, Deny),
            (&[], 9, Deny),
        ];

        for (entries, actor, expected) in cases {
            let members = roster(entries);
            assert_eq!(
                classify_remove_other(&members, &pk(*actor)),
                *expected,
                "roster {entries:?} actor {actor}"
            );
        }
    }

    /// The wire contract: these strings reach NIP-29 clients verbatim, and the
    /// two last-owner phrasings are deliberately distinct.
    #[test]
    fn error_strings_are_the_wire_contract() {
        let cases = [
            (
                ChannelAuthzError::ActorNotAuthorized,
                "actor not authorized",
            ),
            (
                ChannelAuthzError::ElevatedRoleGrantDenied,
                "only owners/admins may grant elevated roles",
            ),
            (
                ChannelAuthzError::RoleChangeDenied,
                "only owners/admins may change an active member's role",
            ),
            (
                ChannelAuthzError::LastOwnerDemotion,
                "cannot demote the last owner — transfer ownership first",
            ),
            (
                ChannelAuthzError::NotActiveMember,
                "actor is not an active member",
            ),
            (
                ChannelAuthzError::LastOwnerRemoval,
                "cannot remove the last owner",
            ),
            (
                ChannelAuthzError::LastOwnerRemovalTransferFirst,
                "cannot remove the last owner — transfer ownership first",
            ),
            (
                ChannelAuthzError::PolicyOwnerOnlyNoOwner,
                "policy:owner_only — agent has no owner set",
            ),
            (
                ChannelAuthzError::PolicyOwnerOnlyDenied,
                "policy:owner_only — only the agent owner can add this agent",
            ),
            (
                ChannelAuthzError::PolicyNobody,
                "policy:nobody — this agent has disabled external channel additions",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }
}
