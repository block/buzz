# Meadow Core

A minimal three-agent persona pack for Buzz.

| Agent | Role |
|-------|------|
| **Skip** | Orchestrator — coordinates the team, delegates work |
| **Lev** | Security reviewer — threat models, auth, injection |
| **Bana** | Architecture reviewer — big picture, simplicity |

## Usage

```bash
# Validate the pack
buzz pack validate ./examples/meadow-core

# Inspect resolved config
buzz pack inspect ./examples/meadow-core

# Import into the desktop app
# Use the "Install Pack" button and point to this directory
```

## Structure

```
meadow-core/
├── .plugin/
│   └── plugin.json          # Pack manifest (OPS-compatible)
├── agents/
│   ├── skip.persona.md       # Orchestrator
│   ├── lev.persona.md        # Security reviewer
│   └── bana.persona.md       # Architecture reviewer
├── skills/
│   └── github-research/
│       └── SKILL.md          # GitHub search skill (shared)
├── instructions.md           # Team-wide instructions
└── README.md
```

## Customizing

Edit any `.persona.md` file to change the agent's behavior. The YAML
frontmatter controls config (model, triggers, channels). The markdown
body is the system prompt.

See `crates/buzz-persona/PERSONA_PACK_SPEC.md` for the full format reference.
