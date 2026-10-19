# Authored-thread reply push notifications

Status: implemented; repository checks, physical delivery validation, and review
are in progress.

## Eligibility

A newly arriving message from another author qualifies when its thread root was
authored by the recipient. Nested replies retaining that root qualify, matching
desktop's root-authorship eligibility rule. Explicit follows, prior mentions,
and participation are not required.

The rule applies to new replies in both existing and newly created threads,
even when the receiving phone has never opened the thread. Enabling the feature
does not generate notifications for historical replies.

Existing direct-mention and DM behavior, access checks, notification controls,
channel mute behavior, and push suppression remain in force. Eligibility does
not guarantee OS delivery or lock-screen presentation.

## Delivery and scope

Preserve existing push grouping and coalescing. A burst of eligible replies does
not require a separate alert for each message; the current iOS resolver selects
the newest qualifying message after a wake. Overlapping eligibility reasons must
not introduce duplicate alerts.

This increment changes push eligibility only. Inbox parity, desktop changes,
participation-based eligibility, prior-mention automatic subscriptions, explicit
follow/unfollow behavior, and cross-device follow synchronization are out of scope.

## Acceptance

- Another author's direct reply to the recipient's thread root is eligible.
- A nested reply retaining that root is eligible.
- A reply to an unrelated root does not qualify through this new rule, even if
  the recipient authored an intermediate message or was previously mentioned.
- New replies to old threads qualify without prior phone viewing or fetching.
- Activation does not backfill historical notifications.
- Self-authored replies do not notify; overlapping mention/DM/thread eligibility
  does not introduce duplicate alerts.
- Authorization, notification controls, channel mutes, and suppression are
  exercised through real matching and native presentation paths.
- Tests distinguish eligibility from platform delivery and presentation.

Implementation and PR validation, including physical-device testing, are authorized.
Do not merge, roll out to production, or include in the conference release.

## Implementation contract

The relay advertises `buzz-authored-thread-replies-v1` alongside its existing
push capability. Supporting mobile clients add a single self-scoped
`authored_thread_replies: true` lease filter, with a persisted `since` timestamp
for the first activation. They do not enumerate or synchronize thread follows.
Old relays retain the existing mention/DM policy.

The existing lease capacity limits still apply. If the authored rule cannot
represent all channel mutes (more than 350) or no subscription slot remains,
the client omits that rule while continuing to synchronize mention/DM policies.
It never truncates mute exclusions or abandons the whole update. The authored
rule returns when capacity is available again.

The authenticated `/query` bridge accepts the same predicate and derives the
recipient from the authenticated identity. Historical query filtering and batch
wake matching share the database root-author relation, including community,
channel, exact event timestamp, and live-root checks. The query applies that
predicate before pagination and retains existing access controls.

The extension queries this relational policy separately from ordinary policies.
It trusts the authenticated relay response for the database relationship, while
still verifying event signatures, kind, activation time, ignore filters,
suppression, and recipient identity locally. An ordinary query response cannot
satisfy the relational policy. The extension also requires the relay response
header `x-buzz-query-extensions: buzz-authored-thread-replies-v1`, so an old
relay silently dropping the query field cannot broaden eligibility after a
rollback. Authored-thread queries cannot mix with other query modes. Existing
newest-message coalescing remains.

Validation still requires the full repository gates, physical-device testing,
and review approval before this work is complete.
