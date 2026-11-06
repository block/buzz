# RFC: community-managed external agents with an HTTP runtime adapter

Status: proposal for maintainer discussion; not an implemented feature or a new standard.
Tracking discussion: [#9220](https://github.com/block/buzz/issues/9220).

## User outcome and scope

An operator configures an externally hosted agent once. Eligible community members
can find that same identity in Agents, choose it in New message, and invite or
mention it in permitted conversations from desktop, web and mobile. The agent
continues working when every desktop is closed. Its reasoning, tools and workflow
state remain in the external runtime; n8n is one reference backend.

This draft changes documentation only. Implementation needs agreement on the
discovery authority and repository boundaries below. It does not claim that an
unchanged released client already supports this complete flow.

Non-goals: launching workflows inside the relay, requiring a desktop plugin or
local configuration on each user's machine, controlling arbitrary infrastructure
from clients, replacing existing Buzz identities/policies, or adding an n8n-specific
agent type to the protocol.

## Existing contracts to preserve

- [Remote agents](remote-agents.md) separates the relay-connected
  harness from its launcher. A server-side launcher is already a supported shape.
- [NIP-OA](nips/NIP-OA.md) attests an independent agent key; it does not transfer
  authorship or prove channel membership. [NIP-AA](nips/NIP-AA.md) describes
  owner-derived relay access, separately from conversation admission.
- [Availability](agent-availability.md) is relay presence, not a deployment receipt.
  Discovery must never imply local process custody or an available runtime.
- The [ACP harness](../crates/buzz-acp/README.md) already owns Buzz subscription,
  author gates and conversation/session routing. Preserve those owners instead
  of implementing a second Nostr client in an n8n Code node.
- Current desktop/web work belongs in [block/buzz-app](https://github.com/block/buzz-app);
  mobile and harness work belong here. Its [shared agent selection](https://github.com/block/buzz-app/blob/main/docs/agents.md#shared-agent-selection)
  is the consumer to extend, rather than reconstructing eligibility in each picker.

These are architectural starting points, not proof of end-to-end release support.
Source audit: block/buzz `38795ada8749509607caddbafd4049162f8a31f9` and
block/buzz-app `8dca3d8be32c5fb2037daaa6638760648adc6acf` (2026-11-06).

## Proposed execution boundary

```text
Desktop / Web / Mobile
          |
       Buzz relay
          |
  existing buzz-acp on an operator's server
          |
      ACP over stdio
          |
  reference HTTP adapter
          |
  authenticated HTTPS webhook
          |
  n8n agent / another external runtime
```

Prefer a small ACP agent adapter outside the relay's request handlers. Whether it
belongs in this workspace or a separate reference repository is a maintainer
decision. It must negotiate only the ACP capabilities it implements. A runtime
that speaks ACP directly does not need the HTTP adapter.

ACP is the existing standard at the harness boundary. The HTTP application payload
below would be a Buzz adapter contract, not an existing ACP-over-HTTP standard.
Existing workflow `call_webhook` remains useful for automation; invoking it alone
does not establish the directory, ownership, DM or session guarantees in this RFC.

### Minimal HTTP contract to agree

| Data | Purpose |
| --- | --- |
| Contract version | Refuse unsupported request/response shapes |
| Invocation ID | One stable logical turn across transport retries |
| Session ID | Opaque, stable external-runtime conversation key |
| Community, agent and destination references | Correlate a turn to its admitted origin |
| Trigger event IDs | Deduplicate and correlate accepted Buzz input |
| Input blocks and bounded context | Preserve content without sharing unrelated history |
| Deadline and declared response mode | Bound synchronous work or an asynchronous completion |

The harness/adapter supplies the routing context from trusted state. Text in a
message cannot select an agent, destination, endpoint or credential. If the
current ACP seam lacks stable trigger metadata, propose a narrow structured
extension there; do not parse ownership or routing out of rendered prompt text.

The initial contract can support one bounded synchronous result. Long-running
backends also need a reviewed asynchronous mode: accept a job, return the same
invocation ID, and deliver a correlated terminal result. A transport acknowledgement
is not a successful agent turn. Callback route names and authentication are open
design decisions, not proposed additions to the relay HTTP API.

A completion must match the stored community, agent, session, invocation and
destination. The backend cannot choose a different channel in its response.
The reference adapter needs one explicit server-side publication path, reusing
the existing Buzz message builder and publisher under the agent's identity.
ACP text updates alone are not a chat publish acknowledgement; do not auto-post
all ACP chunks or private reasoning. Only the final backend result enters this
scoped send path, with destination fixed by trusted turn metadata.

Persist the exact signed reply/event ID before its first submission. On an
ambiguous acknowledgement, resolve or retry that same event through the existing
Buzz event path, never regenerate a new reply on each retry. Correlate the relay
acceptance and retain recoverable publication state. Recheck current admission
after long-running work and refuse a revoked destination; a backend HTTP 200
is not a relay publish acknowledgement.

The adapter should not request an unrelated MCP server, shell, repository checkout
or model credential merely to forward a turn. Additional tool support requires
an explicit capability and authorization design.

## Community registration and client discovery

Execution and discovery are separate deliverables. An ACP adapter alone is not
completion of this feature.

A one-time registration needs an owner-authorized identity and policy, display
metadata, and an operator-private execution mapping. Reuse the existing signed
profile/policy and authoritative membership sources wherever sufficient.
Backend URLs, tokens and private keys never belong in public agent profiles.

Agree how an eligible member discovers an externally operated identity across
clients, including agents owned by another community member. An owner-only
provider inventory is insufficient for this user story. Do not silently reinterpret
a template catalog as a directory of deployed agents, trust an unsigned advertisement,
or create a secretless local managed-agent record to make a card appear.

Every client projects the same exact public key and admission evidence into its
agent list, New message, profile Message action and channel invitation/mention
flow. Discovery does not grant DM access, membership, invocation authority or
runtime management. Creating a DM and adding a member use the existing permission
checks; failed/partial discovery remains distinguishable from an authoritative
empty list.

Ordinary members get conversation actions allowed by policy. Lifecycle controls
remain with their authorized owner/operator and do not imply local Start/Stop.
Presence and work completion remain separate. A dead or unreachable backend must
not be advertised as healthy solely because its adapter still has a relay socket.

## Identity, privacy and failure handling

- Provision a distinct agent key and existing owner attestation. Keep the owner's
  key out of workflow JSON and the backend. Agent signing material stays in the
  trusted server-side harness/publication path, never in HTTP payloads.
- Configure exact endpoints and per-agent credentials through operator-owned
  secret storage. Disabling one agent must not require rotating every backend.
  HTTPS/authentication, replay protection and callback binding are required.
- Reuse applicable outbound HTTP/SSRF protections: no message-selected URL or
  automatic redirect. An explicitly allowed private backend is an operator policy,
  not permission to reach arbitrary private networks.
- Scope all state to the host-selected community and exact conversation. Send
  only authorized context; never the owner's other DMs, credentials or hidden
  reasoning. Buzz history/engrams and the runtime's workflow or private memory
  remain separate responsibilities.
- Persist accepted asynchronous jobs and retry state before acknowledging them.
  Retries reuse the invocation ID and require backend idempotency; unknown delivery
  cannot be retried as a new turn. Bound queues, body sizes, deadlines and retention.
- Refuse duplicate terminal callbacks or late results after cancellation, expiry,
  revocation or a superseding invocation. Cancellation must report whether remote
  execution actually stopped; loss of the HTTP connection is not proof of that.
- Preserve per-session ordering and isolate concurrent conversations. Recovery
  after adapter restart must not mix users, fabricate completion or replay a turn
  whose acceptance is unknown. Do not promise exactly-once remote side effects.

## Acceptance before calling the integration complete

1. Register once without copying the owner's key to n8n or installing a client plugin.
2. A second authorized human, not the agent owner, finds the same agent in Agents
   and New message on current desktop, web and mobile; an unauthorized user cannot.
3. That human opens a DM and mentions/invites the agent in an allowed channel.
   Replies have the agent's signature and correct thread/conversation references.
4. Close all desktop apps. The external runtime still replies from another client.
5. Reject an unsigned/forged registration, cross-community input, unauthorized
   callback, changed destination and messages outside the agent's policy.
6. Exercise backend timeout, duplicate input/completion, adapter restart and
   overlapping conversations. Preserve session mapping and visible failure states.
7. Remove membership or revoke invocation while a job runs. Its completion cannot
   bypass current authorization. Expired, cancelled and failed jobs stay truthful.
8. Run the same contract against a small deterministic HTTP fixture and a minimal
   n8n reference workflow with exported credentials omitted.

## Staged contribution and decisions requested

- **Contract:** agree the authenticated discovery source, registration authority,
  existing event/policy reuse and supported client versions. Allocate no new Nostr
  event kind in this draft.
- **Execution:** implement the smallest reference ACP-to-HTTP adapter and
  conformance fixture, first with synchronous turns; specify asynchronous delivery
  before adding durable job machinery.
- **Clients:** implement missing shared selection/admission projections in
  block/buzz-app and mobile. Keep the deprecated `desktop/` app out of feature work.
- **Reference deployment:** document one server-side launcher, secret references,
  recovery and an n8n example after the protocol and ownership boundaries agree.

Each implementation PR needs its own real-boundary validation. The asynchronous
mode and complete cross-platform experience remain pending until their own
acceptance cases pass; a working echo webhook is not evidence for them.
Older clients may continue ordinary messaging without gaining the new directory
surface. A server upgrade cannot inject missing UI into an unchanged client.

Related work: [external roster #3996](https://github.com/block/buzz/issues/3996),
[remote identity registration #8012](https://github.com/block/buzz/issues/8012),
[hosted execution discussion #4174](https://github.com/block/buzz/issues/4174), and
[hosted DM report block/buzz-app#1090](https://github.com/block/buzz-app/issues/1090).
Historical reports need reproduction on the target release before treating them
as current defects. [PR #3803](https://github.com/block/buzz/pull/3803) targets the
deprecated desktop and is not the implementation vehicle for the new app.
