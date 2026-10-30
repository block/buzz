# Slash commands

Composer slash commands give Slack users the shortcuts they already know,
mapped to the Buzz action that already exists. This document is the shared
contract for desktop and mobile: recognition, autocomplete, execution, and the
command set. Platforms differ only where a command is marked unavailable.

A command never adds a capability. It is a typed entry point to an action the
app already performs from its normal UI, through the same code path, the same
permission checks, and the same relay events. The one exception is `/remind`,
which adds standalone (note-only) reminders and a time parser.

## Agents keep their commands

Agents already receive slash commands: buzz-acp passes a leading `/command`
to the agent's ACP connector (`extract_slash_command` in
`crates/buzz-acp/src/queue.rs`). Buzz commands must not take that away.

- A draft that starts with a mention is never a Buzz command. `@Eva /status`
  sends a message to Eva. The thread address lock's `@Agent ` prefix counts.
- A draft whose first token is not a Buzz command on this platform sends as a
  normal message, unchanged. In a DM with an agent, `/review` still reaches the
  agent. There is no "not a valid command" error.
- To send an agent a command whose name is also a Buzz command, mention the
  agent first.

## Where commands run

| Platform | Composers with commands | Composers without |
|---|---|---|
| Desktop | Channel and DM main composer, thread reply composer | Edit, forum, new message, inbox reply, project agent chat |
| Mobile | Channel and DM compose bar, thread compose bar | Forum post and forum thread compose bars, edit sheet |

In a thread, a channel-scoped command acts on the thread's channel. `/shrug`
in a thread posts a thread reply.

## Recognition

Trim leading whitespace from the draft. The draft is a command when its first
token is `/` followed by a registered name or alias that is available on this
platform and in this composer, and the token ends at whitespace or the end of
the draft. Matching is case-insensitive. `//away`, `/away-now`, and
`/usr/bin/foo` are not commands.

A command draft that also has attachments is rejected with a visible error;
nothing is published and the draft and attachments stay.

## Autocomplete

- Opens while the caret is in the first token and that token is `/` plus zero
  or more name characters, with only whitespace before it.
- Lists commands available on this platform, in this composer and context, and
  not known to be denied to this user (see the permission column below).
  Hidden aliases are never listed, but an exact alias still runs its command.
- Order: commands whose name starts with the typed text, alphabetical by name.
  No fuzzy matching.
- Each row shows `/name`, the argument hint, and a one-line description.
- Choosing a row inserts `/name ` (with a trailing space). It never runs the
  command; send runs it.
- Keyboard and screen-reader behavior matches `@` autocomplete on the same
  platform: arrow keys move, Enter and Tab choose, Escape dismisses, one
  accessible label per row.
- Arguments reuse the existing `@` and `#` autocompletes and their exact-key
  resolution (see [mention-editor.md](mention-editor.md)). An ambiguous typed
  name fails visibly; it never fans out to every identity with that name.

## Execution

- Send (Enter or the send button) runs the command instead of publishing the
  draft.
- The draft clears only after the command succeeds. On any failure — usage
  error, permission denial, relay rejection, offline — the app shows the reason
  and keeps the draft. Failures are never logged-and-ignored, and nothing is
  queued for automatic retry.
- A usage error shows the command's usage line and publishes nothing.
- The relay is the authority. The client hides commands it knows the user
  cannot run; a relay rejection still surfaces as an error.
- A command reuses its existing action path, including any confirmation that
  path already shows (for example, archive). Commands add no new confirmations.
- Success feedback uses the platform's existing toast or snackbar (for example
  "Status set"). Commands that navigate (`/dm`, `/join`, `/who`, `/gif`,
  `/huddle`) show no toast.

## Command set

Context: **any** = channel, DM, or thread; **channel** = a non-DM channel or a
thread in one.

| Command | Hidden aliases | Arguments | Behavior | Context | Hidden when | Desktop | Mobile |
|---|---|---|---|---|---|---|---|
| `/away` | | — | Set the manual presence override to away. | any | — | ✅ | ✅ |
| `/active` | | — | Clear the override (desktop `auto`, mobile online). | any | — | ✅ | ✅ |
| `/status` | | `[emoji] <text>` \| `clear` | Set custom status with no expiry, or clear it. No arguments opens the existing set-status dialog or sheet. | any | — | ✅ | ✅ |
| `/invite` | | `@user…` | Add the users to the channel. | channel | user cannot add members | ✅ | ✅ |
| `/remove` | `/kick` | `@user…` | Remove the users from the channel. | channel | user is not owner or admin | ✅ | ✅ |
| `/join` | | `#channel` | Join the open channel and open it. Already a member: open it. | any | — | ✅ | ✅ |
| `/leave` | `/part` | — | Leave the channel. | channel | — | ✅ | ✅ |
| `/who` | `/members` | — | Open the member list. | any | — | ✅ | ✅ |
| `/topic` | | `<text>` | Set the channel topic. | channel | user may not post | ✅ | — |
| `/rename` | | `<name>` | Rename the channel. | channel | user cannot manage the channel | ✅ | ✅ |
| `/archive` | | — | Archive the channel. | channel | user cannot manage the channel | ✅ | ✅ |
| `/mute` | | — | Toggle mute for the channel. | any | — | ✅ | ✅ |
| `/dm` | `/msg` | `@user… [message]` | Open the DM (up to the DM participant limit) and open it. With a message, send it there first. | any | — | ✅ | ✅ |
| `/huddle` | | — | Start a huddle in the channel, or join the one already running. | any | — | ✅ | ✅ |
| `/gif` | `/giphy` | `[query]` | Open the GIF picker, pre-filled with the query. | any | relay has no GIF endpoint | ✅ | — |
| `/shrug` | | `[text]` | Send `<text> ¯\_(ツ)_/¯`. The rendered message must show the shrug exactly; escape for markdown as needed. | any | — | ✅ | ✅ |
| `/remind` | | `me <what> <when>` | Create a standalone reminder. See below. | any | — | ✅ | ✅ |

`/status` emoji: the first argument is the emoji when it is a `:shortcode:` that
resolves on this platform or a single emoji grapheme; otherwise the whole
argument string is the text and the status has no emoji.

`/topic` ships on desktop only if desktop displays the channel topic somewhere.
If it does not, leave `/topic` out; a command must not change state the user
cannot see.

Not mapped, because Buzz has no equivalent: `/dnd`, `/collapse`, `/expand`,
`/me`, `/feed`, `/apps`, `/shortcuts`, `/prefs`. `/search` is not mapped
because neither app can open search with a query.

## `/remind`

```
/remind me [to] <what> <when>
/remind me <when> [to] <what>
```

- Reminders are personal. Any target other than `me` is a usage error:
  "Reminders are personal. Use /remind me …".
- `<when>` is matched first at the end of the text after `me`, then at the
  start; at each end the longest matching phrase wins (`tomorrow at 4pm`, not
  `at 4pm`). The rest, minus one leading `to `, is the note. An empty note or an
  unmatched `<when>` is a usage error.
- The reminder is standalone: a note and a due time, no target message. It is
  delivered and listed through the same paths as message reminders, with the
  note as its text and no jump-to-message action.
- Times use the device's local time zone. Minutes and hours are exact
  durations; days, weeks, and named days are wall-clock arithmetic, so they
  keep the same local time across a DST change.
- The due time must be after now and at most 365 days ahead.

### `<when>` grammar

Case-insensitive. `T` is `3pm`, `3 pm`, `3:30pm`, `15:00`, `noon`, or
`midnight`. An hour with neither `am`/`pm` nor a colon (`at 3`) is rejected as
ambiguous.

| Form | Due |
|---|---|
| `in N minute(s)\|min(s)\|hour(s)\|hr(s)\|day(s)\|week(s)` (`N` is a positive integer, `a`, or `an`) | now + N units |
| `at T` | today at `T`; tomorrow at `T` if that is at or before now |
| `tomorrow` | tomorrow 09:00 |
| `tomorrow at T` | tomorrow at `T` |
| `[on\|next] <weekday> [at T]` | the next such weekday strictly after today, at `T` or 09:00 |
| `next week` | next Monday 09:00 |

### Vectors

Now is Wednesday 2026-10-28 10:00 local, in `America/Chicago` (DST ends
2026-11-01). Both platforms must pass all rows.

| `<when>` | Due |
|---|---|
| `in 20 minutes` | 2026-10-28 10:20 |
| `in an hour` | 2026-10-28 11:00 |
| `in 2 hrs` | 2026-10-28 12:00 |
| `in 3 days` | 2026-10-31 10:00 |
| `in 1 week` | 2026-11-04 10:00 |
| `at 3pm` | 2026-10-28 15:00 |
| `at 3:30 pm` | 2026-10-28 15:30 |
| `at 15:00` | 2026-10-28 15:00 |
| `at 9am` | 2026-10-29 09:00 |
| `at 10am` | 2026-10-29 10:00 |
| `at noon` | 2026-10-28 12:00 |
| `at midnight` | 2026-10-29 00:00 |
| `tomorrow` | 2026-10-29 09:00 |
| `tomorrow at 4pm` | 2026-10-29 16:00 |
| `on friday` | 2026-10-30 09:00 |
| `Friday at 2pm` | 2026-10-30 14:00 |
| `on wednesday` | 2026-11-04 09:00 |
| `next monday` | 2026-11-02 09:00 |
| `next week` | 2026-11-02 09:00 |
| `at 3` | error |
| `at 25:00` | error |
| `at 13pm` | error |
| `in 0 minutes` | error |
| `in 400 days` | error |
| `yesterday` | error |

| Command | Result |
|---|---|
| `/remind me to submit the report in 20 minutes` | note `submit the report`, 2026-10-28 10:20 |
| `/remind me tomorrow at 4pm to call Sam` | note `call Sam`, 2026-10-29 16:00 |
| `/remind me check the deploy at 3pm` | note `check the deploy`, 2026-10-28 15:00 |
| `/remind me in 2 hours` | usage error (no note) |
| `/remind me to stretch` | usage error (no time) |
| `/remind @sam to stretch in 1 hour` | usage error (personal) |
