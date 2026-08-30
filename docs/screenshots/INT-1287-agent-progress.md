# Agent progress without the junk feed

Calling an agent that talks to Cursor used to dump every tool start/complete
and every assistant fragment into the channel (`⚙ shell`, `✓`, `✗`, leftover
prose). That is not status.

## What you see now

Desktop leftover/historical progress lines collapse into one expandable row:
`Working · N updates · latest: …`. Expand the row to see the tool list.
The real agent answer stays a normal message.

## Screenshots

Collapsed — one Working row, then the answer:

![Collapsed agent progress](INT-1287-progress-collapsed.png)

Expanded — tool lines stay under the row:

![Expanded agent progress](INT-1287-progress-expanded.png)
