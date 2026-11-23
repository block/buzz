# Buzz push notes

Push is experimental. These notes capture current decisions; we will write a
stable specification after validating the design through launch and use.
[NIP-PL](nips/NIP-PL.md) is background, not the current Buzz contract.

- Relays advertise `buzz-push-v1`, never `nip-pl`. Updated clients require it.
- Compatible changes keep the capability name. Incompatible behavior that
  clients must distinguish gets a new version.
- Each gateway serves one server-configured Apple application and APNs transport.
  Enrollment, transcripts, grants, leases and discovery have no application-profile
  selector. The relay advertises transport classes in `class_support`.
- Existing grants and pending enrollment records are not migrated. The new client
  uses separate Keychain records and enrolls again. Keep old production push
  disabled so released clients never acquire legacy push authority.
- Stop old gateway replicas before cutover. The gateway migration requires an
  empty authority store because endpoint fingerprints changed. The relay migration
  requires no active legacy leases. Neither migration deletes that state.
- Publish the matching gateway image and chart, update deployment configuration,
  then enable the new relay capability. Verify enrollment, delivery, renewal and
  revocation on a physical device before production activation.

## Channel identifier grammar

Relays advertise `h_grammar: "uuid-v4-lowercase"`. Mobile also accepts
`uuid-lowercase`, which drops the UUID version and variant restriction so
UUIDv5 starter channels can be subscribed. Against a `uuid-v4-lowercase` relay,
mobile checks positive and ignore channel filters before enrollment or
publication. It keeps the desired filters and rechecks relay capabilities every
five minutes rather than sending a lease the relay would reject.

Do not change the relay advertisement until mobile builds that only parse
`uuid-v4-lowercase` have drained. Those builds parse the descriptor before
renewal, opt-out and revocation, so a new grammar would leave their existing
leases active until expiry.

## Workflow owner attribution

The relay signs workflow messages. Their first `p` tag,
`["p", <owner>, "", "owner"]`, names the workflow owner so clients can show
who owns the message. On an event marked `["buzz:workflow", "true"]` that tag
is attribution, not a mention: `#p` filters (live, history through the mention
index, and push leases), operator listeners and the push hellthread gate all
skip it, so the owner is not notified by every run of their workflow. When a
workflow mentions its owner, the relay adds a plain `["p", <owner>]`, which
notifies them like any mention. Mobile's Activity feed, unread mention priority
and in-app notification rule apply the same rule.

## Channel access

A lease wakes for a channel's events only when its author is a member, with
one exception: a message that `p`-tags a non-member wakes that non-member when
the channel is listed (open, not unlisted, a stream or forum). Workflow owner
attribution does not count as a mention. The relay checks this when it matches
and again before delivery, so a channel that becomes private or unlisted in
between stops the wake. Delivery also applies the connection gate to the
non-member: a ban on them or their agent's owner stops the wake, and so does
losing community membership or their allowlist entry when the relay requires
one. Private channels, unlisted channels, DMs and system channels still require
membership. Anyone in the community can already read a listed channel with
`REQ`, so the exception wakes no one for content they cannot read.
