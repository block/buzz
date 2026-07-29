# Beacon Pulse v0.1 (draft)

Status: draft for discussion; implemented in the Cloudflare portable relay
adapter (`cloudflare/portable-relay`) and not yet upstreamed. Kind numbers are
provisional pending assignment in the shared registry
(`buzz-core/src/kind.rs`). Companion to
`sovereign-sync-agreement-v0.1-draft.md`: kind 30700 is the durable agreement,
kind 20700 the ephemeral witness of now.

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

A pulse is signed by the **node's own witness key** (Cloudflare adapter:
the `BUZZ_NODE_SECRET` Worker secret), a distinct identity from the owner:
**the node witnesses, the owner governs.** When no witness key is
configured the capability is absent — the node stays a silent custodian and
`/health` reports `witness: null`. The witness pubkey is published in
`/health` for out-of-band binding (e.g. an owner-signed binding statement
or a future kind-30700 `witness/<label>` declaration head).

## Event shape

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
      "governance": { "peers": "journal", "readers": "journal", "streams": "bootstrap" }
    }
  }
}
```

(`content` is the JSON-serialized form of the object above.) Tags carry only
routing hints — `n` for the node label, `role` for the node's declared
function — so tooling can filter without parsing content. The fields map to
the pulse vocabulary:

- **head** — the node's journal head: last appended event ID plus sequence.
- **previous** — the witnessed chain: the head recognized before the
  current one, so observers can distinguish advance from replacement.
- **agreements** — the effective kind-30700 declaration head IDs this node
  currently applies (owner-signed, n-tagged for this node), i.e. the
  policy/contract versions in force.
- **coherence** — the node's current observations. v0 reports which
  configuration domains are journal-governed versus env-bootstrap;
  the field is deliberately extensible (future: per-source caught-up
  measures, artifact integrity sweeps, steward findings).

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

Not in v0, deliberately: periodic heartbeat pulses (a Durable Object alarm
can add liveness-without-transition later) and `POST /count` synthesis.

## Standing

The pulse reveals journal metadata — head IDs, cursors, agreement heads —
so under required identity it is addressed to **the parties of the node's
agreements**: the owner and any declared peer or reader verification key.
An authenticated stranger receives no pulse and no error. On an open node
(no required identity) the pulse is open. Future: steward-role standing
(observe + report) once steward declarations are evaluated into config.

## Responses (vocabulary reserved, kind 20701)

A pulse is an invitation to reconcile. Peers may answer with an ephemeral
kind-20701 response `e`-tagging the pulse, with a `stance` in content:

- `recognize` — I recognize this head.
- `advanced` — I have advanced from this head (mine supersedes).
- `conflict` — I observe a conflicting head.
- `diverged` — my local state diverges by this measure.
- `unsatisfied` — I cannot satisfy this agreement.

The relay needs no special handling — ephemeral fan-out already carries
responses. Response semantics (quorum, recognition thresholds) are
deliberately out of scope for v0.1; they belong to the agreement layer.

## Observation

`buzz-ctx pulse [relay-url]` queries the rendezvous custodian (default:
the Cloudflare replica) for a pulse and renders the witness statement.
Any NIP-01 client can subscribe with `{"kinds": [20700]}`.
