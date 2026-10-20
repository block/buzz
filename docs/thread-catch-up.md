# Thread catch-up

**Catch me up** is an on-demand desktop recap of one channel thread. It lists
the thread's **Decisions**, **Still open** questions, and the work **Assigned to
you**. Every point links to the messages that support it. Generating a recap is
private and changes nothing: it does not post anything, mark messages read,
send notifications, or change tasks. A person can review a recap and explicitly
share it with the same thread. Thread members can then add attributed
corrections. Corrections never rewrite the conversation or the generated text.

This document covers local setup, what is shared and who can see it, the
shared-recap content contract, limits, and test steps.

## Local setup

Generation runs the bundled `buzz-agent` over ACP stdio. It has no tools, MCP
servers, skills, or hints, and it uses a cleared environment and an isolated
working directory. Its transport is restricted to local use
(`BUZZ_AGENT_LOCAL_ONLY=1`):

- The model endpoint must be `http://<numeric loopback address>[:port][/path]`,
  such as `http://127.0.0.1:11434/v1`. Host names (including `localhost`),
  credentials in the URL, query strings, redirects, and proxies are refused.
- The setup must name a model, and the person must confirm that this endpoint
  runs the model on this computer. A loopback proxy that forwards to a hosted
  provider, or a Buzz Mesh endpoint, does not count as local inference.
- No provider credential is sent. If local setup is missing or invalid, Catch me
  up stays unavailable and explains why. It never falls back to a hosted model.

These settings (`baseUrl`, `model`, `confirmedLocal`) are separate from global
agent defaults. Buzz does not install or download models.

The endpoint must support OpenAI-compatible JSON-object output and `max_tokens`.
Configure its context window to at least 32,768 tokens; Buzz cannot set that
server setting through the OpenAI-compatible API. For an already installed
Ollama model, a separate model alias can set it without changing other models:

```text
FROM qwen3:4b
PARAMETER num_ctx 32768
```

Save this as a `Modelfile`, run `ollama create buzz-thread-catch-up -f Modelfile`,
then choose `buzz-thread-catch-up` in recap setup. The app does not check the
server's actual context size. A smaller server window may omit context.

## Visibility

- A generated recap exists only in the requesting client until the person
  chooses **Share with this thread**.
- A shared recap is a NIP-AR artifact ([NIP-AR](nips/NIP-AR.md), kind `45010`)
  homed in the thread's channel. The relay applies the channel's normal read
  permission to the recap, its history, and its corrections. Sharing and
  correcting need permission to post a kind-9 message in that channel.
- Sharing stays in the source channel. These commands never move a recap or
  widen its audience.
- Artifact revisions do not add chat unreads, raise reply counts, or repeat
  notifications.
- History depends on the relay's retention and moderation policy. Attribution is
  shown for the revisions the relay still returns. The client does not promise
  that every earlier revision is retained.

## Shared recap contract: `buzz.thread_catch_up`

Each revision is a complete NIP-AR snapshot:

| Tag | Value |
| --- | --- |
| `ar` | `1` |
| `d` | Artifact UUID. On share it is derived from the signer and the canonical recap (UUIDv5), so a retry after a lost acknowledgement finds the existing share instead of creating a duplicate. |
| `h` | The thread's channel UUID. |
| `type` | `buzz.thread_catch_up` |
| `title` | `Thread catch-up` on creation; later revisions keep the current title. |
| `op` | `create` when shared; `update` for each correction. |
| `root` | The thread root event ID. |
| `prev` | The current revision being corrected (absent on create). |

Content is JSON:

```json
{
  "version": 1,
  "recap": {
    "version": 1,
    "channelId": "<uuid>",
    "rootEventId": "<64 hex>",
    "since": 1700000000,
    "until": 1700003600,
    "points": [
      {"id": "p1", "category": "decision", "text": "…", "sourceIds": ["<64 hex>"]}
    ],
    "sourceIds": ["<64 hex>"],
    "coverage": {"messageCount": 2, "inputMessageCount": 2, "truncated": false, "warnings": []},
    "sourceRevisions": {"<64 hex>": "<SHA-256 of projected source>"}
  },
  "correction": {"pointId": "p1", "text": "…"}
}
```

Rules:

- **The recap does not change.** Every revision carries the same `recap`, copied
  from the shared original. If any retrieved revision changes it, this client
  treats the artifact as unsupported. It is left out of the list, counted in
  `unreadableCount`, and cannot be corrected.
- **Source revisions are frozen.** Newly generated recaps carry fingerprints
  of their cited messages after author edits and deletions are projected. Share
  rechecks those fingerprints, including edits made within the same second,
  and asks the person to regenerate if a cited source changed. Older payloads
  without fingerprints remain readable; sharing them requires regeneration.
- **An update adds at most one correction.** A changed `correction` in an
  `update` introduces a correction. A `create` must not carry one. An annotation
  update, restore, or move may carry the last correction forward; it does not
  become a new correction or acquire a new author. The list is rebuilt in
  `prev`-chain order. If the predecessor of the earliest retrieved revision is
  unavailable, that snapshot's carried correction is not attributed as new.
- **Attribution comes from signatures.** Each correction's `authorPubkey` and
  `createdAt` are the signer and `created_at` of its verified revision. Author
  fields inside the content are ignored.
- **Unfamiliar data is kept.** A correction copies the current revision's
  content and changes only `correction`. Unknown content fields and annotation
  tags are kept as they are. Signer-bound NIP-OA `auth` and notification-routing
  `p` tags are not copied. If
  the copied snapshot is not a valid envelope, the edit is declined.
- **Bounds.** Recaps have at most 12 points; point IDs of 1–64 bytes, unique;
  each point's text up to 2,000 bytes and 8 KiB total; categories `decision` or
  `question`; at least one cited source per point, drawn from the recap's
  `sourceIds` (at most 400 unique lowercase event IDs); and at most 20 coverage
  warnings. Corrections must be nonblank text up to 2,000 bytes, with no control
  characters except newline and tab. Model and correction text are plain text.

Assigned to you comes from existing issue and assignment records. It is not
part of the shared payload.

## Native commands

All commands take one camelCase `input` argument containing the captured
`scope` (`relayUrl`, `signerPubkey`, `channelId`, `rootEventId`). Each command
reads the active relay and signing keys once and uses that same pair for every
request it sends. If either value is blank or differs from the captured scope,
the command fails. It checks again just before any write: if the active
community or identity changed during the operation, nothing is published.

- `list_thread_catch_up_artifacts({scope})` returns
  `{recaps, unreadableCount, complete}`:
  - `recaps`: the readable shared recaps for this thread, most recently changed
    first. Each has its `eventId` (the current revision, used as the next
    correction's `prev`), its corrections, and `historyComplete`.
  - `unreadableCount`: how many recaps in the listed window were left out
    because they are malformed, fail signature or envelope checks, or have a
    rewritten recap. The UI must not say a thread has no shared recaps when
    this is greater than zero.
  - `complete`: false when the thread has more than 50 current recaps. The
    command asks the relay for 51 and returns at most 50 of the most recently
    changed recaps.
- `share_thread_catch_up({scope, recap})` checks the recap's structure, then
  revalidates the thread root and every cited source against the relay before
  creating the artifact. Uncited input is retained as coverage metadata;
  changing or deleting it does not block sharing. New shares need saved
  fingerprints for every citation.
- `correct_thread_catch_up({scope, artifactId, prevEventId, pointId, text})`
  re-reads the recap. It fails with an error starting with `conflict:` when
  `prevEventId` is no longer current, or when the relay rejects the write
  because another revision landed first. The client keeps the draft and
  reloads; the command never retries with a replaced `prev`.

Reads use only explicit artifact queries with `kinds:[45010]` and the `#h`,
`#type`, `#root` (and, when named, `#d`) predicates. The client also checks each
returned revision's signature and envelope rather than trusting the relay's
filters.

### History completeness

History is read with `artifact:"history"` in pages of 500 revisions, up to four
pages (2,000 revisions), for all listed recaps together. Reading stops early once
every recap's `prev` chain reaches its creating revision. `historyComplete` is
true only when that chain reaches `create` with no gap. If a revision is
redacted, expired, or past the page cap, `historyComplete` is false and only the
corrections after the gap are listed. In that case the person who shared the
recap is unknown: `authorPubkey`/`createdAt` are only the signer and time of the
earliest revision retrieved. Clients must show "shared by" attribution only when
`historyComplete` is true. Each listed correction's own author and time always
come from its signed revision.

## Limits

- Generation includes at most 200 input messages and 32 KiB of message text.
  It reads up to 400 new-message candidates within a bounded thread scan.
  Context is the root plus up to 20 of the newest replies just before the
  selected period, within a 24-hour lookback. A single context query examines
  at most 40 matching events; other uses of a reply reference may leave fewer
  than 20 actual context messages. Context is background, rather than a full
  history of the thread. Coverage warnings describe clipped message input.
- Generated points also obey the 2,000-byte per-point and 8 KiB total text
  limits. Oversized points are omitted with a warning.
- Generation has a 90-second budget including relay reads and model inference.
- The list returns at most 50 shared recaps per thread, ordered by most recent
  change. `complete: false` signals that older recaps were not returned. These
  commands do not page further.
- Shared recaps that this client cannot verify or interpret are left out of
  `recaps` but counted in `unreadableCount`. Their content is not shown.
- If the same person shares an identical recap again after the original was
  deleted, the share fails: NIP-AR never reuses a deleted `d`. Generating a new
  recap gives it a new identity.
- Older desktop clients and mobile have no reader for this content type.

## Testing

Focused native tests (no external services; they use a loopback relay double
that parses filters with the relay's own `buzz_core::artifact::parse_query` and
enforces `prev` like the relay):

```bash
. ./bin/activate-hermit
cargo test --manifest-path desktop/src-tauri/Cargo.toml --lib thread_catch_up
```

The opt-in live journey (`thread_catch_up_live_tests.rs`) runs the production
commands against a disposable local relay given in `BUZZ_CATCH_UP_TEST_RELAY`
and a freshly built agent given in `BUZZ_CATCH_UP_TEST_AGENT`. It also needs a
local model server at `http://127.0.0.1:11434/v1`. Set
`BUZZ_CATCH_UP_TEST_MODEL` to its model name (default `qwen3:4b`). Use synthetic
data only. This opt-in test exercises real generation, sharing, corrections,
stale-write rejection, and reload; ordinary focused tests need no model server.

Manual check with two identities on an isolated local relay:

1. In a channel both identities belong to, open a thread with a few replies and
   choose **Catch me up**. Confirm that every point opens its source message and
   that the thread's unread state does not change.
2. Choose **Share with this thread**. On the second identity, open the same
   thread. The shared recap appears with the first identity as its author.
3. From the second identity, correct a point. The correction shows the second
   identity and the time it was signed, and the original point text is
   unchanged.
4. From the first identity, without reloading, correct the same recap. The
   correction fails visibly as a conflict and the draft is kept. Reload,
   confirm both corrections are listed, and resubmit.
5. In a private channel, remove the second identity from the channel. It can no
   longer list the recap. Open channels retain their existing read permissions.
