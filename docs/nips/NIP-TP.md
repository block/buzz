NIP-TP
======

Community Thread Policy
-----------------------

`draft` `optional` `relay`

**Depends on**: NIP-01, NIP-10 (thread markers), NIP-11 (relay information document), NIP-WP (admin command shape), NIP-43 (role state)

## Abstract

A community can choose one of two thread policies:

- `nested` (default): replies keep the parent they were sent to and render as a tree.
- `flat`: every reply in a thread is presented as a direct reply to the thread root, like Slack.

The policy is a **read-time presentation rule**. Events are signed, so the relay stores every reply exactly as the client sent it, with its original NIP-10 `root`/`reply` tags. Switching the policy back to `nested` restores the original tree.

## Kinds

| Kind | Name | Signer | Purpose |
|------|------|--------|---------|
| `9034` | Set Thread Policy | admin / owner | Command: set the community thread policy |

### `kind:9034` Set Thread Policy

```jsonc
{
  "kind": 9034,
  "content": "",
  "tags": [["thread_policy", "flat"]]   // or "nested"
}
```

- Exactly one `thread_policy` tag whose value is `flat` or `nested`. Anything else is rejected.
- Authorization is the same as `kind:9033` (NIP-WP): the actor must be `admin` or `owner`. On an open relay whose community has no admin/owner yet, any authenticated member may set it.
- The relay stores the value per community (`communities.thread_policy`). The last accepted command wins.

## Reading the policy

The relay serves the policy in its NIP-11 document as a top-level string field:

```jsonc
{ "thread_policy": "flat" }   // or "nested"
```

A missing field (older relay) means `nested`.

## Flattening rule

Let a reply `R` belong to the thread rooted at `T` (its NIP-10 `root`, or the relay's `root_event_id`). When the policy is `flat`:

1. `R`'s effective parent is `T` and its effective depth is `1`, whatever parent it was sent to.
2. A root's direct-reply count equals its descendant count.
3. A non-root reply has no replies of its own (reply count `0`, no nested summary).
4. Depth limits on thread reads (`depth_limit`, NIP-CW `depth`) of `1` or more include every reply in the thread.

### Relay

The relay applies the rule to its derived thread data before returning it: `parent_event_id`, `depth`, `reply_count` and `descendant_count`, and depth filters. This covers the thread-replies bridge query, NIP-CW thread windows and thread summaries. Raw signed events (REQ, `/query`) are returned unchanged, because their tags cannot be rewritten.

### Clients

Clients that read thread structure from event tags apply the same rule locally, using the NIP-11 policy. When the policy is `flat`, clients also send every new reply with the thread root as both its `root` and `reply` target, and do not offer "reply to this reply".

Clients that do not know this NIP keep showing the original tree. Their posts are still accepted and are shown flat by clients that do know it.

### Agents

Agent harnesses that pick a reply anchor use the thread root for every in-thread reply when the policy is `flat`, including agent-to-agent turns.

## Scope

The policy applies to every channel type. Forum channels are already shown flat by Buzz clients, so in practice it changes stream channels and DMs.
