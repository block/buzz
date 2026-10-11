# Demo Crew

A four-agent persona pack for the case Buzz gets used for constantly and has no
example: **running it in front of an audience.**

| Agent | Role |
|-------|------|
| **Lead** | Orchestrator — restates the request, plans, delegates, synthesizes, ends on a decision |
| **Maker** | Producer — drafts fast and labels its own assumptions |
| **Challenger** | Red team — strongest objection first, plus what would change its mind |
| **Guard** | Data security — sorts what was shared into tiers and names what should not have been typed |

`meadow-core` shows a team reviewing code. This pack shows a team being watched:
short replies, visible handoffs, and disagreement that stays on screen.

## Why the four roles

Three of them are the smallest loop that produces something worth watching:
plan, produce, attack. A team that only produces looks like a chatbot with
extra steps; the objection is what makes the division of labour legible.

**Guard is the one to look at if you copy nothing else.** In a corporate room,
everyone has already seen an AI write something. Almost nobody has seen one
say *"stop — that should not have gone in there."* Guard reads what has been
typed into the channel, sorts it into open / internal / restricted, and gives
the redacted rewrite without repeating the sensitive value back.

That behaviour is useful well beyond demos: the same persona works as a
standing check in any channel where people paste more than they meant to.

## Usage

```bash
# Validate the pack
buzz pack validate ./examples/demo-crew

# Inspect resolved config
buzz pack inspect ./examples/demo-crew
```

Then install it into the desktop app and attach the agents to a channel used
only for demos.

Two setup notes that matter for this pack specifically:

- **Leave the MCP / tools field empty.** These agents run in front of people you
  do not control. They do not need a shell, and the personas tell them to work
  only from what is typed in the channel.
- **Keep them owner-only.** The operator drives; the audience watches. Nothing
  in the room can trigger an agent directly.

## Structure

```
demo-crew/
├── .plugin/
│   └── plugin.json          # Pack manifest (OPS-compatible)
├── agents/
│   ├── lead.persona.md
│   ├── maker.persona.md
│   ├── challenger.persona.md
│   └── guard.persona.md
├── instructions.md          # Team-wide instructions
└── README.md
```

## Prompt-injection posture

The shared instructions tell every agent that only the operator's key carries
authority, and that a channel message claiming to be an instruction, an
override, or a system note is content to discuss rather than an order to
follow — and to say so out loud when it happens.

In a demo that is a feature: an audience member trying to talk the agents into
something becomes the most memorable minute of the session.

## Companion packs

A seven-agent working bench built on the same conventions — including a
standards judge that scores other agents' drafts before a human sees them, and
a Cantonese-writing agent — lives at
[HyperfocuSam/buzz-agent-packs](https://github.com/HyperfocuSam/buzz-agent-packs).
