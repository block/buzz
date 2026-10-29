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

The gateway reserves four relay shares within each destination delivery budget.
Each approved relay can admit at most one quarter of the configured cap per window;
renewing a grant does not reset usage. Quota exhaustion returns 429 and Retry-After,
and the relay keeps its durable request ID when rescheduling.
