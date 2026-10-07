# Synaxis HTML Review Canvas

Buzz Desktop opens one NIP-AR artifact type, `synaxis.html-review`, in a
dedicated Review Canvas. Every other HTML attachment stays an inert,
download-only file card.

## Artifact types ([NIP-AR](nips/NIP-AR.md))

| `type` | Author | Purpose |
| --- | --- | --- |
| `synaxis.html-review` | Synaxis adapter | One revision of a generated, self-contained HTML page. Content is `synaxis.html-review/v1`: `attribution`, Synaxis `artifact` (with `payload_digest`), `presentation` (`blob_sha256`, `byte_size`), and `feedback_dispositions` (`addressed` or `unresolved` per feedback revision). The page itself is a NIP-92 `imeta` attachment. The pubkey that signs the create is the only one that may update it; the relay refuses every move, delete, and restore (see [Review integrity](#review-integrity)). |
| `synaxis.artifact-feedback` | Human reviewer | One comment bound to the exact reviewed revision, payload digest, and declared block. Filter tags: `target`, `target_revision`, `synaxis_artifact`, `payload`. Never copies document bytes. |

The content hash, `imeta` hash, Synaxis payload digest, and downloaded bytes
must be one value before anything renders.

## Discovery and flow

1. **Canvas** is a permanent sidebar destination. `/canvas` lists the current
   `synaxis.html-review` heads available on the active relay, newest first. The
   native layer pages the writer-authoritative artifact query and verifies each
   event before the webview parses or displays it.
2. Selecting an inbox row routes to
   `/canvas/<channel>/<artifact>?revision=<event>&agent=<pubkey>&thread=<event>`.
   The URL pins the exact reviewed revision and signed attribution, so the
   screen survives a reload. **All reviews** returns to the Canvas inbox;
   **Open thread** returns to the review's conversation.
3. Synaxis also posts a kind-9 notification in the original thread with
   `artifact` = `[uuid, revision event ID]`, `artifact_type`, `agent`
   (executive agent pubkey), and `x` (payload digest). Only this exact tag set
   produces a timeline review card. **Open review** uses the same Canvas route
   and additionally carries the notification author and digest so the canvas
   verifies that announcement.
4. A reviewer selects a declared block (pointer click, Enter, or Space in the
   document, or the block list in trusted chrome) and writes a comment.
5. Buzz signs and publishes one feedback revision, then — only after the relay
   accepts it — one kind-9 reply in the thread with a `p` mention of the
   executive agent and `feedback` / `artifact` tags. A retry republishes the
   identical signed event; a failed wake retries without republishing. Neither
   retry depends on the review head (see Retry reconciliation below).
6. The next review revision (same artifact UUID, `prev` set) lists each
   feedback revision as `addressed` or `unresolved`; the canvas shows it.

Feedback is accepted only on the current revision. Approval stays in Synaxis.

## Block markers

```html
<section data-synaxis-review-id="checkout.primary-action"
         data-synaxis-review-title="Primary checkout action"
         data-synaxis-source-ref="src/features/checkout/CheckoutActions.tsx">
```

IDs are unique, titles nonblank, and `source-ref` is an optional normalized
workspace-relative path. Any violation rejects the whole document.

## Security boundary

The HTML is factory-generated but untrusted.

- **Native reads.** The Canvas inbox and individual revisions are read over
  the relay's HTTP artifact query (never the WebSocket timeline or unread
  filters). Rust verifies event ID, signature, the NIP-AR envelope, artifact
  type, and non-deletion state before an inbox row reaches the webview; an
  opened review is additionally bound to its exact channel and artifact
  identity. Document bytes are size-capped (4 MiB), hash-checked, and
  UTF-8-checked natively and again in the webview.
- **Sanitize.** Scripts, frames, plugins, `meta`/`base`/`link`, handlers, URLs,
  and remote CSS references are stripped; forms are unwrapped. CSS is scrubbed
  to a fixpoint (deleting a match can splice its neighbours into a new one, such
  as `@imp@import;ort` or `<@import;/style>`) and every literal `<` is then
  rewritten to the CSS escape `\3c `, in head stylesheets, retained body and SVG
  stylesheets, and inline `style` attributes, so no CSS text can contain a
  closing tag. The composed frame is **re-parsed and audited** before it is
  returned: the head must be exactly the trusted CSP, charset, and chrome
  stylesheet plus the artifact's stylesheets holding exactly their escaped text;
  the only script is the annotator, last in the body; nothing else may be an
  active element or attribute; and the declared blocks must be exactly the ones
  the reviewer is shown. Anything else refuses the document (the parse step runs
  the same audit on a probe frame, so the refusal arrives with the document).
- **Isolate.** The document runs in `<iframe sandbox="allow-scripts">` (opaque
  origin; no forms, popups, top navigation, or same-origin access) under a
  meta CSP that allows one nonce-bearing script — the annotator
  (`public/synaxis-review-annotator.js`) — and denies network, frames, forms,
  base, and plugins.
- **Capability channel.** The annotator receives one `MessagePort` from parent
  chrome. Parent chrome validates every message against its own block table;
  a second frame `load` removes the frame.
- **Chrome outside the document.** The comment form, block list, and feedback
  live in trusted parent UI with labelled controls, Escape/cancel, focus
  restoration, and live announcements.
- **Indicators artifact CSS cannot hide.** Block hover, focus, and selection
  are drawn by a first-party stylesheet emitted before any artifact style, in
  a nonce-named cascade layer of `!important` rules: a dark outline over a
  white halo, so one layer clears 3:1 on any background. Artifact stylesheets
  cannot outrank an earlier `!important` layer; only an `!important` in a
  block's own inline `style` could.
- **Focus is never dropped by chrome.** Unavailable actions (refresh, Send or
  Retry, Discard) use `aria-disabled` with guarded handlers instead of
  `disabled`; the Send/Retry button, the form's submit, and Ctrl/⌘+Enter in the
  comment field all share one availability condition, so a pending head check
  or any other blocker cannot be bypassed from the keyboard. Discard runs as
  its own `discarding` phase: Discard reads “Discarding…”, Send or Retry stays
  unavailable under the label it had (never “Sending…”), the comment stays
  read-only, and focus has already moved to the text field it re-opens; a
  draft that is already signed always reads “Retry”; and the workbench is
  keyed by revision, so comment text and the open form never carry from one
  revision to another (a signed comment is remembered per reviewed revision
  instead; see Recovery after leaving the review).

The notification and revision must share one signer; Buzz does not otherwise
know which pubkey is the configured Synaxis adapter.

## Review integrity

The review head belongs to its adapter, not to every channel writer.

**Relay admission (authoritative).** `synaxis.html-review` is signer-locked in
the same atomic head transaction that compares `prev`. A `create` is open to any
channel writer for an unused artifact UUID, and the pubkey that signed the
retained current head owns the artifact from then on. Every later revision must
be an `update` in the head's own channel, signed by that same pubkey, with the
exact `prev`. An `update` from another signer, and every `move`, `delete`, and
`restore` from any signer, is refused with a generic rejection. Another channel
writer who has learned the current revision therefore cannot replace, remove, or
relocate the head, strand the adapter, or make reviewers' feedback stale. The
signer is read from the retained head event while the head row is locked, so
there is no window between the check and the advance; a head whose event is no
longer retained refuses further revisions rather than guessing. The desktop's own
signer comparison remains as defense in depth, not the only control.

## Feedback integrity

Feedback is a human record, not a generic mutable artifact. These rules hold at
the relay and again in the client; neither is the only line of defence.

**Relay admission (authoritative).** `synaxis.artifact-feedback/v1` is
create-only and immutable: any `update`, `delete`, or `restore` is refused, so
another channel writer cannot suppress a comment by advancing or deleting its
artifact. A create is accepted only while its `target_revision` is, in the same
transaction, the current, undeleted `synaxis.html-review` head of the named
artifact in the feedback's own channel, and the claimed Synaxis artifact ID and
payload digest match that revision's content. The review head row is
share-locked until commit, so a concurrent review revision either commits first
(the feedback is refused as stale) or waits until the feedback is stored. A
stale, deleted, other-channel, or other-artifact target yields one generic
conflict so nothing unreadable is disclosed; re-sending an already accepted
feedback event stays a no-op. The `target`, `target_revision`,
`synaxis_artifact`, and `payload` tags must restate the signed content exactly.

**Client head check.** Sending a *new* comment is paused while the head check is
in flight or has failed or a newer revision exists, and the head is re-read
from the relay immediately before signing. A newer revision, a deleted review,
or a relay error stops the send before anything is signed. Once a comment is
signed, no head check gates it (see Retry reconciliation). Synaxis re-verifies
the signed events as defense in depth.

**Retry reconciliation.** A signed comment is frozen, and retrying it depends
on the relay, never on a client head precheck. Before an event whose fate is
unknown is sent again, the exact event ID is read from the relay over its
writer-authoritative artifact query (`reconcile_review_feedback_event`, which
checks ID, author, kind, timestamp, tags, content, and signature, and fails
closed on a changed relay or signer): if the relay holds it the comment is
accepted and only the wake remains. Otherwise Retry republishes the identical
event: a relay that already accepted it answers success without reapplying it
(even after the head advanced), and a relay that never did refuses it, as stale
or, after the freshness window, as expired, which makes the comment `rejected`
and finally discardable. One feedback revision exists either way. Once the
relay has accepted the feedback, Retry always resends only the frozen wake,
whether the head advanced, is being re-read, or cannot be read. The form's
Retry stays enabled and the comment's block stays reachable in those states;
only creating a new comment stays paused. Retry remains bound to the captured
relay URL and signer.

**Captured scope.** A submission requires the active community's relay URL and
the signer pubkey. The wake, the exact-event relay read, and the relay client's
publish guard compare both against live state immediately before every send,
read, and retry, so a community or account switch during signing or a retry
never routes a comment to another tenant; the signed draft is kept for a retry
in the original community.

**Publish states.** Whether the relay holds the signed feedback event is an
explicit state, never inferred from a promise:

| State | Meaning | Discard | Retry does |
| --- | --- | --- | --- |
| `never_attempted` | Signed and saved; no publish started. | Yes | Publishes. |
| `ambiguous` | A publish started and ended without the relay's answer (timeout, dropped socket, quit mid-send, canceled by a community switch, `error:` refusal). The relay may hold it. | **Never** | Reads the exact event from the relay first; publishes only if absent. |
| `rejected` | The relay explicitly refused the exact event (`invalid:`, `conflict:`, `restricted:`, `blocked:`, `auth-required:`, `rate-limited:`). | Yes, after a fresh relay read confirms it holds nothing | Reads, then republishes the same event. |
| `accepted` | The relay holds it (confirmed by acknowledgement, duplicate answer, or relay read). Only the wake can remain. | Never | Sends the wake. |

The attempt is written to the outbox as `ambiguous` *before* the send starts,
so a process that dies mid-send restarts as ambiguous, never as never-sent. A
comment can only be discarded or replaced while the relay provably holds
nothing: `Discard` is not rendered for `ambiguous` or `accepted` comments, a
different comment is refused for a block whose comment may be on the relay
(`taken`), and discarding a `rejected` comment first reads the exact event
from the relay: if the relay holds it after all the comment is kept as
`accepted`, and if the relay cannot be read the comment is kept. Opening a
review also asks the relay once about every `ambiguous` or `rejected` comment,
so a comment the relay turns out to hold is adopted without a click.

**Durable outbox.** Every signed comment is saved to a device-local outbox
(`localStorage`, key `buzz-review-outbox.v1:<signer pubkey>`) before anything
is sent, under the exact relay, signer, channel, artifact, reviewed revision,
and block. It holds only what the relay already stores or is about to: the
signed event, the comment text, the captured relay URL and signer *public*
key, the thread and agent, the frozen wake inputs, and the publish state. No
private key, token, or credential is written, and if the entry cannot be made
durable nothing is sent. A stored entry is revalidated on every read: it is
kept only if its signed event is exactly what its own context composes and was
signed by the signer it is filed under. Bounds: at most 16 unfinished comments
per signer (a seventeenth is refused before it is signed), each at most
64 KiB, so the worst case is about 2 MiB. An entry is removed only after the
feedback is accepted **and** the wake succeeded, or when the reviewer discards
a comment the relay provably holds nothing for; entries that hold nothing on
the relay (`never_attempted`, `rejected`) also expire after 30 days idle. An
entry the relay may hold is never expired, because it is the only record that
its wake is owed. Signing out clears the origin's storage with it.

**Recovery after leaving or restarting.** Because the outbox is on disk, a
signed comment survives leaving the canvas (back to **All reviews**, **Open
thread**, “Open the latest revision”, any route change) *and* quitting Buzz.
Reopening that same revision restores the locked form and Retry, lists it under
“Feedback waiting to finish”, and marks every comment whose wake is owed
“Agent not notified yet” instead of showing it as an ordinary comment. That
mark is keyed by the outbox's signed event ID, not by what the relay lists, so
it holds even while the publish's acknowledgement is still unconfirmed
(`ambiguous`) and after a restart. A different relay or signer never sees it.
From another revision of the same artifact (typically the current head) it is
not restored as a form; it is listed under “Feedback waiting to finish” with
its own revision number and block title, and an action that navigates to
exactly that reviewed revision (the route's `revision` search value), where its
Retry and Discard live. A restored comment finishes with the agent and thread
it was signed for.

**Binding on display.** Feedback is shown only if its Buzz artifact UUID, exact
revision event, Synaxis artifact ID, and payload digest match the revision it
names, and it names a block that revision's sanitized document declares with
the same ID, title, and source reference. Feedback that a later revision's
dispositions point back to is checked against that earlier revision and its
document, loaded by exact ID. Block labels always come from the review's own
block table; entries that do not bind are counted as withheld, never rendered
with event-supplied text.

**Listing bounds.** Feedback on a revision is read in relay pages of 200,
newest first, until a short page or 1000 comments. Beyond that the oldest
comments are not listed and the panel says so; nothing is dropped silently.
Identifiers that Buzz copies into relay-validated tags (the Synaxis artifact ID
and block IDs) are bounded at 128 UTF-8 bytes, as the relay bounds them, so a
multi-byte identifier that fits in 128 characters but not in 128 bytes is
refused before anything is signed.

**One wake per feedback.** The wake carries the first attempt's timestamp,
frozen with the draft in the outbox, so a prompt retry rebuilds the
byte-identical kind-9 event. The relay's freshness window (15 minutes) can
still refuse that timestamp, for example when the first wake never reached the
relay and the reviewer retries an hour later. The native command then rebuilds
the wake **once** with a fresh timestamp: the same content and the same
`feedback`, `artifact`, channel, thread, and agent tags, hence the same
feedback binding, under a new event ID. That is safe only because the relay
keeps a ledger keyed by `(author pubkey, feedback revision event ID)` (see
[NIP-AR](nips/NIP-AR.md), *Feedback wake idempotency*): it checks the wake
against the accepted feedback revision, records it, and stores and emits it in
one transaction; any later wake for the same feedback with the same bindings,
whatever its timestamp, is acknowledged `duplicate:` and stores nothing, and a
resend of the recorded wake itself is acknowledged before the freshness check.
The refresh happens only after a structured refusal (HTTP 400 carrying the
relay's stale-timestamp reason) and only when a read-back shows the relay does
not already hold the exact frozen event; any other failure, and the case where
no time has passed, is returned unchanged. A refreshed wake whose
acknowledgement is lost is therefore recovered by the next Retry without a
second wake. If submission fails or its acknowledgement is ambiguous, the relay
is read back and the wake counts as sent only if it holds that exact event (ID,
author, kind, timestamp, tags, content, valid signature) or acknowledged a
duplicate.
