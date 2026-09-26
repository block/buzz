# Browser agent Observe / Drive

Status: shipped (Hula Desktop). Not OpenClaw gateway Chromium / CDP.

## Product

Playground Sites (and OS pop-outs of playground) support two grant modes. **Pinned sites are out of scope** for Observe/Drive:

| Mode | Human | Agent |
|------|-------|-------|
| **Off** | Full control | No session access |
| **Observe** | Drives the page | May **read** console, network (headers/status/**bodies**), nav/URL/title, DOM/a11y when available. No PHI scrub. |
| **Drive** | Sees theater cursor; input locked (unless Take control) | Takes over navigate/click/type/scroll/key/waitFor. |

**Take control** (chrome, Drive only) → temporary human assist: page unlocks, grant stays **Drive**. **Release control** restores the Drive lock without re-picking the agent. **Off** / clear grant is the separate revoke path.

### Grant binding

- One agent per browser webview at a time.
- Grant names **which agent** (pubkey / managed id) via **which channel** + optional **thread root**.
- Entry: agent message card (“Watch this Site” / “Let agent drive”) **or** chrome Off|Observe|Drive picker (agent+thread required if not opened from a card).
- Replacing an existing grant requires explicit confirm.
- **Lifetime:** until that browser webview is **disposed/closed**. Hide does not clear; dispose/close clears.

### Agent target (prefer `surfaceId`)

Agents should target a grant by **`surfaceId`** (playground session id). It stays stable across detach / OS pop-out / host rebind. `webviewLabel` changes when the webview moves (`playground-{sid}` → `playground-{sid}--{window}`).

| Field | Stability | Use |
|-------|-----------|-----|
| **`surfaceId`** | Stable for the session | **Preferred** on `browser_drive`, `browser_observe_poll`, `browser_snapshot` |
| `webviewLabel` | Changes on popout | OK for a single turn if you just listed grants; re-resolve after detach |
| omit both | — | Allowed when the agent has **exactly one** live grant |

`browser_agent_grants` returns both fields; after popout Desktop rebinds the grant and updates the mirror so `surfaceId` resolves to the live label.


### Browser tabs (same group)

Playground sessions live in a **browser group** (`browserId`, `tabSids[]`, `activeTabSid`). The Browsers left-nav shows **one row per group** (preview = active tab). Chrome shows a thin tab strip (title; **main tab has no X**; extras have X; **no +** — tabs only from in-page `window.open` / `target=_blank`). Secondary tabs lock the URL bar (read-only); main stays editable unless Drive locks chrome.

- Each tab is a normal playground session + its own `playground-{sid}` WKWebView.
- **Drive screen chat posts:** While a **Drive** grant is active, each meaningful navigation (URL without hash; hash-only churn ignored) triggers a host-side viewport screenshot posted as the grant's agent into the bound channel/thread, with a short path caption (bullets from Drive actions since the last post — click/type/key/navigate; typed text never echoed). Debounced (~1.2s settle + min interval). Observe-only grants do not post.
- **New tab from the page:** `window.open` / `target=_blank` is handled as **MVP B** — Rust registers `WebviewBuilder::on_new_window`, emits `playground-webview-new-tab`, schedules a sibling tab in the opener's group, focuses it, and returns **`NewWindowResponse::Deny`** (no random NSWindow). The opener may break (`window.open` return value / `window.opener`) — intentional for this MVP; a later path A can use `Create { window }` with a related webview.
- **One live WKWebView per playground sid:** label is always `playground-{sid}`. Detach / pin Open / RHS stage **reparent** that child across windows instead of creating `playground-{sid}--{window}` siblings (siblings forked DOM + Drive phase). Pop-out close reparents back to main (hidden) when possible so Observe/Drive stay warm.
- **Grants:** one agent per browser group. Observe/Drive bind to the **active tab** `surfaceId`. The **main tab** (`tabSids[0]` / `mainTabSid`) is **primary focus** and cannot be dismissed from the strip (Browsers **Remove** still disposes the group). Switching tabs (or focusing a new tab) **rebinds** the grant onto the active tab's webview label (same idea as detach host rebind). Closing a secondary tab disposes that session/webview only.
- **Agent tabs:** MCP `browser_tabs` lists the group (`mainTabSid`, `activeTabSid`, `tabs[]` with `isMain`). `browser_switch_tab` focuses a `surface_id` (Desktop rebinds). Observe events `tab_opened` / `tab_switched` announce changes. Prefer returning to the main tab for primary work.
- Existing sessions migrate to one-tab groups (`browserId` may equal `sid`).

### Site runbook

Learned how-to knowledge for a **playground session** (`sid:…`) or **pinned site** (`pin:…`) survives agent sessions:

- **Agent brief** — short imperative instructions mirrored into grant context / `browser_observe_poll.runbook`.
- **Procedures** — engineer-readable steps (`active` | `pending` | `archived`). Agents propose via `browser_runbook_propose` (pending until Accept). UI: Settings → Pinned sites → **How to use this site**, and Browsers row **Runbook**.
- Inject is brief + active titles/summaries only; full steps via `browser_runbook_get`.
- Community pins sync brief + active procedures in the pin payload.

### Non-goals

- Observe/Drive on pinned sites (sidebar pins / link pin hosts).
- OpenClaw `browser` tool / gateway Chromium / remote CDP.
- Cross-webview multi-agent grants.
- Persisting grants across app restarts.
- PHI redaction on Observe payloads.
- Cutting a desktop release as part of this change.

## Grant schema

Persisted in-memory (and mirrored under app data for local agent tool poll):

```json
{
  "webviewLabel": "playground-{sid}" [| "--{window}"],
  "surface": "playground",
  "surfaceId": "{sid}",
  "agentId": "{managed agent id or pubkey}",
  "agentPubkey": "{hex pubkey}",
  "channelId": "{channel id}",
  "threadRoot": "{event id}" | null,
  "mode": "observe" | "drive",
  "userHasControl": false,
  "createdAtMs": 0
}
```

Rust type: `BrowserAgentGrant`. Cleared on webview dispose/close for that label (and when no sibling labels remain for the same `surfaceId`).

## Chrome

- Playground chrome (main overlay and playground pop-outs): Agent controls sit behind a **Bot** icon toggle next to Detach/Inspect (muted when Off; orange when Observe/Drive granted). Toggle expands a second chrome row with **Off | Observe | Drive**, status chip (`Observe · {AgentName}` / orange `Drive · {AgentName}`), and **Take control** / **Release control** when Drive (does not revoke). Default collapsed on load. Agent name comes from the managed-agent roster (not pubkey). Live Drive chrome also shows a short **activity** caption (from `drive` / `drive_error` / `drive_started` / `snapshot` observe events — Clicking… / Typing… / Pressing Enter / etc.). The strip **persists for the whole Drive turn**: captions update in place between tools; only after ~12–15s sustained idle (LLM think-time) does it show **Finished**, then fade/clear. Observe/Off and the Browsers list Bot panel omit the strip. **No Dispose on chrome** — destroy sessions via Browsers left-nav **Remove** (Confirm?/Cancel → `disposeBrowserSession`); programmatic dispose paths (grant clear / webview close) still work.
- **Pin open host:** conversation/thread pen pin reuses the playground session (Agent chrome + grants) in the **right-hand** idle-auxiliary browser slide-out (same shell as `openLinkSidePanel`) — not a left docked split, not the grant-less URL pin webview alone, and not windowed inset-0 cover. Watch / Let agent drive use the same RHS host.
- **Chrome nav + viewport lock (Drive):** while Drive is granted and the human has **not** Taken control, Back / Forward / Refresh, the editable URL/omnibox, **Desktop | Responsive | Mobile**, resolution W×H inputs / resize handles, and mobile museum device / orientation / **scale** are disabled (tooltip “Agent is driving”). In-page **human scroll** (wheel / trackpad / touch / Space·PageUp·PageDown·arrows) is blocked by `#__buzz_agent_lock` listeners; agent `scroll` / `key` actions still work. Take control re-enables chrome + human scroll; Observe / Off leave them as today. Detach and Browsers-list Open stay available (window chrome only).
- Agent+channel(+thread) picker when grant not pre-bound from a card.
- Playground card actions: **Pin** icon (top-right → current channel/thread header pins), **Open** (RHS slide-out), **Open as Split** (thread cards only), and a **Bot** icon that opens the same Observe/Drive grant dialog as browser chrome. Channel/thread pins persist with playground sessions (same identity); disposing a Browsers row removes that sid’s pins everywhere.
- **Pinned sites:** no Observe/Drive chrome; `browser_agent_grant_set` rejects `surface=pin`.
- **Browsers** left-nav lists one row per **browser group** (playground / agent-driven; preview = active tab; including windowed; Add opens a new one-tab group from a URL). Each row: live page preview (left; ~4s refresh via `playground_webview_screenshot` at **viewport-native** resolution — default WK snapshot, no `snapshotWidth` / no full-page height expand — scaled to fit / letterbox with `object-contain`, not cover-crop; favicon/Globe placeholder if capture fails or session is cold), title/URL, a compact **viewport caption** (`Desktop · 1280×800` / `Responsive · W×H` / `Mobile · W×H` · scale% when not 100 — from the live playground viewport store), + one binding chip per pin/grant (`{channel}` or `{channel}: Thread`; click opens channel or thread), Bot toolbar toggle (Off/Observe/Drive + status — **no** Take/Release here; those stay on live playground chrome), then Open / Window / Remove. **Open** on a windowed row focuses/raises that OS or embed window (not only an in-app overlay). **Remove** disposes the session (closes webview, clears Observe/Drive grants, closes detached host). Rows whose playground WKWebView is not running yet (e.g. after app restart, or never opened this run) show a compact **Cold** chip beside the title; the chip hides once the webview is live. No pinned-site rows (those stay in the sidebar). Preview is real WKWebView pixels on macOS; other platforms / closed webviews stay on the placeholder.
- Grant dialog (chrome + Browsers): select agent → paste channel/thread link (or Advanced raw ids) → Observe|Drive. Agent picker lists **active local** Desktop-managed agents only (`running`/`deployed`, same as sidebar Agents X/Y; empty → “No running local agents”). Optional **Pin this browser to that channel/thread** uses `pinPlaygroundToConversation` (same header pin store as playground cards).

## Observe data plane

Instrumentation runs **inside** the Buzz WKWebView (page script), not OpenClaw Chromium:

1. On grant Observe/Drive, Desktop injects/hooks: `console.*`, `fetch`, `XMLHttpRequest`, and records nav from existing Rust nav emitters.
2. Events land in an in-memory ring (`BrowserObserveBuffer`) and are mirrored to `{appData}/browser-agent/{label}/events.jsonl`.
3. Bound agent reads via:
   - Desktop IPC / Tauri: `browser_observe_poll` (cursor + limit)
   - Optional subscribe: Tauri event `browser-agent-observe`
   - Local MCP tools on `buzz-dev-mcp`: `browser_observe_poll` (grant-gated by agent pubkey)

**Network bodies:** captured from page-side `fetch`/XHR response text when available (size-capped). Native WKWebView request body taps without eval are not used. If a page is cross-origin opaque or uses streams we cannot clone, body may be omitted — event still carries URL/status/headers when known.

**Pins:** out of scope — no Observe instrumentation or Drive on pinned-site webviews.

## Drive data plane

When `mode=drive`:

1. Human pointer/keyboard on the child webview is locked (full-page `#__buzz_agent_lock` overlay) unless the human has Taken control. Lock stays for the human; agent hit-tests **through** it via `document.elementsFromPoint`, skipping `#__buzz_agent_lock` and `#__buzz_agent_cursor`.
2. Agent actions (`browser_drive`): `navigate` | `click` | `type` | `scroll` | `hover` | `key` | `waitFor` — injected with a visible ghost cursor / highlight.
3. Every action has an `id`. Page returns `{ id, ok, kind, hit?: { tag, role, name }, url, error? }` via cookie `__buzz_ba_drive_result`. Desktop writes a `drive` (ok) or `drive_error` event to `events.jsonl`.
4. **`key`:** `{ "kind":"key", "key":"Enter" }` — Enter, Tab, Escape, Backspace, arrows. Fires keydown/keypress/keyup on `document.activeElement`. If keydown not defaultPrevented: Enter in input+form → `form.requestSubmit()`; Enter in textarea → newline; Tab → move focus. No site special-cases.
5. **MCP → Desktop path:** `buzz-dev-mcp` `browser_drive` validates the DriveAction shape (object or JSON string; field is **`kind`**, not `type`), assigns `id`, appends one line to `{appData}/browser-agent/{label}/drive-inbox.jsonl`. Prefer **`surface_id`** when calling (see Agent target). Desktop drains the inbox via a **Rust-side watcher** (~200ms over live grants) plus optional chrome backup poll; **atomically renames** the inbox to a temp file, then reads/deletes the temp (so appends during processing are not truncated away). Bad lines / unknown kinds → `drive_error` (never silent ok).
6. **Batch wait (default):** after queueing, MCP waits up to ~10s for matching `drive` / `drive_error` events in `events.jsonl` and returns per-step `results` + final `url`. Set `queue_only=true` to return immediately after queue.
7. **Take control** → unlock page, keep Drive grant (`userHasControl`). **Release control** → lock again. **Off** clears the grant.
8. Detach / conversation-pin open / navigation rebinds the grant onto the live webview label and reinstalls instrumentation so Drive lock is not lost across hosts. Agents holding `surfaceId` keep working mid-turn.

Playground only (existing `playground_webview_eval`). Pin navigate/drive paths remain unreachable for new grants.

### Observe nav events

Main-frame only. Same-URL repeats are dropped; title is filled when available (page-load / `pushNavIfChanged`). Keeps “what did you observe” readable.

### DriveAction shape

```json
{
  "kind": "navigate|click|type|scroll|hover|key|waitFor",
  "id": "optional",
  "url": "navigate",
  "x": 0, "y": 0,
  "text": "type or waitFor text",
  "selector": "type target or waitFor",
  "dx": 0, "dy": 0,
  "key": "Enter|Tab|Escape|Backspace|ArrowLeft|ArrowRight|ArrowUp|ArrowDown",
  "urlContains": "waitFor",
  "timeoutMs": 10000
}
```

Optional MCP batch: `actions: DriveAction[]` — validated then queued sequentially; by default MCP waits for per-step results (see batch wait above).

## Agent path (local first)

Prefer **Desktop-managed / local ACP** agents that can call Desktop-side tools:

| Tool | Role |
|------|------|
| `browser_observe_poll` | Drain observe events. Prefer `surface_id`; `webview_label` optional. |
| `browser_drive` | Drive action or `actions` batch (Drive mode only). Prefer `surface_id`. Validates `kind`; waits for results unless `queue_only`. |
| `browser_snapshot` | Request DOM/a11y snapshot. Prefer `surface_id`. Writes `snapshot-request.json`; Desktop fills `kind=snapshot`. Optional `screenshot=true`. |
| `browser_agent_grants` | List grants (`surfaceId` + live `webviewLabel`) for this agent |
| `browser_tabs` | List tabs in the granted browser group (`mainTabSid` primary; extras from in-page open). Prefer `surface_id`. |
| `browser_switch_tab` | Focus a tab by `surface_id` (rebinds grant). Poll `tab_switched` or re-call `browser_tabs`. |
| `browser_runbook_get` | Site runbook for the granted browser: brief + procedure index, or full steps for `procedure_id`. |
| `browser_runbook_propose` | Queue a pending how-to procedure (title + steps). Human Accept in Desktop required. |

Remote Gateway agents: no CDP bridge in this change — document follow-up consent channel if remote needs the same stream.

## Security notes

- One agent per webview; replace requires confirm.
- Drive actions require live Drive grant matching caller pubkey.
- Observe has no PHI scrub (product decision).
- Never routes through OpenClaw browser debug ports (already blocked for playground URLs).
