# Actionable Brief Decisions Design

## Outcome

Turn each Daily Command Brief decision into a short action card that helps the
Commanding Officer decide and then shows whether the direction is being acted
on.

## User experience

Each decision shows:

- **COA A — Recommended:** the adviser's recommended action.
- **COA B — Alternative:** one credible alternative, when one exists.
- **Your direction:** a spell-checked text field that also accepts normal macOS
  or iPhone keyboard dictation.

The user presses **Direct COA A**, **Direct COA B**, or **Issue direction**.
That click is the approval. There is no second confirmation or approval step.
The direction is sent immediately to the Chief of Staff with authority to use
the existing agents and connected systems needed to carry it out.

Sources do not appear in the main decision card. They remain available in the
brief's existing collapsed evidence area.

## Tracking

Each issued direction has one compact state:

- `Queued`
- `In progress`
- `Blocked`
- `Completed`
- `Failed`
- `Stalled`

The Chief of Staff receives a stable direction identifier and is asked to
report `IN PROGRESS`, `COMPLETE`, `BLOCKED`, or `FAILED` using that identifier.
Existing agent-turn activity also moves the item to `In progress`. If neither
agent activity nor a status message is observed for five minutes, the item is
shown as `Stalled`. A stalled item can be sent again from the card.

The state is stored locally and survives a desktop restart. It contains only
the direction, channel and agent identifiers, timestamps, and short status
text. The existing Buzz conversation remains the detailed activity record.

## Implementation boundaries

- Extend pending brief proposals with `alternativeText`. Historical briefs
  without this field remain readable and simply omit COA B.
- Use the existing Chief of Staff persona, managed-agent start flow, DM channel,
  user message command, relay subscription, and active-turn store.
- Do not add a workflow engine, receipt subsystem, new service, or separate
  approval layer.
- Do not build a custom speech-recognition pipeline. The normal text area uses
  native device dictation.
- An adviser may omit a proposal when no command decision is required. It must
  not manufacture a decision to fill the brief.

## Failure behaviour

- Failure to start the Chief of Staff or send the direction is immediately
  shown as `Failed` with a short useful error.
- Agent or connector problems reported by the Chief are shown as `Blocked` or
  `Failed`.
- Re-sending uses the same stable direction identifier so the Chief can resume
  rather than create unrelated duplicate work.
- A completed direction remains visible as a compact status; future briefs do
  not re-offer the same action identifier as an undecided item.

## Acceptance

- A generated decision contains a concise recommended COA and credible
  alternative.
- Selecting either COA sends the direction without another approval.
- Free text and device dictation can issue a different direction.
- Chief of Staff activity changes the card from queued to in progress.
- Explicit completion, blockage, or failure messages update the matching card.
- Five minutes without activity marks an issued direction stalled.
- Status survives restart and the detailed Chief of Staff conversation can be
  opened from the card.

