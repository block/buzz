# Slack Import

`buzz import slack` copies a Slack workspace export into a Buzz community:
channels, messages, threads, and reactions. It is an ordinary client. It needs
no relay configuration, no operator access, and no permissions beyond those of
the member who runs it, so it works the same against a relay you host and one
hosted for you.

```bash
buzz import slack --export-dir ./export --team-id T0266FRGM --dry-run    # plan only
buzz import slack --export-dir ./export --team-id T0266FRGM              # import
# Merge separate Slackdump public/private passes:
buzz import slack \
  --export-dir ./public-export \
  --export-dir ./private-export \
  --team-id T0266FRGM --dry-run
# Write into channels that already exist in Buzz:
buzz import slack --export-dir ./export --team-id T0266FRGM \
  --channel-map ./slack-buzz-crosswalk.csv
# Populate private channels and DMs with the people who hold a Buzz key:
buzz import slack --export-dir ./export --team-id T0266FRGM \
  --user-map U060=npub1abc,U081=npub1def
```

`--team-id` is the Slack workspace id (for example `T0266FRGM`). It namespaces
every provenance reference and channel UUID, so ids from two workspaces cannot
collide.

## The model: an archive, signed by the importer

Buzz events are signed by their author, and a relay accepts an event only when
its `created_at` is within fifteen minutes of the relay's clock. An importer
that is an ordinary client therefore cannot publish a message as the person who
wrote it, or date it in the past. This tool does neither. It publishes an
**archive**:

- **Every event is signed by the CLI identity.** No key is generated for, or
  held on behalf of, anyone else, and no event claims another person's
  signature.
- **Every event is stamped with the time of the import.**
- **The original author and time open each message**, where every client can
  show them:

  ```
  **Ada Lovelace** (2024-03-04 09:43 UTC): the build is green again
  ```

- **Every event carries a provenance tag** naming the Slack record it came
  from:

  ```json
  ["imported_from", "slack", "T0266FRGM/C024BE91L/1709545400.000400"]
  ```

  The reference is `<team id>/<channel id>/<message ts>`, using Slack's ids
  verbatim. A reaction appends its Slack name (`…/1709545400.000400/tada`) and
  a DM-open event uses `<team id>/<conversation id>`. The tag marks an event as
  imported; it does not claim that the signer is the original author. Its shape
  follows the convention proposed in
  [#3306](https://github.com/block/buzz/issues/3306).

History that must keep its original timestamps and per-author signatures needs
relay support and each author's participation; see
[Relation to consented import](#relation-to-consented-import).

## What gets imported

| Slack | Buzz | Notes |
|-------|------|-------|
| Public channels (`channels.json`) | open stream, kind `9007` create + `9002` topic/purpose | UUID derived from the team and channel ids, recorded in the state file |
| Private channels (`groups.json`, or Slackdump private records in `channels.json`) | **private** stream | Visibility is narrowed before any history write; never promoted to open |
| DMs (`dms.json`) / MPIMs (`mpims.json`) | native Buzz DM (`41010` open) | The exact active-human participant set must be mapped before history can be written |
| Messages (per-day JSON) | kind `9` stream message, `h`-tagged | Attribution header in the content; `created_at` is the import time |
| Threads (`thread_ts`) | NIP-10 `e` reply tags | Slack threads are flat; every reply is a direct reply to the root |
| Reactions | kind `7` | One reaction per distinct emoji, signed by the importer |
| Classic attachments / Block Kit | Markdown text, fields, lists, quotes, links | Interactive app actions are not executable after import |
| Files | Links appended to message content | Blobs are not re-hosted (see Limitations) |
| Custom emoji | — | Use `scripts/grab-emoji.sh` (separate tool, needs a Slack API token) |

### Message subtypes

Slack tags non-plain messages with a `subtype`. The importer is deliberately
conservative: it imports genuine conversation and skips system noise.

| Slack subtype | Handling |
|---|---|
| *(none)* — a plain message | Imported |
| `thread_broadcast` / `reply_broadcast` — a reply also broadcast to the channel | Imported (as a reply) |
| `bot_message` and app/integration posts | Imported when text, files, classic attachments, or blocks are renderable |
| `file_share`, `file_comment`, `me_message`, huddle/canvas sharing content | Imported when renderable |
| `channel_join` / `channel_leave` / `channel_topic` / `channel_purpose` and other system notices | Skipped |
| `message_changed` (edits) and `message_deleted` / tombstones | Skipped — only final, still-present text is imported |
| Message with empty top-level `text` | Block Kit rich text, classic attachment fallback/text/fields, and files are rendered; only genuinely contentless records are skipped |

Unknown future subtypes are accepted when they carry renderable conversation
content. A dry run makes the filtering auditable: it reports raw record totals
and separate system, mutation, contentless, and duplicate counts.

## Conversations and people

`--export-dir` is repeatable. This matters for Slackdump exports where public
and accessible-private conversations were exported in separate passes. The
importer merges user and conversation indexes, deduplicates identical Slack
timestamps across roots, and rejects conflicting conversation ids or target
names before connecting to the relay. `users.json` and Enterprise
`org_users.json` are both supported.

`--channel-map` writes into channels that already exist in Buzz instead of
creating new ones. The file is CSV with `slack_channel_id` and
`buzz_channel_id` columns (extra columns are ignored), or a JSON array of
objects with those two fields. Every Slack id must exist in the loaded export,
Buzz UUIDs must be unique, and a map cannot override a different UUID already
recorded in the state file. An adopted archived channel is unarchived for the
import and archived again afterwards. DMs and MPIMs cannot be adopted this way:
they are opened through Buzz's native DM command so the relay can enforce the
exact participant set.

`--user-map` tells the importer which Slack users hold a Buzz key, as
`SLACKID=npub-or-hex` pairs. It takes public keys only and is used for
membership, not attribution:

- **Private channels** become private Buzz streams, and each mapped member is
  added. Unmapped, deactivated, and bot users are counted in the summary and
  never replaced by another account.
- **DMs and MPIMs** need a mapping for every active human participant, because
  a Buzz DM's participant set is fixed when it is opened. The CLI identity must
  itself be the mapped key of one participant; otherwise the importer would add
  the person running it to someone else's private conversation. Buzz supports
  2–9 DM participants and one DM per participant set, so two Slack
  conversations that resolve to the same set are blocked rather than merged. A
  dry run lists each blocked conversation with its reason.

## Ordering, pacing, and resume

- **Order is kept by spacing messages one second apart.** A relay orders a
  channel by `created_at` in whole seconds, so each imported message gets its
  own second, in Slack order. Imports therefore run at no more than one message
  per second — about an hour per 3,600 messages — and no message is ever dated
  in the future. A relay's per-key rate limit (60 messages a minute by default)
  allows the same pace; when the relay answers 429 the importer backs off and
  resends the same signed event.
- **Thread roots come before their replies.** Conversations are imported one at
  a time, sorted by Slack `ts`. When a thread root is itself skipped (a
  contentless or system record), its replies are imported as top-level messages
  rather than dropped, with a warning per reply.
- **A state file makes the import resumable.** By default it is
  `<export-dir>/buzz-import-state.json`, with a `.journal` file beside it. It
  maps each Slack conversation to its Buzz channel and each
  `"<channel>:<ts>"` to the event that was published, and it is pinned to one
  workspace. Re-running skips everything already recorded, including on a newer
  export of the same workspace.
- **A message is never published twice.** Its signed event id is journaled
  before it is sent. If the run is interrupted, or the relay's answer is lost,
  the next run looks that id up on the relay and either adopts the stored event
  or signs the message again.
- **A partial run exits non-zero.** The summary's `failed` count is the number
  of writes the relay did not confirm; run the same command again to finish.
  `skipped` counts records that can never be imported, such as a message over
  the relay's size limit.

## Before you run it

- **Run a dry run first.** `--dry-run` reads the export and the state file,
  reports what would be written, and never contacts the relay.
- **Import before the community is busy.** Imported messages are new messages
  to the relay: members of the destination channels see them as fresh activity,
  and workflows or agents that react to channel messages will see them too.
  Import into channels before adding people, agents, or message-triggered
  workflows, or expect the noise.
- **Existing messages stay above imported ones.** History written into a
  channel that already has messages lands after them. Import first, then start
  using the channel.
- **Mentions are not `p`-tagged.** A Slack `<@U123>` becomes plain `@Name`
  text, so an import does not fill anyone's mention feed.

## Text conversion (mrkdwn → markdown)

Code blocks and inline code are preserved verbatim. Outside code:

- `<@U123>` → `@DisplayName`
- `<#C123|name>` → `#name`
- `<http://url|label>` → `[label](url)`; `<http://url>` → `url`
- `<!here>` / `<!channel>` / `<!everyone>` → `@here` / `@channel` / `@everyone`
- `&lt;` `&gt;` `&amp;` unescaped
- `*bold*` → `**bold**` (conservative, single-line, non-space-adjacent)

When a message opens with block-level markdown (a code fence, quote, list, or
heading), the attribution header is placed on its own line so the block still
renders.

## Limitations

- **Timestamps are the import time.** The original time is in each message's
  header and in its `imported_from` reference, not in `created_at`, so date
  filters and "jump to date" see the day of the import.
- **Messages appear under the importer's identity.** The original author is
  named in the header. Nothing here lets a person later take over the messages
  they wrote in Slack.
- **Per-reactor identity is not preserved.** One key can react to a message
  once per emoji, so several people's 👍 become one.
- **Files are not re-hosted.** Slack file URLs, which require Slack
  authentication, are appended as links.
- **Private coverage is bounded by the export and the user map.** Conversations
  absent from the export cannot be recovered. Review
  `private_memberships_unmapped`, `dm_conversations_blocked`, and
  `dm_messages_blocked` in the dry run before writing.
- **Edit history is not reconstructed.** The export holds only final text.
- **Interactive Slack app actions do not survive.** Attachment text, fields,
  links, and Block Kit rich text are converted to markdown; buttons, menus, and
  callbacks cannot be replayed.
- **Slack workflows are not translated** to `buzz-workflow` YAML.

## Relation to consented import

[#8981](https://github.com/block/buzz/pull/8981) proposes NIP-CI, in which the
participants of a conversation each sign their own past messages and the relay
accepts the original timestamps inside a session everyone has consented to.
That is the right tool when the authors are present and each holds a key. It
keeps real authorship and real dates, which this importer cannot.

This importer covers the other case: an operator bringing over a whole
workspace as a readable record, including the words of people who have left and
will never hold a key. The two do not overlap on the wire. An archive import
never carries the `import` tag that NIP-CI reserves, and it asks the relay for
nothing a member cannot already do.

## CLI reference

```
buzz import slack
  --export-dir <PATH>      unzipped Slack export directory; repeat to merge roots
  --team-id <ID>           Slack workspace id (required, e.g. T0266FRGM)
  --state <PATH>           state file (default: <export-dir>/buzz-import-state.json)
  --channel-map <PATH>     CSV/JSON map: Slack channel id → existing Buzz UUID
  --channels <a,b,c>       import only these conversation names or Slack ids
  --dry-run                parse and report what would be imported; no writes
  --skip-reactions         do not import reactions
  --user-map <MAP>         SLACKID=npub-or-hex,… Slack users who hold a Buzz key
```

Output follows CLI conventions: progress on stderr and a final JSON summary on
stdout (`channels_created`, `messages_imported`, `reactions_imported`,
`private_members_added`, `private_members_unmapped`, `skipped`, `failed`,
`warnings`, `state_file`). A dry run instead reports conversation classes,
raw and skipped record counts, reaction use, private membership coverage,
channels to adopt, and blocked DMs.
