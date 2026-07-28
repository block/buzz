# buzz-workflow

YAML-as-code workflow engine for channel automation.

**Key responsibilities:**
- Loading and parsing workflow YAML files
- Evaluating trigger conditions (message_posted, reaction_added, schedule, webhook)
- Executing action chains (send_message, send_dm, set_channel_topic, add_reaction, call_webhook, request_approval, delay)
- Cron scheduler (ticks every 60 seconds)
- `evalexpr` condition evaluation with custom string/regex functions

**Approval gates** (request_approval action): schema, API, and UI are built; executor persistence is pending.

**Related:**
- [WorkflowEngine](../concepts/workflow-engine)
- [EventPipeline](../concepts/event-pipeline)
- [Channel](../entities/channel)
