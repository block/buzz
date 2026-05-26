# Persona Pack Specification

## 1. Overview & Goals

A **Persona Pack** is a portable, self-contained bundle that defines one or more AI agent personas
for deployment in Sprout. It is a **superset of the [Open Plugin Spec](https://open-plugin-spec.org)**
— every valid Persona Pack is also a valid OPS package, but not vice versa.

A pack contains: personas (identity + system prompt), skills (on-demand instruction sets), MCP
server config, pack-level instructions, lifecycle hooks, and distribution metadata.

### Design Goals

1. **Portable** — zip file or git repo; no Sprout tooling required to inspect
2. **Composable** — skills and MCP servers shared across agents; per-agent overrides additive
3. **OPS-compatible** — discoverable by any OPS-compatible tool
4. **Harness-honest** — explicit about what the agent runtime does vs. what sprout-acp does

---

## 2. Open Plugin Spec Compatibility

A Persona Pack is a valid OPS package. The `.plugin/plugin.json` manifest follows the OPS schema,
and Sprout-specific extensions live alongside the OPS fields at the top level. Since the Open
Plugin Spec defines no model configuration fields, there are no collisions. OPS consumers safely
ignore unknown fields.

### `.plugin/plugin.json`

```json
{
  "$schema": "https://open-plugin-spec.org/schema/v1/plugin.json",
  "id": "com.example.meadow-security-team",
  "name": "Meadow Security Team",
  "version": "1.2.0",
  "description": "A four-agent security review team for Sprout.",
  "author": "Meadow Engineering",
  "license": "MIT",
  "homepage": "https://github.com/example/meadow-security-team",
  "keywords": ["security", "code-review", "sprout"],
  "engines": {
    "sprout": ">=0.9.0"
  },
  "personas": [
    "agents/pip.persona.md",
    "agents/lep.persona.md",
    "agents/thistle.persona.md",
    "agents/berry.persona.md"
  ],
  "pack_instructions": "instructions.md",
  "mcp_config": ".mcp.json",
  "hooks_config": "hooks/hooks.json",
  "defaults": {
    "model": "anthropic:claude-sonnet-4-20250514",
    "temperature": 0.7,
    "max_context_tokens": 128000,
    "triggers": {
      "mentions": true,
      "keywords": [],
      "all_messages": false
    },
    "subscribe": [],
    "thread_replies": true,
    "broadcast_replies": false
  }
}
```

The `defaults` object sets pack-wide behavioral defaults for all personas. Any behavioral config
field that a persona does not explicitly set is resolved from this object. In the example above,
all four agents default to Sonnet — but `pip.persona.md` overrides with Opus:

```yaml
# agents/pip.persona.md (frontmatter excerpt)
model: "anthropic:claude-4-opus-20250514"
subscribe:
  - "#security-reviews"
```

pip gets Opus; lep, thistle, and berry get Sonnet. Temperature 0.7 applies to all four because
none of them override it.

> **Note**: `subscribe` and `triggers` in `defaults` are valid but unusual — most packs set
> these per-persona since agents typically monitor different channels and respond to different
> triggers.

### Compatibility Rules

- **OPS consumers**: see standard metadata; safely ignore unknown fields including `personas`,
  `defaults`, `pack_instructions`, `mcp_config`, and `hooks_config`.
- **Sprout**: reads both OPS fields and the Sprout-specific fields; `personas` is authoritative.
- **Version negotiation**: `engines.sprout` specifies minimum required Sprout version; sprout-acp
  rejects packs requiring a newer version.
- **Extension mechanism**: Sprout-specific fields sit at the top level of `plugin.json` alongside
  OPS fields. No OPS core field is overloaded.
- **`defaults`**: ignored entirely by OPS consumers. sprout-acp resolves it at deploy time before
  constructing per-persona configurations (see Section 10 and Section 12).

---

## 3. Pack Layout

```
my-pack/
├── .plugin/
│   └── plugin.json          # OPS manifest (superset)
├── agents/
│   ├── pip.persona.md        # Persona: identity + system prompt
│   ├── lep.persona.md
│   ├── thistle.persona.md
│   └── berry.persona.md
├── skills/                   # Pack skills (harness copies to .agents/skills/)
│   ├── code-review/
│   │   └── SKILL.md
│   ├── security-review/
│   │   └── SKILL.md
│   └── shared/
│       └── SKILL.md
├── .mcp.json                 # Pack-level MCP server config (shared)
├── hooks/
│   └── hooks.json            # Lifecycle hooks (harness-managed)
├── instructions.md           # Pack-level instructions (injected by harness)
├── pack.lock                 # Version lock (Phase 1+)
├── README.md                 # Human-readable description
└── my-pack-1.2.0.sproutpack.sha256  # Checksum (required for zip distribution)
```

### Directory Conventions

- `agents/` — all persona files. No nesting; flat directory.
- `skills/` — one subdirectory per skill. Each skill directory contains a `SKILL.md` file.
  Both `name:` and `description:` frontmatter fields are **required** — see Section 6.
- `.plugin/` — OPS-required location for the manifest.
- `hooks/` — optional; omit if no hooks are needed.
- `instructions.md` — optional; omit if no pack-level instructions.
- `.mcp.json` — optional; omit if no shared MCP servers.

Pack contents must not include: agent working directory state (`.agents/`, etc.),
secrets or API keys (use `${VAR_NAME}` references), or build artifacts.

---

## 4. Persona File Format (`.persona.md`)

A persona file is a markdown document with YAML frontmatter. The **YAML frontmatter** defines
identity, skills, MCP servers, and behavioral config. The **markdown body** (everything after the
closing `---`) is the agent's persona prompt text.

> **Note**: The persona prompt is currently delivered as a `[System]` prefix in the user message text (see Section 12). True system prompt injection (once at session creation rather than every turn) is planned — see Section 16.

### Full Schema

```yaml
---
# === Identity ===
name: "lep"
display_name: "Lep 🍀"
avatar: "./avatars/lep.png"
description: "Security-focused code reviewer"

# === Open Plugin Spec fields ===
version: "1.0.0"
author: "Meadow Team"

# === Skills ===
skills:
  - "./skills/security-review/"
  - "./skills/code-review/"

# === MCP Servers (per-persona) ===
mcp_servers:
  - name: "semgrep"
    command: "semgrep-mcp"
    args: ["--stdio"]
    env:
      SEMGREP_TOKEN: "${SEMGREP_TOKEN}"

# === Behavioral Config (Sprout-specific) ===
subscribe:
  - "#security-reviews"
  - "#code-reviews"
triggers:
  mentions: true
  keywords: ["security", "vulnerability", "CVE"]
model: "anthropic:claude-sonnet-4-20250514"
temperature: 0.3
max_context_tokens: 128000

# === Hooks (harness-managed) ===
hooks:
  on_start: "./hooks/setup-semgrep.sh"
  on_stop: "./hooks/cleanup.sh"
  on_message: null
---

You are Lep, a security-focused code reviewer on the Meadow team.
...
```

### Field Reference

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | ✅ | Machine name / agent ID. Lowercase, no spaces, unique within pack. |
| `display_name` | string | ✅ | Human-readable name shown in Sprout UI. |
| `avatar` | string | ❌ | Pack-relative path to avatar image. |
| `description` | string | ✅ | One-line description. |
| `version` | string | ❌ | Semver. Defaults to pack version if omitted. |
| `author` | string | ❌ | OPS compatibility field. |
| `skills` | string[] | ❌ | Pack-relative paths to skill directories for this agent only. |
| `mcp_servers` | object[] | ❌ | Per-persona MCP servers. Merged with pack-level `.mcp.json`. |
| `subscribe` | string[] | ❌ | Channels to monitor. See Section 10. |
| `triggers` | object | ❌ | Controls which messages activate a response. See Section 10. |
| `model` | string | ❌ | Model to use. See Section 10. |
| `temperature` | float | ❌ | Sampling temperature. See Section 10. |
| `max_context_tokens` | int | ❌ | Context window limit. See Section 10. |
| `thread_replies` | bool | ❌ | Reply in-thread when triggering message is in a thread. See Section 10. |
| `broadcast_replies` | bool | ❌ | Surface thread replies to the main channel. See Section 10. |
| `hooks` | object | ❌ | Lifecycle hooks. Harness-managed. See Section 9. |

> **Legacy alias**: The YAML key `respond_to` is accepted as an alias for `triggers` in persona frontmatter. In `plugin.json` defaults, both `triggers` and `respond_to` are accepted. The canonical key is `triggers`.

### Markdown Body (Persona Prompt)

Everything after the closing `---` is the persona prompt text. Pack-level `instructions.md` is
appended after it. Embed the prompt directly — do not reference external files or `.mdc` rule
files (agent runtimes typically do not read them).

---

## 5. Two-Layer Prompt Architecture

sprout-acp assembles the agent's context from two distinct prompt layers before sending each
message. Understanding this layering is essential for persona authors — content that belongs in
one layer should not be duplicated in the other.

### Prompt Section Order

Each message delivered to the agent runtime includes these sections in order:

```
[Base]
<platform orientation — injected by sprout-acp>

[System]
<persona prompt — markdown body of .persona.md>

---
# Team Instructions
<contents of instructions.md, if present>

[Context]
<scope, channel name, and contextual hints>

[Thread/Conversation Context]
<recent message history, if applicable>

[Sprout event]
<the triggering message or event>
```

### The `[Base]` Layer

The `[Base]` layer is compiled into sprout-acp and is **identical for every agent**. It covers:

| Content | Purpose |
|---------|---------|
| Platform identity | Tells the agent it is running inside Sprout and what that means |
| MCP tool reference | Documents the tools available via the connected MCP servers |
| Workspace layout | Describes `$AGENT_CWD`, skill discovery paths, and file conventions |
| Message polling | Explains how to check for new messages proactively |

Pack authors do not write or configure the `[Base]` layer — it is maintained by the Sprout team
and updated in sprout-acp releases.

**Disabling or customizing the base layer**: Set `SPROUT_ACP_NO_BASE_PROMPT` to omit the `[Base]`
section entirely. To replace the compiled-in default with custom content, set
`SPROUT_ACP_BASE_PROMPT_FILE` to a file path — sprout-acp reads it at startup and uses it instead.

### The `[System]` Layer

The `[System]` layer is the persona prompt — the markdown body of the `.persona.md` file. It is
**unique per agent** and defines the agent's role, identity, and behavioral rules. This is where
pack authors write their persona content.

What belongs in `[System]`:

| Content | Examples |
|---------|---------|
| Agent name and role | "You are Lep, a security-focused code reviewer" |
| Team protocols | Escalation rules, @-mention discipline, handoff conventions |
| Domain rules | Security checklists, review criteria, coding standards |
| Behavioral autonomy | When to act independently vs. when to ask |

### Guidance for Pack Authors

**Do not duplicate base layer content in persona prompts.** Users with the base layer enabled
(the default) would see that content twice per message. Specifically, do not re-explain:

- How to use MCP tools (covered by `[Base]`)
- How to poll for new messages or use the `since` parameter (covered by `[Base]`)
- Workspace layout or skill loading mechanics (covered by `[Base]`)
- That the agent is running inside Sprout (covered by `[Base]`)

Focus persona prompts on what makes this agent unique: its role, personality, domain expertise,
and team-specific protocols.

---

## 6. Skills

> **Implementation note**: Skill paths are stored as declared in persona frontmatter. Resolution
> to `SKILL.md` `name:` fields and runtime copying to `$AGENT_CWD/.agents/skills/` is planned
> for a future release.

Skills are reusable instruction sets that agents load on demand. They are markdown files that teach
the agent how to perform a specific task.

### Discovery

The agent runtime discovers skills from these directories relative to the session working directory
(`$AGENT_CWD` — see definition below):

> **Note**: `.agents/skills/` is sprout-acp's canonical skill location. The other paths shown
> (`.goose/skills/`, `.claude/skills/`) are agent-runtime-specific and listed for reference only.

```
$AGENT_CWD/.goose/skills/<skill-name>/SKILL.md
$AGENT_CWD/.claude/skills/<skill-name>/SKILL.md
$AGENT_CWD/.agents/skills/<skill-name>/SKILL.md   ← sprout-acp uses this one
```

> **Note**: `$AGENT_CWD/skills/` is NOT scanned. Skills placed at the pack root `skills/` directory
> are not discoverable by the agent runtime until the harness copies them.

### `$AGENT_CWD` Definition

Throughout this spec, **`$AGENT_CWD`** refers to the `cwd` field in the ACP `NewSessionRequest`
— the working directory passed to the agent runtime when creating a session. The value is delivered via the ACP protocol.

However, operators can control this value by setting the `AGENT_CWD` environment variable on the
sprout-acp process. sprout-acp determines what value to pass as `NewSessionRequest.cwd` in this
order:

1. The `AGENT_CWD` environment variable on the sprout-acp process, if set.
2. `std::env::current_dir()` as a fallback.
3. If both fail, sprout-acp logs an error and **refuses to start**.

The agent runtime stores this value as `session.working_dir` and uses it for all skill discovery.

### Skill Name Resolution (Load Key)

The load key used in `load(source: "skill-name")` is the `name:` field from `SKILL.md` frontmatter.

**Both `name:` and `description:` are required fields in `SKILL.md` frontmatter.** The skill
metadata schema is:

```rust
#[derive(Debug, Deserialize)]
struct SkillMetadata {
    name: String,
    description: String,
}
```

If either field is absent or the frontmatter is malformed, `parse_frontmatter` returns `None` and
the skill is **silently skipped**. There is **no fallback to the directory name**.

> **Recommendation**: Use the directory name as the `name:` value for consistency (e.g., a skill
> in `skills/security-review/` should have `name: "security-review"`). This avoids load key
> mismatches and makes `load(source: "security-review")` predictable.

### Skill Scoping Rules

Skills in the pack's `skills/` directory are copied to agent working directories according to these rules:

| Condition | Destination |
|-----------|-------------|
| Skill directory is listed in **at least one** persona's `skills:` array | Copied **only** to that persona's `$AGENT_CWD/.agents/skills/` |
| Skill directory is **not listed in any** persona's `skills:` array | Copied to **all** agents' `$AGENT_CWD/.agents/skills/` |

**Key implication**: Once a skill is claimed by any persona, it is no longer automatically shared
with other agents. If you want a skill available to all agents AND explicitly listed in one persona's
`skills:` array, list it in every persona's `skills:` array.

### Collision Handling

If a skill with the same load key already exists in `$AGENT_CWD/.agents/skills/`, the pack skill
is **not overwritten**. This allows operators to pin custom skill versions. sprout-acp **must log
a warning** when a pack skill is skipped due to a collision:

```
WARN: Skill "security-review" already exists at .agents/skills/security-review/; skipping pack version
```

### Loading

Skills are not auto-loaded into context. The agent must explicitly load them:

```
load(source: "security-review")
```

sprout-acp lists available skills in the user message prefix so the agent knows what's available.
See Section 12 for the full message format.

### Skill File Format

```markdown
---
name: "security-review"
description: "Reviews code for security vulnerabilities using OWASP Top 10 and semgrep"
---

# Security Review

...content...
```

Both `name:` and `description:` are **required**. A skill missing either field is silently skipped
by the agent runtime. `sprout pack validate` warns on skill name mismatches but does not yet
enforce required metadata fields (see PF-5).

---

## 7. MCP Server Configuration

MCP servers provide external tool access (GitHub, Semgrep, databases, etc.). Configuration is
defined at two levels: pack-level (shared across all agents) and per-persona (agent-specific).
sprout-acp merges them and passes the result via the ACP protocol — no filesystem placement required.

> **Transport Warning**: Only `stdio` and `streamable_http` transports are supported. SSE transport
> is rejected by the ACP runtime with the error `"SSE is unsupported, migrate to streamable_http"` and
> will cause session startup to fail. Migrate any SSE-based MCP servers to streamable_http before
> packaging.

### Pack-Level: `.mcp.json`

```json
{
  "mcpServers": {
    "github": {
      "command": "github-mcp-server",
      "args": ["stdio"],
      "env": {
        "GITHUB_PERSONAL_ACCESS_TOKEN": "${GITHUB_TOKEN}"
      }
    },
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/workspace"]
    }
  }
}
```

### Per-Persona: `mcp_servers` in Frontmatter

```yaml
mcp_servers:
  - name: "semgrep"
    command: "semgrep-mcp"
    args: ["--stdio"]
    env:
      SEMGREP_TOKEN: "${SEMGREP_TOKEN}"
```

### Merge Rules

1. Pack-level servers are the base set; per-persona servers merged on top.
2. **Name collision**: per-persona entry wins entirely (no partial merge).
3. The merged set is passed to the agent runtime via `NewSessionRequest.mcp_servers`.

### Environment Variable Interpolation

> **Implementation note**: MCP env var interpolation (`${VAR_NAME}` resolution) is planned but not
> yet implemented. In the current release, `${VAR_NAME}` strings are passed through as literals
> to the agent runtime, which may resolve them via its own MCP server configuration handling.

When implemented, all `env` values will be scanned for `${VAR_NAME}`. sprout-acp will resolve
from the process environment **before** passing to the agent runtime. Unresolved variables will
cause a startup error.

### Delivery

sprout-acp passes the merged config via `NewSessionRequest.mcp_servers`. **No `.mcp.json` is written to the agent's working directory.**

---

## 8. Pack-Level Instructions

`instructions.md` contains shared rules, coding standards, and team norms that apply to all agents
in the pack. sprout-acp appends it to the persona prompt in the user message prefix.

sprout-acp appends `instructions.md` to the persona prompt in the user message prefix (see
Section 12). **No file is written to disk.**

**What does NOT work**: `.mdc` rule files (agent runtimes typically don't read them), `rules/` directory (no
`--rules-dir` flag), relying on the pack's `AGENTS.md` for runtime injection (it's for human
contributors only).

> **Note**: Some agent runtimes auto-load `AGENTS.md` and `.goosehints` from `$AGENT_CWD` (walking up to git
> root). Operators can place instructions there as a secondary mechanism, but the canonical path
> is harness injection via the user message prefix.

---

## 9. Lifecycle Hooks

> **Implementation note**: Hooks are parsed and validated at pack load time but not yet executed.
> Hook execution is planned for a future release.

Hooks are shell commands fired by sprout-acp at agent lifecycle points. **Agent runtimes typically have no hook system**
— hooks are entirely a harness feature.

### `hooks/hooks.json`

Pack-level hooks apply to all agents:

```json
{
  "on_start": "./hooks/setup.sh",
  "on_stop": "./hooks/cleanup.sh",
  "on_message": null
}
```

### Per-Persona Hooks

Per-persona hooks override pack-level hooks for that agent:

```yaml
hooks:
  on_start: "./hooks/setup-semgrep.sh"
  on_stop: "./hooks/cleanup.sh"
  on_message: null
```

### Hook Points

| Hook | When Fired | Use Cases |
|------|-----------|-----------|
| `on_start` | Before the agent session starts | Install dependencies, warm caches, validate credentials |
| `on_stop` | After the agent session ends (normal exit or error) | Cleanup temp files, flush logs, release locks |
| `on_message` | Before each message is dispatched to the agent | Rate limiting, logging, message preprocessing |

### Hook Execution

Hooks run as the sprout-acp user; working directory is `$AGENT_CWD`; agent env vars are available.
Exit codes: `on_start` non-zero → abort startup; `on_stop` non-zero → logged only; `on_message`
non-zero → message dropped and error logged.

### `on_message` Hook Contract

The `on_message` hook receives the incoming message content via **stdin** (UTF-8 text). It is a
**read-only side-effect hook** — it cannot modify the message. If you need message transformation,
that must be implemented directly in sprout-acp's dispatch loop, not via a hook.

- **Timeout**: 5 seconds. Hooks that exceed this are killed (SIGKILL) and the message is dropped.
- **Non-zero exit**: Message is dropped and an error is logged. The agent does not see the message.
- **Stdout/stderr**: Captured and logged at DEBUG level. Not passed to the agent.

### `on_stop` Crash Caveat

`on_stop` fires on normal exit and on handled errors. It **will not fire** if sprout-acp crashes
(SIGSEGV, OOM, etc.). For critical cleanup (lock files, external resource release), use a
systemd/supervisor cleanup unit or a process supervisor that runs cleanup unconditionally.

**Hooks are NOT agent runtime features.** They are implemented entirely in sprout-acp. Bypassing
sprout-acp means no hooks fire.

---

## 10. Behavioral Configuration

The behavioral config fields in a persona's frontmatter control how the agent participates in
Sprout conversations. These are all Sprout-specific — the agent runtime has no awareness of them. They sit
at the top level of the frontmatter alongside identity fields like `name` and `description`.

### Pack Defaults

Teams of four or more agents often share the same model, temperature, and response settings. The
`defaults` object in `plugin.json` sets pack-wide values for all behavioral config fields.
Per-persona frontmatter fields override them.

If `plugin.json` does not contain a `defaults` key, level 4 is skipped entirely and fields fall
through directly to built-in defaults (level 5).

**Example**: A four-agent security team where all agents use Sonnet except the orchestrator (pip),
which uses Opus.

`plugin.json`:
```json
{
  "personas": [
    "agents/pip.persona.md",
    "agents/lep.persona.md",
    "agents/thistle.persona.md",
    "agents/berry.persona.md"
  ],
  "defaults": {
    "model": "anthropic:claude-sonnet-4-20250514",
    "temperature": 0.7,
    "max_context_tokens": 128000,
    "triggers": {
      "mentions": true,
      "keywords": [],
      "all_messages": false
    },
    "subscribe": [],
    "thread_replies": true,
    "broadcast_replies": false
  }
}
```

> **Note**: `subscribe` and `triggers` in `defaults` are valid but unusual — most packs set
> these per-persona since agents typically monitor different channels and respond to different
> triggers.

`agents/pip.persona.md` (frontmatter excerpt):
```yaml
model: "anthropic:claude-4-opus-20250514"
subscribe:
  - "#security-reviews"
```

Result:
- **pip**: model=Opus, temperature=0.7 (from pack default), max_context_tokens=128000 (from pack default)
- **lep, thistle, berry**: model=Sonnet, temperature=0.7, max_context_tokens=128000 (all from pack default)

### Precedence Model

In this spec, **"deploy time"** means when sprout-acp loads the pack and constructs per-persona
session configurations — typically at sprout-acp process startup. For git-based packs, this occurs
each time sprout-acp starts and reads the installed pack directory.

When sprout-acp resolves the effective configuration for a persona, it applies this order (highest
wins):

```
1. Operator env vars           — e.g. GOOSE_MODEL, GOOSE_PROVIDER (agent-runtime-specific)
                                 already set in the parent process environment
2. Desktop UI per-agent        — overrides set in the Sprout desktop app per-agent settings
3. Per-persona frontmatter     — behavioral config fields set directly in the persona's frontmatter
4. Pack-level defaults         — the `defaults` object in plugin.json
5. Built-in defaults           — sprout-acp's hardcoded fallback values
```

sprout-acp resolves levels 3–5 at deploy time (when the pack is loaded and sessions are
constructed). Levels 1–2 are applied at runtime and are outside the pack's control.

**Level 1 — Operator env vars**: If the operator has already set env vars for model, provider, temperature, or context limit in the parent process environment, sprout-acp MUST NOT override them with pack/persona values. sprout-acp only injects env vars for fields that are NOT already set in the parent environment. This ensures operators can always override pack configuration.

**Implementation**: when constructing the child process environment, sprout-acp checks
`std::env::var(key)` for each env var. If the parent already has it set, skip injection. If not,
inject the resolved pack/persona value.

### Empty and `null` Semantics

The following rules govern how absent, empty, and null values are interpreted in a persona's
behavioral config frontmatter fields:

- **All behavioral config fields absent** (no `model`, `temperature`, `subscribe`, etc.) is
  equivalent to having no overrides — all pack defaults apply.

- **`temperature: null`** — `null` values are treated as absent. The field falls through to the
  next precedence level (pack default, then built-in default). This allows a persona to explicitly
  "unset" a field it previously set.

- **`subscribe: []`** — an empty array is NOT treated as absent. It means "subscribe to nothing."
  This is an intentional override that prevents pack defaults from applying.

- **`triggers: {}`** — an empty object is NOT treated as absent. It means "use default sub-field
  values." This overrides the pack default `triggers` object entirely, and each sub-field falls
  through to its built-in default.

> **Rule of thumb**: `null` = absent (fall through). Empty containers (`[]`, `{}`) = present
> (override).

### Merge Semantics

Field merging is **shallow replacement** — there is no deep merge. The rules are:

- **Simple fields** (`model`, `temperature`, `max_context_tokens`, `thread_replies`,
  `broadcast_replies`): the first defined value in the precedence chain wins entirely.
- **Object fields** (`triggers`): if the persona sets `triggers`, the entire object replaces
  the pack default. Individual sub-keys are not merged. If the persona does not set `triggers`,
  the pack default `triggers` object is used as-is.
- **Array fields** (`subscribe`): if the persona sets `subscribe`, the entire array replaces the
  pack default. There is no union or append behavior. If the persona does not set `subscribe`, the
  pack default `subscribe` array is used as-is.

**Example — object replacement**:

Pack default (`defaults` in `plugin.json`):
```json
"triggers": { "mentions": true, "keywords": ["security"], "all_messages": false }
```

Persona override (frontmatter):
```yaml
triggers:
  mentions: true
  all_messages: true
```

Effective result for that persona:
```json
{ "mentions": true, "all_messages": true }
```

Note: `keywords` is **gone** — the persona's `triggers` replaced the entire object. There is no
implicit inheritance of sub-keys.

**Example — array replacement**:

Pack default (`defaults` in `plugin.json`):
```json
"subscribe": ["#general"]
```

Persona override (frontmatter):
```yaml
subscribe:
  - "#security-reviews"
  - "#code-reviews"
```

Effective result: `["#security-reviews", "#code-reviews"]` — `#general` is not included.

**Example — empty object override**:

Pack default (`defaults` in `plugin.json`):
```json
"triggers": { "mentions": true, "keywords": ["security"], "all_messages": false }
```

Persona override (frontmatter):
```yaml
triggers: {}
```

Effective result for that persona:
```json
{ "mentions": true, "keywords": [], "all_messages": false }
```

Note: `triggers: {}` is NOT absent — it overrides the pack default entirely. Each sub-field
falls through to its **built-in default** (not the pack default). `mentions` defaults to `true`,
`keywords` to `[]`, `all_messages` to `false`.

### Canonical Behavioral Config Field Schema

This schema applies identically to both the `defaults` object in `plugin.json` and the top-level
behavioral config fields in `.persona.md` frontmatter. The same keys, types, and validation rules
apply to both.

| Field | Type | Built-in Default | Valid Range / Values | Description |
|-------|------|-----------------|----------------------|-------------|
| `subscribe` | string[] | `[]` | Any channel name strings | Channels to monitor. `#` prefix stripped before relay calls. |
| `triggers` | object | see sub-fields | — | Controls which messages activate a response. Replaced as a whole unit on override. |
| `triggers.mentions` | bool | `true` | `true` / `false` | Respond when @mentioned. |
| `triggers.keywords` | string[] | `[]` | Any strings | Respond when message contains any keyword (case-insensitive). |
| `triggers.all_messages` | bool | `false` | `true` / `false` | Respond to every message in subscribed channels. |
| `model` | string | none (agent runtime uses operator default) | `"provider:model-id"` format | Model to use. Split on first `:` for provider + model env vars. |
| `temperature` | float | `0.7` | Provider-dependent (typically 0.0–2.0). sprout-acp passes through without range validation; `sprout pack validate` checks type only (must be a number), not range. | Passed as env var to agent runtime. |
| `max_context_tokens` | int | none (provider default) | Positive integer | Passed as env var to agent runtime. |
| `thread_replies` | bool | `true` | `true` / `false` | Reply in-thread when the triggering message is in a thread. |
| `broadcast_replies` | bool | `false` | `true` / `false` | Also surface thread replies to the main channel. |

**Unknown keys** in `defaults` (in `plugin.json`) are **validation warnings** in `sprout pack
validate` — this catches typos like `temprature` at validate time. Unknown keys in persona
frontmatter are **hard errors** (via `deny_unknown_fields` in the YAML parser). At deploy time,
sprout-acp logs a `WARN` and ignores unknown manifest keys, remaining fail-soft:

```
WARN: Unknown key "temprature" in defaults (plugin.json); ignoring
```

### Full Behavioral Config Reference

```yaml
# In a .persona.md frontmatter — behavioral config fields at top level:

subscribe:
  - "#security-reviews"
  - "#code-reviews"

triggers:
  mentions: true
  keywords:
    - "security"
    - "vulnerability"
    - "CVE"
  all_messages: false

model: "anthropic:claude-sonnet-4-20250514"
temperature: 0.3
max_context_tokens: 128000

thread_replies: true
broadcast_replies: false
```

### Channel Name `#` Convention

The `#` prefix in `subscribe` entries is a **display convention only**. Channel names in the Sprout
relay are stored and queried **without** the `#` prefix. sprout-acp strips the leading `#` before
making any relay API calls. `"#security-reviews"` and `"security-reviews"` are equivalent in this
field.

### Env Var Mapping

sprout-acp resolves pack defaults and per-persona overrides (precedence levels 3–5) into a single
effective configuration per persona **before** injecting environment variables into the child
process. The env vars set reflect the fully-resolved values — not the raw persona frontmatter.

sprout-acp translates persona behavioral config fields to agent configuration via environment
variables injected into the child process at spawn time:

| Persona field | Env var(s) | Notes |
|---|---|---|
| `model: "anthropic:claude-sonnet-4-20250514"` | `GOOSE_PROVIDER=anthropic` + `GOOSE_MODEL=claude-sonnet-4-20250514` | Split on first `:` |
| `temperature: 0.3` | `GOOSE_TEMPERATURE=0.3` | Read by agent runtime at startup |
| `max_context_tokens: 128000` | `GOOSE_CONTEXT_LIMIT=128000` | Read by agent runtime at startup |

If `model` is omitted from both the persona frontmatter and `defaults`, sprout-acp does not set
`GOOSE_PROVIDER` or `GOOSE_MODEL`, and the agent runtime uses its configured operator default.

> **Implementation note**: `AcpClient::spawn` accepts per-persona env vars via the `extra_env` parameter. sprout-acp checks `std::env::var(key)` before injecting each var — if the parent environment already has the key set, injection is skipped (operator precedence, level 1).

See the Canonical Behavioral Config Field Schema table above for the full field reference.

> **Built-in defaults note**: The "Built-in Default" column in the Canonical Behavioral Config
> Field Schema table lists sprout-acp's built-in fallbacks (precedence level 5). If `defaults` is
> present in `plugin.json`, those values take precedence over the built-in defaults (level 4 >
> level 5). The built-in defaults only apply when neither the persona nor the pack defaults specify
> a value.

All fields are consumed entirely by sprout-acp. None are passed to the agent runtime directly — they are projected as env vars or used by the harness's subscription/dispatch logic.

---

## 11. Distribution

### Phase 1: Zip File

A pack is distributed as a `.sproutpack` file (zip archive):

```bash
sprout pack validate ./my-pack
sprout pack ./my-pack --output my-pack-1.2.0.sproutpack
sprout install ./my-pack-1.2.0.sproutpack
sprout install https://example.com/releases/my-pack-1.2.0.sproutpack
```

#### Pack Integrity (Required)

Zip packs **must** ship with `<pack-name>-<version>.sproutpack.sha256` containing `sha256sum`
output (`<hex-digest>  <filename>`). sprout-acp **must** verify before installation and refuse on
mismatch. For HTTP installs, the checksum file is fetched from the same base URL.

#### `pack.lock` for Phase 1

Phase 1 installs record the installed pack in `pack.lock` alongside the pack directory:

```json
{
  "com.example.meadow-security-team": {
    "source": "https://example.com/releases/my-pack-1.2.0.sproutpack",
    "sha256": "a3f1c2d4e5b6...",
    "version": "1.2.0",
    "installed_at": "2026-04-10T11:00:00Z"
  }
}
```

### Phase 2: Git Repository

```bash
sprout install github:example/meadow-security-team
sprout install github:example/meadow-security-team@v1.2.0
sprout install git+https://gitlab.example.com/team/pack.git
```

`pack.lock` for git installs records the resolved commit SHA:

```json
{
  "com.example.meadow-security-team": {
    "source": "github:example/meadow-security-team",
    "resolved": "github:example/meadow-security-team#abc1234",
    "version": "1.2.0",
    "installed_at": "2026-04-10T11:00:00Z"
  }
}
```

### Phase 3: App Store UI

A Sprout-hosted registry and in-app browser for discovering and installing packs. API-compatible
with OPS registries. Details TBD.

### Installed Pack Location

Installed packs live at `~/.sprout/packs/<pack-id>/`. sprout-acp reads packs from this location
at agent startup.

### Desktop App Import

The Sprout desktop app can import persona packs via the Import button:

- **My Agents → Import**: Accepts `.persona.md` files (individual personas) or `.zip` files
  (persona packs detected by `.plugin/plugin.json`). Pack zips are resolved in a temp directory;
  each persona is previewed and imported individually into the persona library.
- **My Teams → Import**: Accepts `.zip` files (persona packs). The pack name becomes the team
  name; each persona becomes a team member.

> **Note**: The Import button parses and previews personas from the pack — it does not install the
> pack directory itself. For full pack installation (which copies the pack to
> `<AppDataDir>/agents/packs/<pack-id>/` with re-validation), use the `install_persona_pack`
> Tauri command or a future "Install Pack" UI button.

---

## 12. Delivery Mechanism Summary

How each pack component reaches the running agent:

| Component | Delivery Method | Mechanism | Filesystem Write? |
|-----------|----------------|-----------|-------------------|
| Skills | Copy at deploy time (planned) | sprout-acp will copy `skills/` → `$AGENT_CWD/.agents/skills/` | ✅ Yes (only one) |
| MCP servers | ACP protocol | `NewSessionRequest.mcp_servers` | ❌ No |
| Persona prompt | User message prefix | `[System]` block prepended to user message text by sprout-acp | ❌ No |
| Pack instructions | User message prefix | Appended to `[System]` block in user message text | ❌ No |
| Lifecycle hooks | Harness internal | sprout-acp fires shell commands directly | ❌ No |
| Model/provider | Child process env vars | Agent-runtime-specific env vars (e.g. `GOOSE_PROVIDER`, `GOOSE_MODEL`) | ❌ No |
| Behavioral config | Harness internal | sprout-acp subscription + dispatch logic | ❌ No |
| Pack defaults (`defaults`) | Harness internal | Resolved at deploy time by sprout-acp into per-persona effective config; never passed to the agent runtime directly | ❌ No |

> **Pack defaults are resolved at deploy time**, not at runtime. When sprout-acp loads a pack and
> constructs per-persona session configurations, it merges the `defaults` object with each persona's
> frontmatter behavioral config fields (per the precedence model in Section 10) and stores the
> resulting effective configuration. The `defaults` object itself is not forwarded to the agent runtime or
> stored in any runtime artifact — only the resolved per-persona values are used.

### The `[System]` Block — Current Implementation

sprout-acp's `format_prompt()` in `queue.rs` prepends a `[System]` block to the **user message
text** before sending it to the agent runtime. This is a **sprout-acp feature, not an agent runtime
feature**. The agent sees the `[System]` prefix as part of the user message content — it is NOT
injected into the agent's actual system prompt.

For persona-backed agents, the `[System]` block contains:

```
[System]
<persona prompt (markdown body of .persona.md)>

---
# Team Instructions
<contents of instructions.md, if present>

---
Available skills: code-review, security-review, shared
Load a skill with: load(source: "skill-name")
```

### True System Prompt Injection — Planned

The `[System]` prefix re-sends the full persona prompt on every turn. True system prompt injection
— calling `agent.extend_system_prompt()` after `create_agent_for_session()` in `on_new_session()`
— fires once at session creation. This is planned work; see Section 16.

### What Does NOT Work (Anti-Pattern Reference)

| Anti-Pattern | Why It Fails |
|-------------|-------------|
| `goose acp --skill-path ./skills` | `--skill-path` flag does not exist in goose |
| `goose acp --rules-dir ./rules` | `--rules-dir` flag does not exist in goose |
| `goose acp --system-prompt-file ./prompt.md` | Flag does not exist in goose-acp |
| `rules/*.mdc` files | Agent runtimes typically don't read `.mdc` files |
| `skills/` at pack root (without copying) | Agent runtimes scan `.agents/skills/`, not `skills/` |
| Hooks in goose config | Agent runtimes have no hook system; hooks are a harness feature |
| SSE transport in `.mcp.json` | ACP runtime rejects SSE; use stdio or streamable_http |
| SKILL.md without `name:` or `description:` | Skill silently skipped; no fallback |
| Setting `GOOSE_MODEL` on parent process (multi-persona) | Affects all agents; use per-subprocess injection via `extra_env` |
| Expecting `defaults` sub-key inheritance | No deep merge; object/array fields replaced entirely |

---

## 13. Security Considerations

### Secret Management

Never embed secrets in pack files. Use `${VAR_NAME}` references in all `env` blocks. Currently,
`${VAR_NAME}` strings are passed through as literals to the agent runtime (see Section 7). When
harness-side interpolation is implemented, sprout-acp will resolve them from the process
environment at startup and refuse to start if any are unresolved. Inject secrets via your
deployment mechanism (systemd env files, Vault, Kubernetes secrets, etc.).

### Pack Integrity

- **Phase 1 (zip)**: Packs **must** ship with `<pack-name>-<version>.sproutpack.sha256` containing
  `sha256sum` output (`<hex-digest>  <filename>`). sprout-acp **must** verify before installation
  and refuse on mismatch.
- **Phase 2 (git)**: `pack.lock` pins the resolved commit SHA; sprout-acp verifies on install.
- **Phase 3 (registry)**: Registry signatures TBD.

### Hook Execution

Hooks run with sprout-acp's privileges — significant attack surface. Only install packs from
trusted sources. Review all hook commands before installing. Consider sandboxing sprout-acp
(container, restricted user) for untrusted packs. sprout-acp should display hook commands before
first execution (Phase 2 feature).

### MCP Server and Skill Trust

MCP servers are external processes with tool access; audit all configs before deploying. Skills
are markdown injected into agent context; malicious content can attempt prompt injection. Treat
both with the same caution as any untrusted prompt content.

---

## 14. Migration Path

### From V6 (sprout-namespaced) Format

Field mapping from V6 `.persona.md` to current `.persona.md`:

| V6 location | Current location |
|---|---|
| `sprout.model` | `model` (top-level frontmatter) |
| `sprout.temperature` | `temperature` (top-level frontmatter) |
| `sprout.max_context_tokens` | `max_context_tokens` (top-level frontmatter) |
| `sprout.subscribe` | `subscribe` (top-level frontmatter) |
| `sprout.respond_to` | `triggers` (top-level frontmatter) |
| `sprout.thread_replies` | `thread_replies` (top-level frontmatter) |
| `sprout.broadcast_replies` | `broadcast_replies` (top-level frontmatter) |
| `plugin.json` → `sprout.defaults` | `plugin.json` → `defaults` (top-level) |
| `plugin.json` → `sprout.personas` | `plugin.json` → `personas` (top-level) |
| `plugin.json` → `sprout.pack_instructions` | `plugin.json` → `pack_instructions` (top-level) |
| `plugin.json` → `sprout.mcp_config` | `plugin.json` → `mcp_config` (top-level) |
| `plugin.json` → `sprout.hooks_config` | `plugin.json` → `hooks_config` (top-level) |

**V6 persona frontmatter** (before):
```yaml
sprout:
  model: "anthropic:claude-sonnet-4-20250514"
  temperature: 0.3
  subscribe:
    - "#security-reviews"
```

**Current persona frontmatter** (after):
```yaml
model: "anthropic:claude-sonnet-4-20250514"
temperature: 0.3
subscribe:
  - "#security-reviews"
```

**V6 `plugin.json`** (before):
```json
"sprout": {
  "personas": ["agents/pip.persona.md"],
  "defaults": { "model": "anthropic:claude-sonnet-4-20250514" }
}
```

**Current `plugin.json`** (after):
```json
"personas": ["agents/pip.persona.md"],
"defaults": { "model": "anthropic:claude-sonnet-4-20250514" }
```

### From Pre-V6 JSON Persona Format

Field mapping from flat JSON (`personas/lep.json`) to `.persona.md`:

| JSON field | `.persona.md` location |
|---|---|
| `system_prompt` | Markdown body (after closing `---`) |
| `model` | `model` (top-level frontmatter) |
| `channels` | `subscribe` (top-level frontmatter) |
| `mcp_servers` | Frontmatter `mcp_servers:` or pack-level `.mcp.json` |
| All other fields | Frontmatter (same names) |

### Migration Steps

1. Create pack directory with `.plugin/plugin.json`
2. For each persona JSON → create `agents/<name>.persona.md` using the mapping above
3. Move skills to `skills/<skill-name>/SKILL.md`; ensure each has `name:` and `description:` frontmatter
4. Create `instructions.md` from any shared prompt content
5. Run `sprout pack validate ./my-pack`

### Backward Compatibility

The V6 namespaced `sprout:` block format is not supported. Only the current flat top-level fields format is accepted. The `respond_to` key is accepted as a legacy alias for `triggers` in both persona frontmatter and `plugin.json` defaults.

---

## 15. Open Questions / Future Work

### Unresolved

1. **`session/set_model` as env var alternative**: The ACP runtime implements `on_set_model()` (ACP
   unstable feature). sprout-acp could call `session/set_model` after `session/new` to set the
   model per-session without env var injection. This avoids the `AcpClient::spawn` limitation for
   model (but not provider, temperature, or context limit). Deferred pending stability of the ACP
   unstable feature.

2. **`CONTEXT_FILE_NAMES` env var**: The goose agent runtime supports this env var to control which filenames are
   scanned for hints. Should sprout-acp set this to include pack-specific filenames? Deferred
   pending use case.

3. **Skill versioning**: Skills are identified by load key only. If two packs provide a skill with
   the same name, the no-overwrite rule means the first-installed wins silently. A versioned skill
   format (e.g., `code-review@1.2.0`) would resolve this.

4. **Pack signing**: Phase 3 registry needs a signing scheme. Ed25519 keypairs tied to pack author
   identity is the likely approach, but not yet designed.

5. **Multi-pack conflicts**: What happens when two installed packs define agents that subscribe to
   the same channel with overlapping `triggers` rules? Need a conflict resolution policy.

### Future Work

`sprout pack init` scaffolding; hot reload of skills/instructions; skill marketplace; pack dependencies; agent-to-agent handoff within a pack.

---

## 16. Planned Features

Features required by this spec but not yet implemented.

| ID | What | Where |
|----|------|-------|
| PF-1 | True system prompt injection via the ACP protocol's `on_new_session()`. Current `[System]` prefix re-sends persona prompt on every turn; true injection fires once at session creation. | ACP server `on_new_session()` |
| PF-2 | `sprout pack validate` CLI: **Implemented.** Schema-validates `plugin.json`; checks `.persona.md` required identity fields; validates behavioral config fields; warns on unknown keys and skill name mismatches. Remaining: verify `skills:` and `hooks:` paths exist; error on `SKILL.md` missing `name:` or `description:`. | `sprout-cli` / `sprout-admin` |
| PF-3 | Skill collision warning: emit `WARN` when a pack skill is skipped because a skill with the same load key already exists in `.agents/skills/`. | sprout-acp skill copy logic |
| PF-4 | `$AGENT_CWD` resolution: determine `NewSessionRequest.cwd` from (1) `AGENT_CWD` env var, (2) `std::env::current_dir()`, (3) error and refuse to start. | sprout-acp startup / session init |
| PF-5 | Skill parse failure warning: emit `WARN` when `parse_skill_content` returns `None` (missing `name:`, missing `description:`, or malformed frontmatter). Currently the agent runtime silently skips. sprout-acp should pre-validate during skill copy. | sprout-acp skill copy logic |
| PF-6 | Per-subprocess env var injection: **Implemented.** `AcpClient::spawn` accepts `extra_env: &[(String, String)]` injected via `Command::env()`. sprout-acp checks `std::env::var(key)` before injecting — operator env vars take precedence (level 1). | `sprout-acp/src/acp.rs` `AcpClient::spawn()` |

---

*End of Persona Pack Specification*
