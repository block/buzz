# Secure Workflows Mode

A workflow message carries a `buzz:workflow-mention` tag for each agent the
owner's stored step text names. That tag lets the message direct the agent on
the owner's behalf (see "Workflow Mentions" in [NOSTR.md](../NOSTR.md)).
Secure workflows mode keeps the tag off messages from runs that someone other
than the owner could have steered.

| Setting | Default |
|---------|---------|
| `BUZZ_SECURE_WORKFLOWS` | `false` |

With the setting off, the relay adds the tags for every run. With it on, a
`send_message` step adds them only when the run is eligible: its cause
qualifies and its steps qualify.

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

## When to turn it on

When people other than a workflow's owner can trigger it: by posting or
reacting in its channel, or through its webhook.

## Rollout

Any pod with the setting off still adds the tags, so the setting protects
agents only once every relay pod runs with it on. Keep client-side defenses in
place until then.

## What it does not cover

- An agent that acts on a workflow message still reads thread, pull request
  and repository text that anyone can write, the same as for a direct mention.
- A replayed workflow message behaves as it does with the setting off.
