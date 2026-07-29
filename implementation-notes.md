# Implementation Notes

## 2026-07-29

- Keep Bookmark Bee and other managed agents in `owner-only` mode. The fix must
  not broaden the inbound author gate to every channel author.
- Add an explicit `actor` tag to relay-signed workflow messages while retaining
  the existing first `p` attribution tag for compatibility with current
  message consumers.
- Treat a workflow actor as authoritative in `buzz-acp` only when all of these
  checks pass:
  - the event signer matches the relay `self` pubkey fetched from NIP-11;
  - the event signature verifies locally;
  - the event is a stream message;
  - there is exactly one `["buzz:workflow", "true"]` tag;
  - there is exactly one valid 64-hex `actor` tag.
- If NIP-11 is unavailable or malformed, fail closed for workflow attribution.
  Normal user-signed messages continue to use `event.pubkey`.
- Apply the same effective-author rule in normal and setup-listener modes so a
  scheduled workflow receives either a real agent turn or an honest setup
  nudge.
- Existing relay-signed workflow events without the new `actor` tag remain
  rejected by `owner-only`. The relay and ACP harness both need the new version
  before scheduled wakeups work.
- Deployment order is safe either way. Until both components are updated, the
  current fail-closed behavior remains.
- Reject malformed duplicate `actor` or `buzz:workflow` tags, not just duplicate
  well-formed tags. Trusting one valid tag while ignoring a second ambiguous tag
  would weaken the "exactly one" contract.
- The Nostr pubkey parser accepts some surprising 64-hex fixtures such as all
  zeroes and all `f`s, so malformed NIP-11 tests use wrong-length/non-hex input.
  The trust boundary requires canonical 64-hex plus an event signature that
  verifies under the corresponding relay key.
- Full `just ci` passed on the implementation worktree. `just test` completed
  its unit phase, but the integration phase could not start because the local
  Docker Desktop daemon was unavailable at
  `/Users/justinperea/.docker/run/docker.sock`. The relay's new fast unit test
  and all 636 `buzz-acp` library tests passed without Docker.
