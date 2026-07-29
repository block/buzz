# Workflow Engine

Buzz has a YAML-as-code workflow engine for channel automation. Workflows are defined in YAML files, scoped to a channel, and executed by `buzz-workflow`.

## Triggers

| Trigger | Description |
|---|---|
| `message_posted` | Fires when a message is posted matching a text pattern |
| `reaction_added` | Fires when a reaction is added matching an emoji |
| `schedule` | Cron-based scheduled trigger (ticks every 60s) |
| `webhook` | External HTTP webhook trigger |

## Actions

| Action | Description |
|---|---|
| `send_message` | Post a message to the channel |
| `send_dm` | Send a direct message to a user |
| `set_channel_topic` | Update the channel topic |
| `add_reaction` | Add an emoji reaction |
| `call_webhook` | Call an external webhook URL |
| `request_approval` | Request human approval before proceeding |
| `delay` | Wait before executing next action |

## Conditions

Conditions are expressions evaluated with `evalexpr`, with custom functions: `str_contains`, `str_starts_with`, `str_ends_with`, `matches_regex`, and more.

**Approval gates** are partially built (schema, API, and UI exist but executor persistence is pending).

**Related:**
- [buzz-workflow](../components/buzz-workflow)
- [Channel](../entities/channel)
- [EventPipeline](event-pipeline)
