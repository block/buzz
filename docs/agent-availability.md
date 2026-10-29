# Agent availability and lifecycle

Availability means conversational presence on the relay, not process health.
A retained deployment receipt, PID, or `running` record cannot turn an agent's
availability dot green. A successful presence snapshot with no entry means
Offline **only for an identity included in that snapshot’s requested keys**.
An unqueried identity is unknown, never implicitly Offline. An initial pending
read, failed read, or disconnected relay means unknown, including
when the cache previously contained Online. Online and Away require presence.

Agents cards and profiles share the existing presence query and live subscription;
the Agents action owner queries its full managed set once and passes its reader
to persona/custom/unknown cards. Members owns one full-roster query shared by
rows, menus and bulk actions. The profile's actions use its existing snapshot,
and its popover also preserves failed/disconnected reads as unknown. Native
query errors reject the IPC call rather than fabricating an empty snapshot.
A live update or self heartbeat cannot heal a failed aggregate snapshot by
marking cached siblings successful; those reads remain unknown until a
successful snapshot retry. No per-row polling is needed, and
there is no second availability cache or substrate poller. Lifecycle controls
remain separate: a deployed provider agent still offers Shutdown while offline.
Shutdown sends a request, not a confirmed termination, and absence of presence
is not permission to deploy a duplicate body. Local Stop/Start routing is unchanged.

A locally stopped record with current exact-key Online or Away presence does
not establish local process ownership. Its card replaces Start (including a
stale restart/error badge) with the presence dot; its profile and member menu
retain the lifecycle-derived Start label but disable activation with an
explanation. No Stop is invented. Local/pair-owned running controls keep their
existing Stop/Restart routing, even when presence is Offline. The same positive
presence guard covers list/profile callbacks and member bulk respawn so the
visible affordance is not the only fence. This is a UI startup guard, **not** a
distributed singleton lock: missing, expired, failed or disconnected presence
cannot certify that starting another body is safe. Existing relay query/live
subscription freshness remains authoritative; no separate heartbeat or cache
is introduced. Runtime details remain accessible through the card/profile.


## Lifecycle decision authority

Presence does not grant management authority. Backend type, deployment receipt,
local/pair ownership and the existing owner gates still decide which operations
are offered and where they route. All presence-dependent decisions use the same
surface-owned reader, not raw query data. The reader checks the canonical query
cache and current relay connection at invocation, including after asynchronous
channel discovery and between persona siblings. Display observers refresh on
query data/status and connection changes (without the warning-banner debounce).
A successful cached snapshot remains usable during an in-flight background
refetch under the existing query policy; a settled error revokes it even though
TanStack retains the old data. A live single-author update cannot heal a failed
aggregate. This is not a new freshness cache or a distributed lock.

For deletion of a provider agent with a deployment receipt:

| Availability / route | Before local record deletion |
| --- | --- |
| Online or Away, channel available | Await a shutdown request; warn it does not confirm termination. |
| Unknown, channel available | Also await shutdown; explicitly identify unknown availability in the action confirmation, never claim Offline. |
| Established Offline | Preserve intentional offline removal without a shutdown request; warn the remote deployment may still exist. |
| No channel | No shutdown route; warn the remote deployment may still run, without claiming it will. |

A shutdown request failure propagates and leaves the management record and
channel memberships intact for retry; it is not silently converted into force
removal. A successful request still requires the existing orphan confirmation
(or the profile’s already-accepted equivalent) before forced **local record**
deletion. The profile confirmation describes this policy without asserting the
current availability or promising remote deletion. Cancelling after a request
retains the record; it cannot unsend the request. Channel removal follows record
deletion. Local deletion does not consult presence and still delegates to the
native stop-before-remove boundary. Custom-persona native cascade continues to
refuse provider-deployed targets; built-in persona instance removal uses these
same per-instance rules, treating unqueried siblings as unknown.


## Regression evidence

- `desktop/src-tauri/src/commands/profile_presence_tests.rs`: actual command
  against loopback HTTP; empty success versus auth/rate-limit/storage/malformed
  response and transport failure, with authenticated kind-20001 requests.
- `UnifiedAgentsSectionCardTarget.test.mjs`: three card types use one request
  and one polling observer, reordered while the snapshot is in flight; exact-key
  live updates, failed warm-cache poll, retry, changed author set and unmount.
- `agent-availability.spec.ts`: real Agents/Members surfaces request-count check;
  failure uses the native string rejection shape, not a fabricated success.

- `desktop/src/features/agents/lib/useAgentAvailability.test.mjs`: successful,
  missing, unavailable, and disconnected presence; lifecycle routing and badges.
- `desktop/tests/e2e/agent-availability.spec.ts`: deployed/offline profile and
  card, stopped/Online and Away presence, disabled keyboard/click Start in
  profile and member menu, restored Offline startup, running-but-Offline,
  live updates and disconnected-state behavior. Its real hover-popover journey
  holds the single-key IPC read pending, then distinguishes successful omission
  and explicit Offline from failed cached Online and disconnection, checking
  visible badges plus accessible names and screen-reader status text. Restoring
  the popover's Offline fallback must fail this regression.
- `desktop/src/features/agents/ui/UnifiedAgentsSectionCardTarget.test.mjs`:
  mounted persona/custom/unknown cards, exact-key versus other-key presence,
  Online/Away/offline/missing transitions and actual Start callback activation.
- `desktop/tests/e2e/agents.spec.ts`: Start badge animation and avatar continuity,
  runtime-only negative control, then authored Online presence.
- `desktop/tests/e2e/profile.spec.ts`: created-agent initial Online snapshot and
  live Offline/Online transitions alongside independent Stop/Start controls.
- `desktop/tests/e2e/onboarding.spec.ts`: Welcome kickoff stays pending after
  runtime start alone, then greets the owner and addresses Honey/Pollen only
  after the scenario publishes their explicit presence. Healthy-team scenarios
  use `tests/helpers/welcomeTeam.ts`, without changing the product readiness wait.

Mock create/start/stop only model runtime bookkeeping. Scenarios that require
availability use `__BUZZ_E2E_EMIT_MOCK_PRESENCE__` to seed the snapshot and emit
kind 20001 with the agent as author. Updating a directory row or posting a chat
message is not presence. Mock browser tests do not certify native relay TTL,
provider termination, or substrate health.

- `managedAgentDeletionAvailability.test.mjs`: mounted production Agents and
  profile deletion hooks through safe mock IPC, including failed/disconnected
  cached Online **and Offline**, pending, genuine missing/Offline, Online/Away,
  channel-discovery suspension, unqueried persona siblings, successful in-flight
  refetch, shutdown failure/ordering/cancel, no-channel and local delegation.
- `agent-availability.spec.ts`: real profile Delete dialog/action with failed or
  disconnected cached Online and failed cached Offline. A rejected shutdown
  retains the record; retry orders shutdown → deletion → membership removal.
  Genuine Offline preserves no-request deletion. All destructive IPC is mocked.
