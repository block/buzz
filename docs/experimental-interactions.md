# Experimental interaction events (v1)

Signed buttons, polls and forms let a human or agent ask for a typed decision in
a Buzz channel. This contribution implements the first protocol/relay/CLI slice
and opt-in desktop cards. It is disabled by default and is not a stable NIP.

## Enable and try it

Set both variables on **every relay pod**, using the same existing relay identity:

```dotenv
BUZZ_EXPERIMENTAL_INTERACTIONS=true
BUZZ_RELAY_PRIVATE_KEY=<your existing persistent relay private key>
```

Startup rejects enabling the experiment without a persistent key. Do not rotate
that identity while prompts are open: state addresses include its public key.
The normal migrations create the storage tables even while the flag is off.
When off, new interaction writes are rejected, fallback matching and the worker
are disabled, and the capability is not advertised. Existing history is retained.
Re-enabling processes overdue prompts and pending delivery records.

In desktop Settings → Experimental Features, enable **Interaction cards**. The
relay flag controls acceptance; the desktop toggle controls rendering only.
Clients can discover `buzz-interactions-v1` in `/info`'s
`supported_extensions` and the relay's public key in `self`.

```sh
buzz interactions ask --channel "$CHANNEL_ID" --type buttons \
  --text 'Render E001?' \
  --option approve:Approve:primary --option revise:'Send back' \
  --option deny:Deny:danger --field note:Note:text:optional \
  --responders role:owner --closes first --expires 24h

# Use the returned event_id, which identifies the original signed prompt.
buzz interactions answer --prompt "$PROMPT_ID" --choice approve
buzz interactions get --prompt "$PROMPT_ID"
buzz interactions wait --prompt "$PROMPT_ID" --timeout 24h

buzz interactions poll --channel "$CHANNEL_ID" --text 'Which thumbnail?' \
  --option 1:A --option 2:B --option 3:C --max 1 --expires 2h

buzz interactions ask --channel "$CHANNEL_ID" --type form \
  --text 'Episode details' --field title:Title:text:required \
  --field length:Length:select:required --select length=60s,6min,9min
buzz interactions answer --prompt "$PROMPT_ID" \
  --value 'title=The Door' --value length=6min
buzz interactions close --prompt "$PROMPT_ID"
```

`ask --json @prompt.json` (also inline JSON or `--json -`) accepts an unsigned
`{"content":"Question", "tags":[...]}` envelope, capped at 64 KiB. The CLI sets
the prompt kind and signs locally. This avoids delimiter escaping in complex
labels. `--reply-to` uses the normal channel thread parent and root markers.
For listed responders, use `--responders listed --responder <pubkey>` repeatedly.

Writes use the existing `{event_id, accepted, message}` JSON contract. `get`
returns `[signed_prompt, signed_state]`, or just the prompt if no state was
returned. `wait` returns `[signed_final_state]`; its content is a JSON string.
It uses bounded HTTP polling and returns exit code 4 on timeout, never an
inferred denial or approval. Other exit codes follow the existing CLI contract.
The original response event IDs in state can be queried through the ordinary
Nostr bridge to inspect comments, form values and signatures.

## Source review of the September 8 outline

The signed prompt/response primitive fits Buzz's Nostr-first API and agent-first
vision. Several assumptions in the outline differ from this source tree:

| Outline assumption | Source finding and implementation decision |
| --- | --- |
| Three 4xxxx kinds, with addressable state | NIP-33 addressable kinds are 30000–39999. The central registry allocates **40010 prompt**, **40011 response**, **40012 close**, **39010 state**. The state kind is relay-only. |
| Unknown clients automatically display a new prompt kind | Timeline clients subscribe to explicit message kinds. Each prompt therefore atomically creates a relay-signed kind-9 text projection. Aware desktop clients upgrade that projection to a card. |
| A DM has no `h` tag | Buzz DMs are channel-scoped. All four kinds require one canonical channel UUID in `h`, including a DM channel. This experiment provides channel privacy, not end-to-end encryption. |
| A root-only tag anchors a thread | Buzz's NIP-10 resolver needs a reply parent. CLI creation preserves the parent and derives its root. Only the text projection increments timeline thread counters. |
| A relay-signed fallback counts as the responder | That would collapse every text answer onto the relay key. Acceptance instead keys votes by the **original signed message author**, retaining that event as evidence. A synthetic response has `via` and `actor` provenance. Clients cannot supply those tags on typed interactions. |
| Hidden responder lists make private polls private | Raw responses, queries and notifications would still reveal the answers. V1 accepts only `visibility=public`, meaning visible to readers of the channel. Listed responders restrict answering, not reading. |
| NIP-40 expiration only closes the prompt | NIP-40 tells clients to ignore expired events and relays not to deliver them, which would hide the original question. V1 uses a separate `deadline` Unix timestamp and rejects `expiration` on prompts. Decision deadlines and event retention are separate policies. |
| A secret field can simply use gift-wrap | The HTTP ingest bridge rejects gift-wrap, and there is no field-level encrypted envelope with a validation contract. V1 rejects `secret`, `asker-only` and `tallies-only` rather than storing them as plaintext. |
| `request_approval` already durably suspends a workflow | `buzz-workflow` currently finalizes this action as `approval_not_supported` (WF-08). Durable suspension, resumption and transactional consumption of a decision need a separate change; existing approval kinds and workflow YAML are untouched. |
| ACP permission requests are interchangeable with user questions | `buzz-acp` currently handles permission options in `handle_permission_request`, including its allow-once behavior. Mapping them needs explicit timeout, cancellation and permission policy; it is not a generic AskUserQuestion transport. No permission behavior changes here. |

Protocol references: [NIP-01 event kinds](https://github.com/nostr-protocol/nips/blob/master/01.md) and
[NIP-40 expiration](https://github.com/nostr-protocol/nips/blob/master/40.md).

Relevant source: [kind registry](../crates/buzz-core/src/kind.rs),
[shared ingest](../crates/buzz-relay/src/handlers/ingest.rs),
[thread markers](../crates/buzz-core/src/nip10.rs),
[workflow engine](../crates/buzz-workflow/src/lib.rs),
[ACP harness](../crates/buzz-acp/src/acp.rs). The related community discussion is
[block/buzz#3261](https://github.com/block/buzz/issues/3261); that discussion does
not constitute acceptance of these experimental kind allocations.

## Wire contract

Prompt content is Markdown. Schema tags follow the proposed `itype`, `opt`,
`field`, `optsel`, `responders`, `p`, `min`, `max`, `closes`, `deadline`,
`visibility` and optional `ref` shape. Option and field IDs are stable ASCII
identifiers. Unknown provenance tags such as `ref` are preserved by signatures
but do not grant authority or execute a workflow.

* Buttons have 1–8 options and exactly one selected choice. They can carry
  accompanying form fields, such as the outline's optional note.
* Polls have 2–12 options, bounded min/max selection counts and no form fields.
  They close manually or on expiry.
* Forms have 1–12 fields: text, finite number, select, boolean (`true`/`false`),
  or ISO date (`YYYY-MM-DD`). Select values come from `optsel` tags. Required
  fields cannot be omitted; unknown or repeated fields and choices are rejected.

Each prompt must expire within 30 days. A first or quorum close can happen
earlier; the asker may explicitly close any open prompt using kind 40012 with
`["e", "<prompt-id>", "", "prompt"]` and `h`. Expiry is a response deadline;
it does not delete the audit trail. `quorum:N` counts distinct eligible keys,
not votes for a particular option. Eligibility is rechecked on each new answer:
current channel membership plus members/listed/community-owner/admin rules.
An owner's earlier accepted answer remains recorded if they later leave.

Typed responses use that same `e` marker and channel, repeated `choice` or
`value` tags, and optional comment content. They are signed by the answering
key. One effective answer per key may change while open. A newer `created_at`
wins; equal timestamps use the lower event ID, matching Nostr ordering.
All accepted historical events remain stored. Exact retries are idempotent,
including a retry after close. A different answer after close is rejected.

State is addressed by `(39010, relay_pubkey, d=prompt_id)` and also has `h`.
Its JSON content contains `version`, monotonic `revision`, `status`,
`close_reason`, `tally`, `winner`, `values` and `responders`. A unique leading
button choice becomes `winner` only on close; ties and zero votes yield null.
`values` is a convenience object only when exactly one responder exists;
otherwise it is null. Each responder entry identifies the signed source event,
its author, timestamp and choices. A quorum form's per-person values remain in
those source events.

State creation timestamps advance by at least one second per transition to
avoid same-second addressable replacement races. During a burst they may lead
wall time. Readers render by **revision**; they do not count raw response events
or treat the state timestamp as the time of a person's decision. A prompt caps
at 256 distinct responders and 4096 answer transitions to bound snapshot growth
and clock advancement. Content is capped at 16 KiB and schema tags at 32 KiB;
answer values total at most 8 KiB. Prompt creation is limited to ten per minute
per key and 32 unexpired open prompts per channel.

## Compatibility, persistence and failure behavior

The kind-9 projection includes the question, option IDs and labels, fallback
instructions and the prompt ID. A direct reply to either it or the original
prompt is eligible for fallback only if the **whole trimmed reply**, matched
case-insensitively, is one declared option label or ID. Punctuation is
significant. `approve but tighten the hook` stays a comment. Forms, required
fields and polls requiring multiple choices cannot be answered this way.
Unknown, unauthorized, ambiguous, stale or closed-prompt replies still follow
the ordinary message path and do not cast a vote.

The original message and relay synthesis are separate, verifiable signatures.
A later typed answer replaces that same author's text answer. The relay never
signs as a user. The projection is an explanation of the prompt; the original
signed prompt remains the authoritative question and schema.

The shared HTTP/WebSocket ingest path authenticates the event and checks the
experiment before the interaction transaction. A row lock serializes answers
and close operations; membership/role rows are read on the writer and locked
during acceptance. The source event, updated snapshot, signed state and delivery
records commit together. A durable outbox retries Redis publication after
failure or restart. Delivery is at least once; consumers deduplicate by event ID
and state revision. Historical reads work independently of delivery progress.
Queued events removed by replacement or moderation are discarded rather than
replayed. Community and channel boundaries are carried through delivery and the
existing recipient access gate. The standard event audit path is reused.

Expiry uses a bounded worker with `FOR UPDATE SKIP LOCKED` across pods. Every
answer checks the database clock after acquiring the prompt lock, so a delayed
sweep cannot accept a late vote. Text answers retain ordinary message workflow
triggers with the existing post-commit semantics. Native interaction workflow
triggers are not introduced in this slice.

## Follow-up slices

This contribution intentionally does **not** claim all five rollout phases are
complete. Remaining work includes durable `request_input` workflow suspension
and outputs, migration of approvals, desktop `/ask` and `/poll` authoring and a
workflow editor, ACP/Hermes transport adapters, interaction-specific push
navigation, native mobile cards, and any encrypted/private ballot protocol.
Agents can already use the CLI's ask/answer/wait path without an adapter.
An automatic agent action must still enforce its existing author allow list.

Before removing the experiment gate, settle kind allocation with maintainers,
define prompt/projection moderation as one user-visible action, add relay-key
rotation migration, and exercise multi-pod races against native PostgreSQL and
Redis. The durable interaction snapshot is separate from ordinary event
retention; a retention policy and any privacy extension must explicitly cover
both tables and the outbox. Community deletion includes both tables.

## Validation

Core tests exercise schema errors, form types, replacement, deduplication,
first/quorum/expiry and deterministic equal-timestamp ordering. PostgreSQL tests
bind `Db::accept_interaction` and cover concurrent first-close, authorization,
tenant/channel isolation, fallback identity, rollback on failed state writes
and expiry before the worker runs. Run them with the repository's real database:

```sh
BUZZ_TEST_DATABASE_URL=postgres://... \
  cargo test -p buzz-db --lib store::interaction::postgres_tests -- --ignored
```

Desktop tests exercise keyboard button submission, native form validation,
poll selection, foreign/stale state and the default-off text fallback:

```sh
cd desktop
pnpm build:e2e
pnpm exec playwright test --project=smoke interactions.spec.ts
```

Run `just ci` and the repository's relay integration suite (`just test`) before
merging. Embedded PostgreSQL substitutes do not validate row-lock concurrency.
