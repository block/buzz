NIP-RQ
======

Owner Permission Requests
-------------------------

`draft` `optional` `relay`

**Depends on**: NIP-44 (Encrypted Payloads), NIP-40 (Expiration Timestamp), NIP-OA (Owner Attestation), NIP-42 (Authentication of Clients to Relays)

This NIP defines a small set of event kinds by which an agent asks its owner
to approve an action that the agent cannot or should not perform on its own,
such as creating another agent or using an escalated tool. The owner sees the
request where they already are, decides on their own device, and an installed
handler performs the approved action. The agent never holds owner key material.

## Motivation

Agents regularly reach actions that need their owner: creating an agent,
granting access to a tool, spending against a budget, publishing somewhere
new. Today each of these needs its own bespoke flow: a dedicated tool, a
dedicated review queue in one client, and a dedicated approval screen the
owner has to go and find. Each flow is expensive to build, and none of them
reach the owner when that one client isn't open.

A single request envelope lets any agent ask for any action, lets any client
show the request in context (in the conversation where it came up, or in a
global inbox), and leaves the action itself to a handler that already knows how
to do it. Adding a new kind of approval then means writing one handler, not a
new protocol surface.

## Definitions

- **Agent**: a process with its own Nostr keypair, acting for an owner, whose
  owner relationship is proven by a NIP-OA credential.
- **Owner**: the key that attested the agent.
- **Action**: a namespaced operation the agent asks to have performed, such as
  `buzz.agent.create`.
- **Provider**: the namespace that owns an action's handler, such as `buzz` for
  actions any Buzz client can perform, or a hosting provider's identifier for
  actions only its plugin can perform.
- **Handler**: client-side code, installed on the owner's device, that renders a
  request for one `(provider, action)` pair and performs it on approval, either
  locally or by calling a remote service.

## Kinds

| Kind | Name | Signed by | Encrypted to |
| --- | --- | --- | --- |
| `44210` | Permission request | agent | owner |
| `44211` | Permission decision | owner | agent |
| `44212` | Permission outcome | owner | agent |

All three are regular stored events. None of them carry an `h` tag: like
NIP-AM, they are owner-scoped, and the conversation a request came from lives
inside the encrypted payload so that the relay does not learn which channels
produce which requests.

## Permission request (`kind:44210`)

```json
{
  "kind": 44210,
  "pubkey": "<agent_pubkey>",
  "content": "<NIP-44 v2 ciphertext>",
  "tags": [
    ["p", "<owner_pubkey>"],
    ["agent", "<agent_pubkey>"],
    ["provider", "buzz"],
    ["action", "buzz.agent.create"],
    ["expiration", "<unix_seconds>"],
    ["auth", "<owner_pubkey>", "<conditions>", "<sig>"]
  ]
}
```

| Tag | Required | Meaning |
| --- | --- | --- |
| `p` | yes, exactly one | The owner. |
| `agent` | yes, exactly one | Equal to `pubkey`. |
| `provider` | yes | Handler namespace. Routing only. |
| `action` | yes | Namespaced action identifier. Routing only. |
| `expiration` | yes | NIP-40 expiry. Relays SHOULD reject requests that expire more than 7 days after `created_at`. |
| `auth` | yes | The agent's NIP-OA credential for the owner in `p`. |

`provider` and `action` are cleartext so that relays can apply policy and
clients can route a request to a handler before decrypting it. They MUST NOT
carry anything beyond the identifiers themselves.

### Decrypted payload

`content` is NIP-44 v2 encrypted with `(agent_privkey, owner_pubkey)` and
decrypts to:

```jsonc
{
  "v": 1,
  "title": "Create an agent named Scout",             // REQUIRED, one line
  "reason": "You asked me to hand triage to a new agent.", // REQUIRED
  "params": {                                          // REQUIRED, action-defined
    "name": "Scout",
    "instructions": "Triage new issues in this channel."
  },
  "display": [                                         // OPTIONAL
    { "label": "Model", "value": "Owner default" }
  ],
  "context": {                                         // OPTIONAL
    "channel": "<channel_uuid>",
    "thread": "<thread_root_event_id>",
    "message": "<event_id>"
  }
}
```

- `params` is the machine-readable input the handler executes. Its schema is
  defined by the action.
- `display` is agent-supplied extra text. Clients MUST label it as written by
  the agent and MUST NOT present it as a description of what approving does.
- `context` lets the owner's client show the request inline in the
  conversation it came from. Without it, the request appears only in a global
  inbox.

## Permission decision (`kind:44211`)

The owner's client publishes exactly one decision per request:

```json
{
  "kind": 44211,
  "pubkey": "<owner_pubkey>",
  "content": "<NIP-44 v2 ciphertext>",
  "tags": [
    ["e", "<request_event_id>"],
    ["p", "<agent_pubkey>"],
    ["decision", "approve"],
    ["x", "<sha256 of the canonical approved params>"]
  ]
}
```

- `decision` is `approve` or `deny`.
- `x` is required when `decision` is `approve`. It is the lowercase hex SHA-256
  of the RFC 8785 canonical JSON of the params the owner approved. It binds the
  approval to exact inputs.
- `content` is NIP-44 v2 encrypted with `(owner_privkey, agent_pubkey)` and
  decrypts to `{"v": 1, "params": {...}, "note": "..."}`. Both fields are
  optional. `params` is present when the owner edited the request (for example,
  rewrote instructions). When present, `x` MUST hash these params rather than
  the requested ones.

## Permission outcome (`kind:44212`)

After the handler finishes, the owner's client reports the result to the agent:

```json
{
  "kind": 44212,
  "pubkey": "<owner_pubkey>",
  "content": "<NIP-44 v2 ciphertext>",
  "tags": [
    ["e", "<request_event_id>", "", "request"],
    ["e", "<decision_event_id>", "", "decision"],
    ["p", "<agent_pubkey>"],
    ["status", "done"]
  ]
}
```

`status` is `done` or `failed`. `content` decrypts to
`{"v": 1, "result": {...}, "error": "..."}`, where `result` is action-defined.
For `buzz.agent.create`, it is `{"pubkey": "<new_agent_pubkey>"}`.

## Lifecycle

1. The agent publishes a request and tells the owner in conversation that it is
   waiting.
2. The owner's clients show the request inline (when `context` is present and
   the client has that conversation) and in a global inbox.
3. The owner approves, edits and approves, or denies. The decision is signed on
   the owner's device.
4. On approval, the handler performs the action. It runs locally when the action
   needs the owner's key (agent creation, NIP-OA attestation). It calls a remote
   service when the action is held elsewhere, passing the request and decision
   events as proof.
5. The owner's client publishes the outcome. The agent continues.

The agent MAY withdraw a pending request with a NIP-09 deletion. A request is
**open** until it has a decision, is deleted, or expires.

## Client behavior

- Clients MUST NOT offer Approve for a `(provider, action)` pair that has no
  installed handler. They SHOULD still show the request with Deny, and say that
  no installed handler can perform it.
- The handler, not the agent, decides what the approval screen says about the
  effect of approving. It MUST render that from `params`.
- Clients MUST verify the request's `auth` tag names the `p` owner before
  showing it, and MUST discard requests whose `agent` tag differs from
  `pubkey`.
- Clients MUST ignore expired requests and MUST NOT publish a decision for one.
- Clients SHOULD treat a second decision from another of the owner's devices as
  a no-op once one is observed.

## Handler and executor behavior

A handler or remote service performing an approved action MUST check, before
acting:

1. The decision's `pubkey` equals the request's `p` tag.
2. The request's `auth` credential is valid for that owner and agent.
3. The decision is `approve`, references the request by `e`, and its `x`
   matches the canonical params it is about to execute.
4. The request has not expired, been deleted, or already been executed. Each
   request is executed at most once.

## Relay behavior

Relays implementing this NIP:

- MUST serve all three kinds only to the request's owner and agent (p-gated,
  NIP-42 authenticated reads).
- MUST reject a `kind:44210` whose `auth` credential does not verify for `p`
  and `pubkey`.
- MUST reject a `kind:44211` whose `pubkey` is not the referenced request's
  `p`, or that references a request that is expired, deleted or already
  decided.
- MUST reject a `kind:44212` whose `pubkey` is not the referenced request's
  `p`.
- SHOULD cap open requests per agent (RECOMMENDED: 20) and reject new ones with
  `rate-limited:` above the cap.
- MAY add `44210` to the NIP-PL allow-list of approval-request kinds eligible
  for urgent push, since eligibility is decidable from its envelope.

## Security considerations

- **No owner keys leave the owner's device.** The agent asks, and only the
  owner's device signs. Actions that need the owner's signature (creating or
  attesting an agent) run in the owner's client.
- **What you see is what runs.** Approval binds to a hash of the exact params,
  so an executor cannot be handed different inputs than the owner approved.
  Agent-written text is labeled as such, and the handler describes the effect.
- **Prompt-injected or runaway agents** are bounded by the per-agent cap, the
  required expiry, and the fact that nothing happens without an explicit owner
  decision.
- **Replay** is prevented by one-decision-per-request at the relay and
  execute-at-most-once at the handler or service.
- **Metadata.** The relay learns that an agent asked its owner for some
  `(provider, action)` and when. It does not learn the params, the reason or
  the conversation.

## Relationship to other NIPs

- NIP-OA proves the agent's owner. This NIP never grants a new credential;
  actions that issue one do so through their own NIP.
- The workflow approval kinds (`46010`–`46012`, `46030`–`46031`) gate steps in
  relay-run workflows. This NIP covers agent-to-owner requests, which no
  workflow mediates.

## Example: agent creation

An agent asks for a new agent with `provider: buzz`, `action:
buzz.agent.create` and params `{name, instructions}`. The owner's desktop or
mobile client has a built-in handler. On approval it runs its normal agent
creation flow, including NIP-OA attestation on the owner's device, then
publishes an outcome whose `result.pubkey` is the new agent. The requesting
agent never sees the new agent's keys or any owner key.
