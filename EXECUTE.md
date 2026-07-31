# Execute prompt — live per-channel MCP control

Paste the block below into a fresh agent session (Buzz channel agent, or Claude Code started in `~/Dev/buzz`).

**Branch note:** this prompt puts the agent on its own branch so it can run alongside/against another implementation without collisions. Change the branch name in the block if you want it somewhere else.

## If you're running this through Buzz — read this first

Three facts about this machine's Buzz setup, verified 2026-07-31:

1. **Every Buzz agent has `cwd = /Users/neeyafit/.buzz`.** The harness passes
   `std::env::current_dir()` as the ACP session cwd (`crates/buzz-acp/src/lib.rs:1546`) and there is
   no per-agent working-directory setting in `managed-agents.json`. The agent is *not* in the repo
   and will not auto-load the repo's `AGENTS.md`. The prompt below handles this — don't remove the
   `cd` instruction.
2. **`turn_timeout_seconds: 320`** (~5 min/turn). Do not ask for all 8 tasks in one message. Drive
   it **one task per turn**; a cold `cargo build -p buzz-acp` can consume most of a turn on its own.
3. **Use a fresh channel or DM, and the Opus-5 agent.** Buzz seeds a new session with a small window
   of recent channel history, so starting in a busy channel feeds unrelated chatter into the session.
   Opus-5 is `opus[1m]` — the 1M context matters for a ~1400-line plan across 8 tasks. Avoid
   Grok/Cursor here (different runtime).

---

We're implementing a feature for contribution back to `block/buzz` (upstream). The repo is at
`~/Dev/buzz`. Remotes: `origin` = Monivancan/buzz (my fork), `upstream` = block/buzz.

**First, three things to read, in this order:**

1. `docs/superpowers/plans/2026-07-31-live-mcp-control-buzz-acp.md` — the approved implementation
   plan. This is your instruction set: 8 tasks, each with exact files, real code, and a TDD step
   sequence. Follow it task by task, in order.
2. `docs/superpowers/specs/2026-07-31-live-mcp-control-design.md` — the design the plan came from.
   Read it for the *why*; the plan wins wherever they differ (the plan was written after
   re-verifying the spec against shipped source and records three corrections).
3. `AGENTS.md` — repo conventions and quality gates.

**The feature, in one line:** let a user grant or revoke an MCP server on a live Buzz channel and
have the agent continue the *same* conversation with the new tool set — via `session/resume` with
an unchanged sessionId and a changed `mcpServers` list. Plus multi-server config at spawn, and
opt-in `strictMcpConfig` to stop injecting ~64k tokens/turn of unused global tool schemas.

**Your working directory is NOT the repo.** You start in `/Users/neeyafit/.buzz`. The repo is at
`/Users/neeyafit/Dev/buzz`. Shell cwd does not persist between tool calls, so chain `cd` into every
command (`cd /Users/neeyafit/Dev/buzz && ...`) and use absolute paths for file reads and edits.

**Set up first:**

```bash
cd /Users/neeyafit/Dev/buzz && . ./bin/activate-hermit && \
  git status --short && \
  git checkout feat/live-mcp-control && \
  git checkout -b feat/live-mcp-control-impl
```

The working tree must be clean apart from `KICKOFF.md` / `EXECUTE.md`. If it isn't, stop and report
what's there rather than committing someone else's work.

`. ./bin/activate-hermit` is required before **every** git, cargo, or just command — the repo's
hooks and toolchain live in Hermit, and an unconfigured `PATH` makes them fail in confusing ways.
Do not rewrite hook commands to work around a broken PATH; fix the PATH.

Work directly in `~/Dev/buzz` on the branch above — **not** in a `git worktree`. The pre-commit
hook runs `just desktop-tauri-fmt`, which fails inside worktrees and will block every commit you
try to make (AGENTS.md, Common Gotchas #6).

**Only one agent may edit this checkout at a time.** If someone else is already working in
`~/Dev/buzz`, stop and say so rather than racing them.

**Scope:** Tasks 1–8 of the plan, all inside `crates/buzz-acp`. The desktop UI is explicitly NOT in
scope — it is a separate plan. Do not touch `desktop/`.

**Do ONE task per turn.** Complete Task 1, commit it, then stop and report. Wait to be told to
continue. Your turn is capped at 320 seconds — attempting several tasks in one turn will get you
killed mid-edit and leave the tree in a broken state.

**Non-negotiables:**
- TDD, exactly as the plan lays it out: write the failing test, RUN it and see it fail, implement,
  run it green, commit. Do not batch the tests to the end. Do not skip the "watch it fail" step —
  a test that never failed proves nothing.
- `git commit -s` on **every** commit. CI's DCO check fails the PR without a `Signed-off-by`
  trailer. Verify with `git log -1 | grep Signed-off-by` after your first commit.
- No `unsafe`. No new `unwrap()`/`expect()` in production paths — use `?`.
- Additive only; mirror the existing in-crate patterns the plan names (`session_new_full`,
  `handle_switch_model_control`, `steering_supported`).
- MCP changes apply at **turn boundaries** and must NEVER cancel an in-flight turn. There is
  deliberately no busy-path oneshot, unlike `switch_model`.
- `strictMcpConfig` stays **opt-in per managed channel**. Defaulting it on silently strips users'
  global MCP servers.
- Run `cargo test -p buzz-acp` after every task. Run `just ci` before you call the work done —
  clippy passing does not mean fmt passes.

**Verify, don't trust.** The plan cites shipped adapter source by file:line (e.g.
`acp-agent.js:3981-4008`). Those were verified once, but re-check any line you're about to build
on — the adapter is a versioned npm package under
`~/Library/Application Support/Buzz/node-tools/lib/node_modules/@agentclientprotocol/`, and it can
change under you. If a citation no longer matches, STOP and report rather than coding around it.

**Where to stop and ask:**
- Task 4 restructures `run_prompt_task`'s session lookup and knowingly duplicates a block. The plan
  says extract a helper only *after* the tests are green. If the duplication won't compile cleanly,
  report before inventing a different structure.
- The plan defers http/sse MCP servers (the Rust `McpServer` models stdio only). If a task seems to
  need remote servers, stop — that's a scope change, not an implementation detail.
- Any test you cannot make pass in two attempts: stop and report the actual failure output. Do not
  delete or weaken the assertion to get green.

**Report at the end:** which tasks completed, the `cargo test -p buzz-acp` summary line, the `just ci`
result, and anything in the plan that turned out wrong.

---

## What this is also testing

If you're running this through a Buzz channel agent, the run doubles as a capability test of Buzz
itself. Worth watching for:

- **Context survival** — the plan is ~1400 lines. Does the agent still have Task 1's decisions in
  context by Task 6, or does Buzz's `context_limit` truncate it into amnesia?
- **Long-horizon tool use** — 8 tasks × ~6 steps each, with real compile/test cycles between them.
- **The MCP irony** — this agent is implementing mid-conversation MCP grants while itself stuck with
  whatever MCP set it was born with. If it needs a tool it lacks partway through, that's the exact
  problem the feature fixes, demonstrated live.
