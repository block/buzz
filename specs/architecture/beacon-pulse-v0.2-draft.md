# Beacon Pulse v0.2 (draft)

Status: v0.1 (the pulse itself) is implemented in both portable relay
adapters — the Cloudflare rendezvous (`cloudflare/portable-relay`) and the
laptop node (`crates/buzz-local-relay`) — and deployed on the sovereign
pair. v0.2 adds the response layer (kind 20701 semantics, the recognition
tally, and the sessions observation) and is **specified but not yet
implemented**. Kind numbers are in the shared registry
(`buzz-core/src/kind.rs`), PROVISIONAL pending upstream assignment.
Companion to `sovereign-sync-agreement-v0.1-draft.md`: kind 30700 is the
durable agreement, kind 20700 the ephemeral witness of now.

## Purpose

Synchronization between sovereign nodes is not replication alone — it is an
agreement protocol. The **Beacon pulse** is a node's signed declaration of
"this is the state I currently witness": journal head, replication
checkpoints, and the agreement heads it applies. It is simultaneously a
discovery signal, a synchronization cursor, a signed witness statement, a
health report, and an invitation to reconcile.

A central service says "here is the canonical state." The Beacon says "here
is the state this node currently witnesses." Canonicality is never asserted
by any single pulse; it emerges when enough trusted participants recognize
compatible heads under compatible agreements.

## Decision

The pulse is an **ephemeral Nostr event** (kind 20700, inside the NIP-01
ephemeral range 20000–29999). Three properties follow for free from the
range and the existing portable profiles:

- **never journaled** — an ephemeral event is published live and dropped; a
  pulse is a signal about state, not state itself;
- **never replicated** — durable replication already rejects ephemeral
  kinds, so pulses cannot be replayed into any journal;
- **carriable by the rendezvous** — the relay sees that a state transition
  occurred without holding the substance of the transition; encrypted
  artifacts and engram heads stay content-addressed elsewhere.

## Identity: the witness key

A pulse is signed by the **node's own witness key**, a distinct identity
from the owner: **the node witnesses, the owner governs.** When no witness
key is configured the capability is absent — the node stays silent and
`/health` reports `witness: null`. The witness pubkey is published in
`/health` for out-of-band binding (e.g. an owner-signed binding statement
or a future kind-30700 `witness/<label>` declaration head).

Per adapter:

- **Cloudflare rendezvous** — the `BUZZ_NODE_SECRET` Worker secret; pulses
  carry `role: rendezvous`.
- **Laptop node** — the relay's existing dedicated key (`--relay-key`,
  already used for relay-authored NIP-29 projections; auto-generated beside
  the journal); pulses carry `role: sovereign`. The laptop derives
  `previous` from the journal chain directly (every append is witnessed),
  and reports no checkpoints — it is a push source; the push cursor lives
  with the pusher.

Responses are signed by the **responder's own key**: a node responds with
its witness key; a participant (owner, agent, peer principal) responds with
the key it authenticated with. Nothing responds on another party's behalf.

## Pulse shape

```json
{
  "kind": 20700,
  "pubkey": "<witness key>",
  "created_at": 1753747200,
  "tags": [["n", "cf-rendezvous"], ["role", "rendezvous"]],
  "content": {
    "node": "buzz-portable-relay.example.workers.dev",
    "label": "cf-rendezvous",
    "adapter": "portable-relay-cloudflare-v0.1",
    "journal": { "sequence": 412, "head": "<event id>" },
    "previous": "<prior witnessed head>",
    "checkpoints": { "ted-laptop/sovereign": "cf-sqlite-v1:412" },
    "agreements": { "read/ted-laptop/sovereign": "<30700 head id>" },
    "coherence": {
      "governance": { "peers": "journal", "readers": "journal", "streams": "bootstrap" },
      "sessions": { "count": 2, "principals": ["<pubkey>", "<pubkey>"] },
      "recognition": {
        "head": "<event id>",
        "pulse": "<pulse event id the tally answers>",
        "responses": { "<responder pubkey>": "recognize" },
        "window_secs": 300
      }
    }
  }
}
```

(`content` is the JSON-serialized form of the object above; `sessions` and
`recognition` are v0.2 additions, optional and independently adoptable.)
Tags carry only routing hints — `n` for the node label, `role` for the
node's declared function (`rendezvous` | `sovereign`) — so tooling can
filter without parsing content. The fields map to the pulse vocabulary:

- **head** — the node's journal head: last appended event ID plus sequence.
- **previous** — the witnessed chain: the head recognized before the
  current one, so observers can distinguish advance from replacement.
- **agreements** — the effective kind-30700 declaration head IDs this node
  currently applies (owner-signed, n-tagged for this node), i.e. the
  policy/contract versions in force.
- **coherence** — the node's current observations (see
  *Coherence observations* below).

## Emission

Two modes, both synthesizing a fresh signed pulse from live state:

1. **On request** — a query or subscription whose filter *explicitly names*
   kind 20700 receives a current pulse (after stored matches, before EOSE).
   Open filters never surface it: witnessing happens on request, not by
   accident. Because synthesis happens at the query layer, the pulse is
   observable through the existing HTTP `POST /query` bridge with no new
   endpoint and no client changes — subscribing *is* asking.
2. **On transition** — every journal append (direct write or replication
   ingest) emits a pulse to live subscribers. Emission is the only place
   the witnessed chain (`previous` → `head`) advances; reads observe,
   never transition.

Not in v0.2, deliberately: periodic heartbeat pulses (a Durable Object
alarm can add liveness-without-transition later) and `POST /count`
synthesis.

## Standing

The pulse reveals journal metadata — head IDs, cursors, agreement heads —
so under required identity it is addressed to **the parties of the node's
agreements**: the owner and any declared peer or reader verification key.
An authenticated stranger receives no pulse and no error. On an open node
(no required identity) the pulse is open.

Responses inherit the same boundary in both directions: only a party with
standing to observe a pulse may respond to it, and responses fan out only
to parties with pulse standing (a response reveals the same class of
metadata a pulse does). Future: steward-role standing (observe + report)
once steward declarations are evaluated into config.

## Responses (kind 20701)

A pulse is an invitation to reconcile. A **response** is a party's signed
answer: what its own state says about the head the pulse witnessed.

### Shape

```json
{
  "kind": 20701,
  "pubkey": "<responder key>",
  "created_at": 1753747205,
  "tags": [
    ["e", "<pulse event id>"],
    ["p", "<pulse witness pubkey>"],
    ["n", "<responder node label, when a node responds>"],
    ["role", "sovereign | rendezvous | participant"]
  ],
  "content": {
    "stance": "recognize | advanced | conflict | diverged | unsatisfied",
    "head": "<the pulse's journal head being answered>",
    "mine": { "sequence": 140, "head": "<responder's own head>" },
    "observed": { }
  }
}
```

- The `e` tag anchors *which statement* prompted the response;
  `content.head` restates *which head* the stance is about. Both are
  required — a stance bound only to an event ID would turn ambiguous the
  moment the witness pulses again.
- `mine` is the responder's own witness half: every response is itself a
  miniature pulse. A response without `mine` claims a stance while
  witnessing nothing; evaluators treat it as noise.
- `observed` carries stance-specific evidence (see table).

### Stance vocabulary and decision procedure

A responder evaluates against its own journal, in order; the first matching
row is the stance. "Holds" means the event appears in the responder's
journal — journals are append-only logs, and replication makes an admitting
node's journal a superset of the streams it admits, so **containment is the
portable ancestry test**.

| Order | Condition (responder's view) | Stance | `observed` evidence |
|---|---|---|---|
| 1 | An agreement head the pulse pins is one I cannot apply — revoked, unknown, or pinning a head I reject | `unsatisfied` | `{ "agreement": "<d-tag>", "reason": "..." }` |
| 2 | I hold the pulse's head and it is my own head | `recognize` | — |
| 3 | I hold the pulse's head and my journal extends beyond it | `advanced` | `{ "since": <records past it> }` |
| 4 | I do not hold the pulse's head, **and** its claims about my own streams contradict my journal (a checkpoint beyond my length; a different event at a sequence I hold) | `conflict` | `{ "claim": "...", "mine": "..." }` |
| 5 | I do not hold the pulse's head (I may simply be behind), or our states differ by a measure other than containment (e.g. effective agreement heads disagree) | `diverged` | `{ "measure": "head-unknown \| agreements", "detail": "..." }` |

The two easily-confused stances: `diverged` is symmetric and curious —
"our states differ by this measure; reconciliation wanted." `conflict` is
an accusation with evidence — "your statement contradicts something I
hold." A node that is merely behind says `diverged` with
`measure: head-unknown`, never `conflict`.

### Freshness, silence, and non-durability

- **Freshness** — respond only to a pulse observed live or younger than the
  recognition window (default 300 s). Evaluators ignore responses whose
  `created_at` falls outside the window of the pulse they answer; a stance
  about a stale pulse is itself stale.
- **Silence is not a stance.** Responses are ephemeral and best-effort; a
  missing response means "not observed," never "not recognized." Nothing
  may read silence as dissent — or as assent.
- **Conversation, not record.** Responses are never journaled and never
  replicated (ephemeral range guarantees both). If a stance is worth
  keeping — a conflict worth adjudicating, a divergence worth tracking —
  the party journals a durable event about it (a kind-1 note, a steward
  finding, or a future durable kind). The pulse layer deliberately refuses
  to become a second journal.

### Relation to hand-offs

A `recognize` from an agent-bound key is the natural liveness signal for
delegation: "I am here, I hold your head, I can be handed work." The
handoff lifecycle stays durable (kind-1 records); responses only answer
*who is reachable and coherent right now*.

## Coherence observations

`coherence` is the extensible core of the pulse: the node's own current
observations, each independently adoptable. v0.2 defines three.

### `governance` (v0.1)

Which configuration domains are journal-governed versus env/file bootstrap.

### `sessions` (v0.2) — the socket-layer observation

The node already knows, at the transport layer, which authenticated
principals hold live connections. That knowledge becomes useful presence
only when it is **witnessed**: disclosed through a signed statement, under
the standing rule, with honest provenance — this is the *node's
observation*, not anyone's self-assertion.

```json
"sessions": { "count": 2, "principals": ["<pubkey>", "<pubkey>"] }
```

- Coarse by design: authenticated principal pubkeys and a total count,
  never transport metadata (no addresses, user agents, or per-principal
  connection multiplicity).
- Present only when the node requires identity — anonymous sockets have no
  principals worth witnessing, so an open node omits `sessions` entirely.
- Automatically gated by pulse standing: only the parties of the node's
  agreements see who else is at the table.

**Three grades of "online",** each with a different home — never conflate
them:

| Grade | Meaning | Asserted by | Mechanism |
|---|---|---|---|
| connected | a socket exists | the node | `coherence.sessions` in the pulse |
| present | "I am here" | the subject | upstream kind 20001 heartbeat (`KIND_PRESENCE_UPDATE`) |
| witnessing | "I am here **and** this is what I hold" | the subject | kind 20700 pulse / kind 20701 `recognize` |

### `recognition` (v0.2) — the roll-call tally

A node that emits pulses also observes the responses they draw, and may
fold the tally into its next pulse:

```json
"recognition": {
  "head": "<head the tally is about>",
  "pulse": "<the pulse event id that was answered>",
  "responses": { "<pubkey>": "recognize", "<pubkey>": "advanced" },
  "window_secs": 300
}
```

- Latest stance per responder within the window wins; the tally resets when
  the head advances.
- The tally is the node's **observation of ephemeral traffic — a report,
  not a proof**. Responses cannot be audited after the fact (they were
  ephemeral); the honesty model is that any party with standing could have
  observed the same responses live.
- **No quorum is defined at the pulse layer.** "Settled when N of M
  recognize" is agreement policy and belongs in a kind-30700 declaration
  field (future work), never in the transport of witness statements.

The pulse is the question, responses are the roll, and the next pulse's
`recognition` is the minutes.

## Relation to presence prior art

- **Upstream Buzz** already defines subject-asserted presence — kind 20001
  `KIND_PRESENCE_UPDATE` (ephemeral heartbeat, 90 s TTL) and kind 40902
  `KIND_PRESENCE_SNAPSHOT` (relay-signed sidecar) — and synthesizes
  relay-signed presence events **on request** in its query bridge, the same
  pattern this spec uses for the pulse. Its disclosure guardrail: presence
  queries must name `authors` — point queries, never open enumeration. The
  portable adapters adopt 20001 as-is if subject-asserted presence is
  wanted (ephemeral fan-out already carries it); any future synthesis must
  mirror the authors-required rule on top of pulse standing.
- **Nostr ecosystem** — NIP-38 kind 30315 (durable user status: mood, not
  liveness) and NIP-53 kind 10312 (room presence: subject-asserted,
  periodically republished, staleness by age). Both confirm the
  subject-asserted heartbeat shape; neither carries state substance, which
  is precisely what 20700/20701 add.

## Observation

`buzz-ctx pulse` with no argument witnesses every node of the pair — the
local sovereign node and the cloud rendezvous — side by side; shared
reality is visible exactly when their heads and agreements are compatible.
`buzz-ctx pulse <relay-url>` targets one node. Any NIP-01 client can
subscribe with `{"kinds": [20700]}`. Future: `buzz-ctx pulse --respond` to
hold a subscription open and answer observed pulses with this machine's
stance.

## Changelog

- **v0.2** — kind-20701 response semantics (shape, stance decision
  procedure, freshness window, silence and non-durability rules);
  `coherence.sessions` (witnessed socket-layer presence, standing-gated);
  `coherence.recognition` (roll-call tally, report-not-proof); presence
  prior-art positioning (upstream 20001/40902, NIP-38, NIP-53). Specified,
  not yet implemented.
- **v0.1** — the pulse itself: kind 20700 shape, witness keys per adapter,
  synthesis-on-request + push-on-transition, standing rule, ephemeral
  guarantees. Implemented and deployed on both nodes of the sovereign pair.
