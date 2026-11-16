# Owner-Only Agent Mentions

A workflow message carries a `buzz:workflow-mention` tag for each agent the
owner's stored step text names. That tag lets the message direct the agent on
the owner's behalf (see "Workflow Mentions" in [NOSTR.md](../NOSTR.md)).
Owner-only agent mentions keep the tag off messages from runs that someone
other than the owner could have steered.

| Setting | Values | Default |
|---------|--------|---------|
| `BUZZ_WORKFLOW_AGENT_MENTIONS` | `any`, `owner` | `any` |

Any other value fails startup. With `any`, the relay adds the tags for every
run. With `owner`, a `send_message` step adds them only when the run is
eligible: its cause qualifies and its steps qualify. The setting is
relay-wide: it applies to every community the relay serves.

## Cause

The relay records what caused each run when it creates it.

| Trigger | Cause | Eligible |
|---------|-------|----------|
| Event (message, reaction, diff) | the event's signer; for a reaction, the reactor, not the author of the message reacted to | only if that is the owner |
| Schedule (cron or interval) | the timer | yes |
| Webhook | unknown | never |
| Manual | whoever ran it | only if that is the owner |

Only the owner or a timer makes a run eligible. The owner's other agents count
as someone else.

Runs created before causes were recorded have no cause, and are never eligible.

## Steps

Every step of the workflow definition must be `send_message` or `delay`. Any
other step makes the run ineligible, even one the run skips. A run that
resumes after an approval is ineligible too.

## What people see

An ineligible run's messages still post, with their `p` tags, so mentioned
people are still notified. No agent acts on them.

When an ineligible run's message names a channel member in the workflow's
stored text, and that name is also in the posted text, the relay leaves off
that member's `buzz:workflow-mention` tag. The step's output in the run's
trace, which the owner sees in the app's run history, then records why:

```json
{"sent": true, "event_id": "…", "agent_mentions": {"withheld": true, "reasons": ["cause", "steps"]}}
```

`reasons` lists every reason that applies, in this order:

| Reason | Meaning |
|--------|---------|
| `cause` | Neither the owner nor a timer started the run. |
| `steps` | The definition has a step other than `send_message` or `delay`. |
| `resumed` | The run resumed after an approval. |

The key is absent when no tag was left off: on eligible runs, with `any`, and
for messages with no such mention, such as plain text, a mention that comes
only from substituted text, or a name that matches no member or more than one.

## Logs and metrics

Each withheld step logs at info level `owner-only agent mentions withheld
buzz:workflow-mention tags`, with `community`, `workflow`, `run_id`, `step`
and `reasons`. The counter `buzz_workflow_agent_mentions_withheld_total`
increases by one for each reason, labelled `reason` (`cause`, `steps` or
`resumed`), so one step with two reasons counts under both.

## Discovery

The NIP-11 relay information document always carries
`"workflow_agent_mentions": "any"` or `"owner"`. A client that finds no field
is talking to a relay older than this setting.

## When to use `owner`

When people other than a workflow's owner can trigger it: by posting or
reacting in its channel, or through its webhook.

## Rollout

Any pod set to `any` still adds the tags, so the setting protects agents only
once every relay pod runs with `owner`. Keep client-side defenses in
place until then.

## What it does not cover

- An agent that acts on a workflow message still reads thread, pull request
  and repository text that anyone can write, the same as for a direct mention.
- A replayed workflow message behaves as it does with `any`.
