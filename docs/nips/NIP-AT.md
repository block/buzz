NIP-AT
======

Agent Attention
---------------

`draft` `optional`

An agent's **Attention** is what makes it act when nobody mentions it: its **Interests** (standing instructions), **event watches** (relay events that start a task), **timers** (tasks on a schedule) and **source watches** (outputs of a producer that its host runs). This NIP stores each of these objects as one [NIP-AR](NIP-AR.md) artifact in a private [NIP-CL](NIP-CL.md) system channel, the agent's **attention channel**. So any runtime that can sign in as the agent, on a device or in the cloud, reads the same Attention, and the agent's owner can read and change it from any client.

This NIP defines no new kind and no relay behavior. It is a profile: it names a channel label, an artifact type and a content format, and gives the rules that writers and readers follow so that they agree.

It replaces an earlier draft of NIP-AT that defined `kind:30183` encrypted addressable events and a relay gate. That draft did not merge.

## Roles

- **agent:** the key that the Attention belongs to. Its runtime runs the watches and timers.
- **agent owner:** the key the relay records as the agent's owner, from its [NIP-OA](NIP-OA.md) attestation. It is usually a human.
- **channel owner:** a member with the `owner` role in the attention channel. By default the agent and its agent owner are both channel owners.
- **runtime:** a process that runs the agent's Attention. A runtime signs as the agent.
- **writer:** a client that changes a record: the agent's own tools, its runtime, or a client of the agent owner.
- **reader:** a client that reads the set: a runtime, or a client that shows the agent's Attention.

## Relay support

A client stores Attention on a relay only when the relay's [NIP-11](11.md) document shows that it supports NIP-CL system channels and labels, and NIP-AR artifacts with sync and the `head` view. How a relay advertises each one is defined by that NIP. On any other relay, the client keeps Attention local and MUST NOT create an attention channel.

## Attention channel

| Setting | Value |
| --- | --- |
| Channel type | `system` |
| Visibility | `private` |
| Label | `agent-attention`, as a `t` tag |
| Creator (`P`) | the agent |
| Members | the agent and its agent owner, both with the `owner` role |

A private channel lets only its members read its artifacts, and NIP-AR lets only members with a writing role (`owner`, `admin` or `member`) write them. A member with the `guest` role can read but not write.

### Find

A client finds the attention channel with:

```
{"kinds": [39000], "#P": [<agent>], "#t": ["agent-attention"]}
```

`#P` asks for system channels (NIP-CL, "Asking for system channels"). The client keeps only events that the relay key signed, whose channel type is `system`, whose creator is the agent and whose labels include `agent-attention`. When more than one channel remains, every client uses the oldest one (NIP-CL, "Choosing one channel").

### Create

Only the agent creates its attention channel. A reader trusts a channel only when the agent created it, so a client of the agent owner MUST NOT create one. When Find returns nothing, the client of the agent owner shows that the agent's runtime has not started on this relay.

A runtime ensures the channel when it starts, and before the agent's first write. When Find returns nothing, the agent:

1. sends a kind:9007 with a new random channel ID, `["channel_type", "system"]`, `["visibility", "private"]` and `["t", "agent-attention"]`, and MAY add a `name` tag (the reference client sends `["name", "Attention"]`); then
2. sends a kind:9000 that adds its agent owner with `["role", "owner"]`.

The creator gets the `owner` role from the relay. An agent with no agent owner has a channel with only itself as a member.

If step 2 did not complete, a later ensure finds a channel whose only member is the agent. It then sends step 2 again. It changes the roster in no other case: after Create, the roster belongs to the channel owners.

If two processes of the agent both create a channel, every client uses the oldest, and the agent deletes the others (kind:9008) when it finds them.

### Roster and trust

A reader does not check who signed a record. The relay accepts a write only from a member with a writing role, and the reader trusts that gate. So the roster decides whose records apply. Each roster change is a signed kind:9000 or kind:9001, so the channel's history shows who could write at each time.

Either channel owner can add or remove members and change roles, as long as a channel owner remains. For example, the agent owner can make the agent a `guest`, so that only the agent owner changes the agent's Attention. A profile that needs a stricter rule, such as "only the agent owner's records apply", adds its own check on top of this one.

### Lifecycle

These follow NIP-AR, "Home channel lifecycle":

- **Archived:** writes fail with the archive reason. Readers keep reading, and runtimes keep running the set.
- **Deleted:** its records leave every read. The runtime runs no watches or timers. The agent's next ensure creates a new channel.

## Records

Each object is one artifact of type `buzz.agent-attention` in the attention channel:

```jsonc
{
  "kind": 45010,
  "pubkey": "<writer>",
  "tags": [
    ["ar", "1"],
    ["d", "<d>"],
    ["h", "<attention channel>"],
    ["type", "buzz.agent-attention"],
    ["title", "watch/project-messages"],
    ["op", "create"]
    // and ["prev", "<event id>"] on every later revision
  ],
  "content": "{\"schema\":\"agent-attention/v1\",\"slug\":\"watch/project-messages\",\"value\":{...}}"
}
```

- **Type.** One type for every object, so one sync filter reads the whole set. NIP-AR does not let a type change, so the object's kind is in `value.type`, not in the artifact type.
- **Slug.** `interest/<id>` for an Interest, `watch/<id>` for an event watch, timer or source watch. `<id>` matches `^[!-~]{1,64}$`. All watches share the `watch/` space. A later object family uses a new prefix and a new `schema`.
- **`d`.** NIP-AR's standard derivation (NIP-AR, "Deterministic identity and the ensure flow") with `h` = the attention channel, `type` = `buzz.agent-attention` and `key` = the slug. Two writers that add the same slug use the same `d`.
- **Title.** The slug. Only channel members can read it.
- **Content.** A UTF-8 JSON object with exactly the fields `schema`, `slug` and `value`, at most 65,535 bytes. `schema` is `agent-attention/v1`.
- **Deletion.** A `delete` revision has empty content and no `title`, and keeps the head's `root`, as NIP-AR defines.
- **Other tags.** This NIP defines none. Readers ignore tags they do not know, and editors keep them, as NIP-AR requires. A NIP-OA `auth` tag is not one of them: it belongs to the revision's signer, and the relay checks it against that signer. An editor never copies another revision's `auth` tag.
- **Modified time.** The revision's `created_at`, for display only.
- **Order.** Objects have no order. Readers list them by slug.

## Values

`value.type` is required and agrees with the slug: `"interest"` for `interest/`, and `"event"`, `"timer"` or `"source"` for `watch/`. Every field not marked optional is required. A value with an unknown field, a missing required field, a wrong type or a value over a limit is invalid. A new field needs a new `schema` string, and readers that know it ship before writers that write it.

Common types:

- **text:** a JSON string with no U+0000. **Not only whitespace** means that some character remains after leading and trailing white space is removed as ECMAScript's `String.prototype.trim` defines it.
- **label:** text of at most 256 bytes, with no control characters (U+0000–U+001F, U+007F–U+009F). A field can set another byte limit; tag values allow 1,024 bytes.
- **integer:** a JSON number with an integer value from 0 to 2^53 − 1. Writers SHOULD send it with no fraction or exponent; readers accept `1.0` and `1e3`. **time:** an integer of Unix seconds.
- **hex key** and **event ID:** 64 lowercase hex characters.
- **channel:** a lowercase UUID in the 8-4-4-4-12 form.

### Interest

| Field | Type | Meaning |
| --- | --- | --- |
| `type` | `"interest"` | |
| `instructions` | text, not only whitespace, ≤ 16,384 bytes | Given to the agent with every task that uses this Interest. |
| `title` | label, not only whitespace, optional | The Interest's name for people. |
| `summary` | text, not only whitespace, ≤ 1,024 bytes, optional | One or two sentences about the Interest, for people. |
| `paused` | boolean, optional | `true`: none of the Interest's watches, timers or source watches runs. Their own `enabled` values stay as they are. |

### Event watch

An event watch selects relay events. Each matching event starts one agent task.

| Field | Type | Meaning |
| --- | --- | --- |
| `type` | `"event"` | |
| `interest_id` | Interest `<id>` | The Interest to use. |
| `name` | label, optional | Display name. Without it, the client chooses what to show. |
| `enabled` | boolean | `false` stops the watch without removing it. |
| `since` | time, ≥ 1 | Only events with `created_at` ≥ `since` match. The writer sets it when it creates the watch. |
| `channels` | `"all"`, or an array of 1–100 unique channels | `"all"`: any channel the agent can read. Array: the event's `h` value is in the array. |
| `kinds` | array of 0–100 unique integers ≤ 65,535 | Empty: any kind. Otherwise the event's kind is in the array. |
| `authors` | array of 1–100 unique hex keys, optional | The event's `pubkey` is in the array. |
| `exclude_authors` | array of 1–100 unique hex keys, optional | The event's `pubkey` is not in the array. A watch MUST NOT have both `authors` and `exclude_authors`. |
| `tags` | object of 1–16 keys, optional | Each key is one ASCII letter other than `h`. Each value is an array of 1–32 labels of 1–1,024 bytes (see "label"); all values together are at most 16,384 bytes. For each key, the event has a tag `[key, v, ...]` with `v` in the array. |
| `is_reply` | boolean, optional | `true`: the event is a reply. `false`: it is not. A **reply** has an `e` tag with an event ID and the [NIP-10](10.md) `reply` marker. |
| `classifier` | object, optional | A model check (see "Classifier"). |
| `channel`, `root` | optional | Destination (see "Destination"). |

### Timer

| Field | Type | Meaning |
| --- | --- | --- |
| `type` | `"timer"` | |
| `interest_id` | Interest `<id>` | The Interest to use. |
| `prompt` | text, not only whitespace, ≤ 16,384 bytes | The timer's own instructions. |
| `enabled` | boolean | `false` stops the timer. |
| `interval_secs` | integer, 1–31,536,000 | Time between occurrences. |
| `armed_at` | time | The schedule start. The first occurrence is due at `armed_at + interval_secs`; later ones follow "Timer schedule". |
| `max_occurrences` | integer ≥ 1, or `null` | Occurrences allowed in one schedule. `1` is a one-shot; `null` is no limit. |
| `expires_at` | time, or `null` | No occurrence runs at or after this time. `null` is no deadline. |
| `channel`, `root` | optional | Destination. |

### Source watch

A source watch names a producer that the runtime's host offers. The runtime starts the producer, and each non-empty output starts one agent task.

| Field | Type | Meaning |
| --- | --- | --- |
| `type` | `"source"` | |
| `interest_id` | Interest `<id>` | The Interest to use. |
| `name` | label, optional | Display name. Without it, the client chooses what to show. |
| `enabled` | boolean | `false` stops the producer and drops its queued output. |
| `provider` | text matching `^[a-z0-9][a-z0-9._-]*/[a-z0-9][a-z0-9._-]*$` | The producer, as `<namespace>/<source>`. |
| `config` | object | The producer's settings. The provider defines and checks them. |
| `classifier` | object, optional | As for an event watch. Each output passes it. |
| `channel`, `root` | optional | Destination. |

Providers are host capabilities: each host offers its own set. A runtime runs only providers that its host offers. A source watch for any other provider is still a valid record, not a skipped one: the runtime does not run it, and reports that its provider is not available here. It MUST NOT delete the record, so the source runs again on a host that offers it.

### Destination

| Field | Type | Meaning |
| --- | --- | --- |
| `channel` | channel, optional | The conversation the task continues in, and its default place to post. Absent: an independent task. |
| `root` | event ID, optional | A thread root in `channel`. Requires `channel`. |

A destination grants no permission. A runtime holds delivery while the agent's membership in `channel` is not confirmed.

### Classifier

A classifier is `{"questions": {"<name>": <question>, ...}}` with 1–8 questions. A name matches `^[a-z][a-z0-9_]{0,31}$` and is not `true` or `false`. A question is an object:

| Field | Type |
| --- | --- |
| `question` | text ≤ 2,048 bytes |
| `true`, `false` | text ≤ 4,096 bytes, optional: what each answer means |
| `guidance` | text ≤ 8,192 bytes, optional |
| `threshold` | number greater than 0 and less than 1, optional; default 0.2 |

The `question`, `true`, `false` and `guidance` text of all questions is at most 16,384 bytes together.

The runtime asks a model for the probability that each question is true for the event or output. A question passes when its probability is at least its threshold, and the classifier passes when any question passes. The model and prompt belong to the runtime. If the model call fails (timeout, no capacity, service error, missing credentials) or returns an invalid probability for any question, the classifier **passes**, and the runtime records why.

## Limits

- A set holds at most 100 Interests, and at most 100 watches (event, timer and source together). A writer refuses a change over a limit.
- A reader that finds more valid objects than a limit allows sorts that group by slug, applies the first 100 and reports the rest as skipped.
- A NIP-AR sync fails when the set is larger than the relay's `max_events`, and it counts every head of the type, valid or not. So a writer also refuses to create a record while the channel holds 200 heads of type `buzz.agent-attention`.
- A writer MAY add lower limits on its own writes. For example, the reference client refuses a timer interval under 60 seconds from the agent's own tools.

## Writing

Every change edits exactly one object, and is one NIP-AR revision.

1. **Read.** Find the channel and sync once: send `REQ` with `{"artifact": "current", "kinds": [45010], "#h": [<channel>], "#type": ["buzz.agent-attention"]}`, wait for `EOSE`, then `CLOSE`. Only `EOSE` gives a complete set. `CLOSED`, a disconnect or a timeout before `EOSE` fails the change.
2. **Check.** Apply the change to that set and check the result: the value rules, the limits, and these rules across objects:
   - a watch names an Interest that exists;
   - an Interest is not removed while a watch, valid or skipped, names it;
   - a watch does not change its `type` or its `interest_id`.
3. **Publish** one revision and wait for the relay's `OK`:
   - **Add:** NIP-AR's ensure flow. Send `create`. On `conflict` or `duplicate`, read the head with the `head` view. If it is a deletion, send `restore` with `prev` set to it. If it is live with the same value, the add is done. If it is live with another value, fail: the object exists. If there is no head, the slug was retired; fail and ask for another `<id>`.
   - **Change:** `update` with `prev` = the head from step 1.
   - **Remove:** `delete` with `prev` = the head from step 1.
4. **Lost `OK`.** Read the head. If it is the revision sent, the change is done. If it is still the head from step 1, send the revision again. If it is another revision, fail with `conflict`. If the head cannot be read, do not send again: fail, and say that the change may not have been saved.
5. **Conflict.** Another writer changed the object after step 1. Fail with `conflict`. A writer MUST NOT send the revision again with only `prev` replaced. It shows the new head, and the agent or the agent owner decides whether to make the change again.

A writer MUST NOT write over a record that it skipped (see "Reading"), because it cannot check what it would replace. Only a removal can replace it, and only one that the agent owner confirms.

### Order

When one action changes more than one object, the writer publishes them in this order, so that no watch is left without its Interest:

1. removed watches;
2. added and changed Interests;
3. added and changed watches;
4. removed Interests.

Within each step, by slug.

### Who signs

| Write | Signer |
| --- | --- |
| Ensure the channel (kind:9007, kind:9000, kind:9008) | the agent |
| The agent's Attention tools | the agent |
| Runtime writes (see "Runtime writes") | the agent |
| Changes from the agent owner's client | the agent owner |
| Moving local Attention to the relay, and removals the agent owner confirms | the agent owner |

## Reading

1. **Find** the channel. If there is none, the set is empty. Check again when the agent writes, or after a delay.
2. **Sync** with the filter from Writing, step 1. Buffer each `EVENT` until `EOSE`. At `EOSE`, replace the whole set with the buffer. Do not merge it into the old set.
3. **Check each record.** A record is skipped, and reported, when any of these is true:
   - it does not have `["ar", "1"]`;
   - its content is not a JSON object with exactly `schema`, `slug` and `value`, or is over 65,535 bytes;
   - its `d` is not the derivation of its `slug`;
   - its `schema` is not one the reader knows;
   - its slug or value is invalid.

   A skipped record does not stop the other records. A reader keeps a record whose `d` does not match its slug under its `d`, so that it cannot hide the record it names.
4. **Live changes.** After `EOSE`, each `EVENT` names one record by its `d`. A reader removes that record, before it decodes any content, when the event is:
   - a kind:45011 removal (a move took the artifact out of the channel);
   - a kind:45010 deletion; or
   - a kind:45010 head that no longer matches the sync filter.

   Any other kind:45010 head replaces the record, after step 3.
5. **Errors.** On `CLOSED "resync: ..."` or `CLOSED "restricted: ..."`, start again from step 1. On `CLOSED "error: ..."` or a dropped connection, keep the last complete set, report the problem, and sync again after a delay. A runtime runs no watches or timers before its first complete set, unless it kept a complete set from an earlier read.

A reader shows skipped records to the agent owner, by slug and problem, so that the agent owner can remove them. A runtime does not run them.

A watch or timer can name an Interest that does not exist, for example after another writer removed it. The runtime still starts the task, with the event or the timer's `prompt`, and a note that the Interest was not found.

## Running

### Event watch matching

An event matches an event watch when all of these are true, in this order:

1. The watch is enabled, its Interest is not paused, and `created_at` ≥ `since`.
2. The event has exactly one `h` tag, and its kind is one that the runtime reads from channels. Which kinds that is depends on the host. The reference client reads kinds 5, 7, 9, 9000, 9001, 9002, 9005, 9008, 9022, 20002, 30620, 39000, 39002, 39005, 40002–40008, 40099, 40100, 45001–45003, 45010, 48100–48103 and 48106.
3. `channels`, `kinds`, `authors`, `exclude_authors`, `tags` and `is_reply` match. All present fields must match; the values in one array are alternatives. For a kind:7 reaction, `tags.e` matches the event that the reaction is for ([NIP-25](25.md)).
4. The agent is not its author, and it is not directly addressed to the agent. A directly addressed event goes to the runtime's mention path, never to a watch. An event is directly addressed when:
   - it is a kind:9, 40003, 40007 or 46010 with a `p` tag for the agent;
   - it is a kind:4 DM with a `p` tag for the agent; or
   - it is a kind:9 in a one-to-one direct-message channel with the agent.

   A reply to one of the agent's events, or a reaction to it, is not directly addressed. Only a watch can wake the agent for it.
5. The classifier, if any, passes. A runtime checks it last, because it calls a model.

Each matching watch starts its own task. One watch handles one event ID at most once.

A watch that appears or changes while the runtime runs catches up from `since`. When a runtime starts, it continues each watch from the position it saved. A watch with no saved position (a new host, or lost state) starts at the current time. A runtime bounds each catch-up and reports where it stopped.

### Timer schedule

The runtime keeps each timer's state on its host: occurrences used and the next due time. This state is never written to the channel.

- When the next due time comes, the timer is enabled, its Interest is not paused, `expires_at` has not passed and occurrences used < `max_occurrences`, the runtime starts one task with the Interest's instructions and the timer's `prompt`. It adds 1 to occurrences used and sets the next due time to now + `interval_secs`. It never runs a timer before its due time.
- A runtime that was stopped past one or more due times runs at most one occurrence, then continues from now.
- A new `armed_at` (a rearm) sets the next due time to `armed_at` + `interval_secs` and keeps occurrences used. Other edits keep the state: a new `interval_secs` applies after the next run, and a raised limit lets a spent timer run again at its next due time.
- **No state** (first seen, a host move, or lost state): the runtime counts every occurrence due at or before now as used and does not run it. The next due time is the first occurrence after now. So a timer never runs more often than its limits allow.

A timer is **spent** when occurrences used reach `max_occurrences` or `expires_at` has passed. The task for its last occurrence tells the agent that it is the last.

### Runtime writes

A runtime writes in two cases only:

- it removes a spent timer after the task for its last occurrence succeeds;
- it removes a source watch after every task from its run succeeds.

Each is a `delete` signed as the agent. The runtime first reads the record again (Writing, step 1). It deletes the record only if its value is still the one the task ran with, with `prev` = the head from that read. If the value changed, or the delete gets `conflict`, another writer changed the record during the task: the runtime keeps the record and applies the new head. A timer that is spent with no last task stays, and the runtime reports it as spent.

When a source run has a failed task, the runtime keeps the record, marks the run complete in its own state, and does not run it again until the record changes. A runtime with no such state runs a kept source once more.

When the agent cannot write in the channel (for example, it is a `guest`), the relay refuses a runtime write. The record stays: the runtime reports the spent timer, and keeps the completed source marked in its own state. A runtime MAY skip the write when it knows the agent's role.

## Example

An Interest and an event watch that uses it. Content is shown as JSON for reading; on the wire it is one JSON string.

```jsonc
// interest/reviews
{"schema": "agent-attention/v1", "slug": "interest/reviews", "value": {
  "type": "interest",
  "title": "Code reviews",
  "instructions": "When someone asks for a review in a thread, review the linked change and reply in that thread."
}}

// watch/review-requests
{"schema": "agent-attention/v1", "slug": "watch/review-requests", "value": {
  "type": "event",
  "interest_id": "reviews",
  "name": "Review requests",
  "enabled": true,
  "since": 1767225600,
  "channels": ["3f2b8c1e-5a7d-4e9f-8b6c-0d1e2f3a4b5c"],
  "kinds": [9],
  "is_reply": false,
  "classifier": {"questions": {"asks_review": {
    "question": "Does this message ask for a code review?"
  }}},
  "channel": "3f2b8c1e-5a7d-4e9f-8b6c-0d1e2f3a4b5c"
}}
```

The writer publishes `interest/reviews` first, then `watch/review-requests`. To remove both, it deletes the watch first.

## Security considerations

- **Readers trust the roster, not signers.** Any member with a writing role can add a watch that wakes the agent, or change its instructions. Keep the roster to the agent and its agent owner unless more writers are intended.
- **Instructions are untrusted input** to the agent, like any message. Writing a record grants no permission that the agent's runtime does not already give.
- **Titles and content are not encrypted.** The channel is private, so only members and the relay can read them. Put no secrets in Attention; a source's `config` names commands and paths, not credentials.
- **A destination grants nothing.** The runtime still checks the agent's membership in the destination channel.
- **Removals are history.** Every revision stays under the community's retention policy, so a removed instruction can still be read by members and the relay.

## Relation to other NIPs

- [NIP-AR](NIP-AR.md): records, revisions, sync, the `head` view and the standard `d`.
- [NIP-CL](NIP-CL.md): the system channel, its label and the rule that picks one channel.
- [NIP-OA](NIP-OA.md): who the agent owner is.
- [NIP-AP](NIP-AP.md): agent and team config use the same pattern in their own config channels. Attention has its own channel, so its roster can differ from the agent config channel's.
