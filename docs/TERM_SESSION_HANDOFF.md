# Buzz Term session handoff

Pass a thread’s context into **Buzz Term** as an interactive Claude or Codex
CLI session, via a structured `term-session` chat card.

## Header control

In a thread panel header the trailing actions are:

**Info | Pin | Term | Popout**

**Term** opens **Pass thread context to Buzz Term**:

1. **Agent** — local managed agents that are channel members and active
   (`running` / `deployed`), same policy as Observe/Drive.
2. **Harness** — `claude` or `codex` from the ACP catalog’s `underlying_cli`
   (interactive CLIs only; not ACP adapters). Unavailable binaries stay listed
   but disabled with an install hint.
3. **Go** — posts a thread reply that @-mentions the agent and tells it to call
   buzz-dev-mcp `term_session_card`, then paste only the tool’s fence + a
   one-line ack (full prompt stays in JSON `prompt`).


## MCP tool path (`term_session_card`)

Agents with **buzz-dev-mcp** should build the card via the MCP tool instead of
inventing fence JSON:

1. Call `term_session_card` with `name`, `tool` (`claude`|`codex`), `prompt`,
   and optional `cwd` / `summary` / `sid` / `openclawWorkspace`.
2. Reply in chat with **only** a short one-line ack plus the tool’s returned
   `term-session` fence.
3. Never dump `prompt` as plain markdown; never put tokens/JWTs in the card.
4. `openclawWorkspace` is a **boolean only**, and only when the agent uses
   OpenClaw.

The Go @-mention (`buildTermSessionHandoffInstruction`) keeps `channelId` /
`threadId` / harness as summary context and points at this tool.

Rust: `crates/buzz-dev-mcp/src/term_session_card.rs`.

## Card schema

Fenced as ` ```term-session ` … ` ``` `:

```json
{
  "hula": "term-session",
  "v": 1,
  "name": "short title",
  "tool": "claude",
  "sid": "uuid",
  "cwd": "optional path",
  "prompt": "full handoff prompt",
  "summary": "optional one-liner",
  "openclawWorkspace": true
}
```

- `prompt` is stored for **Open** and **must not** be shown in the card UI.
- `openclawWorkspace` is a **boolean only** (never embed JWTs / Authorization).

## Open

1. Open/show Buzz Term for the channel+thread (`BUZZ_THREAD_ID` via attach).
2. Write the prompt under app data (`term-sessions/<sid>/prompt.txt`).
3. Launch interactive CLI in the PTY:
   - Claude: `claude "$(cat prompt.txt)"` (not `-p`, not `claude-agent-acp`)
   - Codex: `codex "$(cat prompt.txt)"` (not `codex-acp`)
4. Re-Open focuses a live tab for `sid`, otherwise relaunches.

## OpenClaw workspace (Claude-first)

When the card has `openclawWorkspace: true` and `tool` is `claude`:

1. `load_grant()` from the Desktop secret store (reuse only — **no** session
   mint/revoke).
2. If missing → toast `missing_grant_error` guidance; do not launch half-wired.
3. Upsert `openclaw-workspace` HTTP MCP into a **per-sid** `CLAUDE_CONFIG_DIR`
   under app data; set that env on the PTY.
4. Prepend `OPENCLAW_WORKSPACE_STANDING_INSTRUCTIONS` into the prompt file.

**Codex:** no MCP config bridge yet — Open continues without MCP wiring and
surfaces a toast. Treat as a known Claude-first gap.



## Return path (Buzz)

Every term-session card `prompt` must end with a **Return path (Buzz)** block so
the Term agent knows how to hand back to Buzz:

```
## Return path (Buzz)
- origin channelId: <...>
- origin threadId:  <...>
- mention to use:   @<AgentName>
- On "report back" / "hand back" / "I'm done":
  call buzz_draft_message with the origin channelId + threadId.
  Content = "<mention> <text JM asked for>". Draft only. JM clicks Send.
- Never draft to any other channel or thread unless JM gives new IDs.
```

Rules:

1. **Origin, not source.** IDs are the thread JM launched Term from (Term Go /
   deep link), **not** a different thread that was summarized into the prompt.
   If summarized ≠ origin, label both (`origin` vs `summarized/source`).
2. **Mention** is the Buzz agent that should pick the thread up next (usually
   the agent selected in the Term handoff popover). Do not hardcode a name; do
   not default to Term mentioning itself.
3. Trigger phrases only: `report back` / `hand back` / `I'm done`. Never
   auto-draft progress.
4. Prefer filling origin from a `hulabuzz://message?channel=&id=&thread=` (or
   `buzz://message`) deep link when the card was invoked from one.

Desktop: `buildTermSessionReturnPathSection` /
`resolveTermSessionReturnPathOrigin` in
`desktop/src/features/term-session/lib/returnPath.ts`. The Go instruction
(`buildTermSessionHandoffInstruction`) embeds a filled block and asks the
card-maker to pass `originChannelId` / `originThreadId` / `mentionToUse` to
`term_session_card`. The MCP tool appends the section when those fields are
set and the prompt lacks the heading.

## User-signer MCP (reads as JM; writes = draft)

Buzz Term Claude sessions wire **buzz-dev-mcp** into a per-sid `CLAUDE_CONFIG_DIR`
with `BUZZ_USER_SIGNER_DIR` pointing at `{app_data}/user-signer`. The PTY never
receives `BUZZ_PRIVATE_KEY` / nsec (`buzz_terminal::env_fence` allowlist).

`prepare_term_session_launch` prepends `USER_SIGNER_STANDING_INSTRUCTIONS` so Term
sessions know:

1. **Do not** message/chat with the human via Buzz drafts or `@` mentions of the user.
2. `buzz_draft_message` is **only** for content the **user** will send (they press
   Send) — not for the agent talking about its own progress.
3. Progress/status stays in the Term TUI / stdout; `buzz_read_*` is fine; drafts
   only when the human asked for a message to post.

| MCP tool | Behavior |
|----------|----------|
| `buzz_read_thread` | Desktop queries the relay **as the signed-in user** via file IPC |
| `buzz_read_channel` | Same for recent top-level channel history |
| `buzz_draft_message` | Emits `user-signer-draft`; Desktop saves a composer draft — **JM clicks Send**. Use only when JM asked for content to post; never for agent status |

IPC layout:

- Inbox: `{app_data}/user-signer/inbox/{id}.request.json`
- Outbox: `{app_data}/user-signer/outbox/{id}.response.json`
- Desktop watcher: `desktop/src-tauri/src/user_signer/`
- MCP client: `crates/buzz-dev-mcp/src/user_signer.rs`
- Draft UI: `desktop/src/features/term-session/lib/useUserSignerDraftListener.ts`

### Smoke-test

1. Build Desktop + sidecars (`buzz-dev-mcp` next to the app / target debug).
2. Open a thread → **Term** → pick Claude → **Go** → agent posts a `term-session` card → **Open**.
3. In the Term Claude session, call `buzz_read_thread` with the thread id (and channel id). Expect JSON events with `"asUser": true`.
4. Only if you intentionally asked Term to prepare a message: call `buzz_draft_message` with that body. Desktop should toast **Buzz Term draft ready**, open the channel/thread composer with the draft — click **Send** yourself. Do **not** expect Term to draft “I’m working” / progress notes.
5. In the PTY: `echo $BUZZ_PRIVATE_KEY` / `env | rg PRIVATE` must be empty; `echo $BUZZ_USER_SIGNER_DIR` should show the IPC path.

## Files

- `desktop/src/features/term-session/` — parse, roster, harnesses, card UI,
  handoff popover, launch/open, `returnPath.ts` (Return path section)
- `crates/buzz-dev-mcp/src/term_session_card.rs` — MCP `term_session_card`
- `desktop/src-tauri/src/commands/term_session.rs` — `prepare_term_session_launch`
- Markdown fence registration mirrors playground cards
- `desktop/src-tauri/src/user_signer/` — Desktop IPC signer watcher
- `crates/buzz-dev-mcp/src/user_signer.rs` — `buzz_read_*` / `buzz_draft_message`
- `desktop/src/features/term-session/lib/useUserSignerDraftListener.ts` — draft → composer
