# Agent files

`buzz agents export`, `import` and `edit` turn one of your local agents into an
`AGENT.md` directory and back, so you can keep an agent in Git, review a change
as a diff, and copy an agent to another computer.

The commands run on your computer and talk to the running Buzz desktop through
its [local credential provider](local-credential-provider.md), paired with
`buzz auth desktop pair`. The desktop reads the agent, and it applies a change
only after you approve it there.

## Commands

| Command | What it does |
|---|---|
| `buzz agents export <agent> [--out <dir>] [--name <name>] [--force]` | Writes `<dir>/AGENT.md`. `<dir>` defaults to `./<name>`, and `<name>` to the agent's display name as a slug. |
| `buzz agents import <dir> --dry` | Prints what an import would change, and changes nothing. |
| `buzz agents import <dir> [--new] [--workspace <path>] [--prune-channels]` | Prints the change, then asks the desktop to apply it. |
| `buzz agents edit <agent>` | Opens the agent as `AGENT.md` in `$VISUAL` or `$EDITOR`. When you save and quit, it imports the file. Quit without saving to cancel. |

`<agent>` is an npub, a hex public key, or the agent's exact name.

### Dry runs

Both the live agent and the file are rendered in the canonical form export
writes, and the output is a unified diff of the two, labelled
`live/<name>/AGENT.md` and `desired/<name>/AGENT.md`. Comments, key order and
blank lines in your file don't show up as changes. The first line names the
change:

```diff
UPDATE triage-bot: wake.events[new-pages]
--- live/triage-bot/AGENT.md
+++ desired/triage-bot/AGENT.md
@@ -45,6 +45,7 @@
     - 9b353519-0d6a-4c55-9a8e-5d3c8b1f2a10
     kinds:
     - 9
+    - 1111
```

It is `CREATE`, `UPDATE` with the fields that change, or `NO_CHANGE`.

### Import rules

- **The file is the whole configuration.** A field missing from the file goes
  back to its default. Interests, watches and timers missing from the file are
  deleted. A channel missing from the file is left only with
  `--prune-channels`.
- **The target is `identity.buzz`.** Import never matches an agent by name. A
  file without `identity.buzz` creates a new agent, and import writes the new
  npub into the file. `--new` creates a new agent from any file and leaves the
  file as it is.
- **No stale changes.** If the agent changed after the CLI read it, for
  example because it edited its own attention, the desktop refuses the change.
  Run the command again.
- **Runtime state stays.** Timer arming times, watch cursors and history are
  not in the file. An update keeps them; a new timer or watch starts at the
  time of the import.
- **Edits in the app still work.** The next dry run shows them as changes
  that importing the old file would undo. Export again to keep them.

## Format

```markdown
---
kind: Agent
version: 3
name: triage-bot
display_name: Triage Bot
description: First responder for pages.
enabled: true
identity:
  buzz: npub1...
avatar: https://media.example.org/triage.png
model: model-large
effort: high
buzz:
  community: wss://relay.example.org
  runtime: claude-code
  settings:
    scope: community
  channels:
  - channel: 9b353519-0d6a-4c55-9a8e-5d3c8b1f2a10
    role: member
  interests:
  - id: pages
    title: Pages
wake:
  timers:
  - id: morning
    enabled: true
    every: 1d
    max: unbounded
    interest: pages
    prompt: Look for pages with no owner.
  events:
  - id: new-pages
    enabled: true
    source: buzz
    interest: pages
    channels:
    - 9b353519-0d6a-4c55-9a8e-5d3c8b1f2a10
    kinds:
    - 9
    classifier:
      is_page: Is this a new page?
---

You are Triage Bot.

## When morning, new-pages

Confirm the page and open a thread.
```

The body before the first `## When` heading is the persona prompt. Each
`## When <wake ids>` section names one or more wakes, and its text is the
instructions of the Interest those wakes feed. A wake's Interest is its
`interest` field, or else the first wake id in its section. All the wakes in
one section must feed the same Interest. `buzz.interests` holds an Interest's
title, summary and pause, and the instructions of an Interest that no wake
feeds.

| In `AGENT.md` | In the agent |
|---|---|
| Body before the first `## When` | Persona prompt |
| `model`, `effort`, `buzz.settings` | Settings that sync for the agent type |
| `buzz.runtime` | Agent type |
| `display_name`, `description`, `avatar` | Profile |
| `## When` sections, `buzz.interests` | Interests |
| `wake.timers`, `wake.events` | Timers and event watches |
| `buzz.channels` | Channel memberships and roles |

Wake fields follow `agent-attention/v1`, with `every` (such as `30m`, `24h` or
`7d`) for a timer's interval and `max` (a count, default 1, or `unbounded`)
for how often it fires. A classifier question is its text, or an object with
`question`, `true`, `false`, `guidance` and `threshold`. Wake ids are 1 to 40
lowercase letters, digits and dashes.

The format is closed: an unknown or misspelled field is an error. Version 2
files are read too. Import refuses fields a Buzz agent can't run yet, such as
`skills`, `mcps`, `limits`, `wake.schedules`, `wake.session`, events from a
source other than `buzz`, and `enabled: false`. Export fails, naming the
object, for anything the format can't express, such as a plugin source watch.

The file leaves out secrets, keys, runtime state, the workspace folder, and
the agent's memory.

## Agent document

`agent_read` and `agent_apply` exchange the agent as JSON:

```json
{
  "version": 1,
  "agent": {"pubkey": "<hex>", "community": "wss://relay.example.org", "owner": "<hex>"},
  "type": "claude-code",
  "profile": {"name": "Triage Bot", "about": "First responder for pages.", "picture": "https://..."},
  "instructions": "You are Triage Bot.",
  "settings": {"model": "model-large", "effort": "high", "scope": "community"},
  "attention": {"objects": {"interest/pages": {"type": "interest", "instructions": "..."}}},
  "channels": [{"channel": "<uuid>", "role": "member"}]
}
```

- `agent` is absent in a document that creates an agent. `owner` is read-only.
- `attention.objects` maps each slug to its `agent-attention/v1` value without
  runtime state: no `since` on event watches and no `armed_at` on timers.
- Every object is closed, so a CLI that meets a field it doesn't know fails
  rather than dropping it.
