# Agent profile identity and availability

## An explicit key is exact

Opening a public key from a message, member, DM, deep link, or Instances row
always opens that identity. Active, stopped, archived, and relay-only keys obey
the same rule. A local managed record may supply controls only for that exact
key. An owner-signed kind 30177 `persona_id` (or an archive request's historical
persona link) is **not** an identity alias and does not grant access to a local
sibling's controls, definition, configuration, or Start action.

Only explicit persona navigation (for example, the persona card in My Agents)
selects an archive-aware representative using `pickProfileAgent`. With no live
representative, it remains a persona-only surface and may offer Start. An
explicit relay-only key must not turn into that persona-only surface, even if a
matching definition exists locally. There is no synthetic/secretless
`ManagedAgent` record.

This intentionally supersedes the old historical-message redirect: a message
from stopped A no longer opens running B merely because they share persona P.
The requested key takes precedence even if the caller also supplies persona
context. Local instance profiles may still show their own linked definition and
an explicitly navigable Instances list. Ownership alone permits owner-scoped
relay reads, not local management.

Implementation: `useCanonicalManagedAgentProfile` and `UserProfilePanel`.
The obsolete historical-persona relay lookup and inactive-instance redirect
helper have been removed instead of adding another exception flag.

## Management provenance is not hosting location

**Not managed on this device** means the owned identity has no record in the
successfully loaded local managed inventory. It does not mean another physical
machine, cloud, or provider. Locally managed provider-backed agents do not get
this marker. Mentions (including unique names), member cards, and owned profiles
use the same cloud marker and accessible label. The cloud glyph is the owner-requested
visual convention, not proof of cloud hosting. The app-wide provenance context
also feeds hover cards, message authors, DM headers/sidebar rows, and focused
profile subviews without per-row directory subscriptions. Existing internal `managed-elsewhere` / `OtherSetup` names are
compatibility terms, not additional authority or a host registry.

The marker is presentation only: it does not establish mention eligibility,
channel membership, policy, or capability. Those remain independently verified.

## Presence and deployment are separate facts

The profile hero and managed-agent card availability dots consume the existing
relay presence query/subscription, not `ManagedAgent.status`. A successful
snapshot with no entry means offline. A pending/failed query or disconnected
relay means unknown; cached online data is not painted as online during a
reported disconnection. Normal presence TTL, polling, connection debounce, and
reconnection behavior are unchanged; presence can lag and is not substrate
telemetry. The profile omits an unknown dot; the card says **Availability
unknown** with a neutral dot when it is displaying lifecycle status rather than
an action.

`deployed` is a saved deployment receipt (`backend_agent_id`), retained after
shutdown. `running` is local runtime bookkeeping. Neither proves conversational
availability. Runtime diagnostics may still report these bookkeeping states.
`isManagedAgentActive` remains the lifecycle-action predicate, not a presence
predicate. Start/restart/error affordances and action routing are preserved;
this patch does not infer permission to redeploy from offline presence.

### Deliberately not solved here

A provider receipt still routes the primary control to Shutdown even when
presence is offline. Shutdown is a relay request, not a confirmed stop or a
substrate kill switch; its notice now says so. Choosing when to offer redeploy,
confirming termination, and handling failed bodies require a separate lifecycle
contract. There is no backend polling, host inventory, new cache, or remote-stop
guarantee in the profile/availability portion of this change. Mention discovery
and publication have their own ownership, policy, and membership gates.

## Regression gates

- `profile/lib/resolveCanonicalManagedAgent.test.mjs`: exact identity across
  active/stopped/archived/relay-only keys, and persona representative selection.
- `profile/lib/useCanonicalManagedAgentProfile.test.mjs`: remote A cannot borrow
  persona P/local B, including persona-only navigation and returning to A.
- `messages/ui/MentionAutocomplete.test.mjs`: unique and duplicate names render
  the same truthful management marker without changing target selection.
- `agents/lib/useAgentAvailability.test.mjs`: retained receipt with online,
  away, offline, absent, failed, and disconnected presence; controls unchanged.
- `tests/e2e/remote-agent-identity-ux.spec.ts`: relay-only profile controls,
  persona navigation, offline deployment and disconnected availability, with
  screenshots. `profile.spec.ts` intentionally expects historical messages to
  match the exact Instances selection rather than the current persona card.
