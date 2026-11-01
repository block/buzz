# muse-acp: Muse as a Buzz agent harness

`muse-acp` lets a Buzz channel talk to Muse — the Meta AI assistant behind
muse.ai — the same way it talks to Goose, Codex, or Claude Code: as a
first-class agent member of the channel.

There is no public Muse agent API, so the bridge does what the browser
does. It speaks the Agent Client Protocol (ACP) over stdio to `buzz-acp`
and relays each turn to the user's own Muse web session in Chromium, then
streams Muse's reply back into the channel as ACP message chunks. One Buzz
channel session maps to one browser tab, so each channel keeps its own
continuous Muse conversation — exactly the continuity you get chatting on
muse.ai, plus the channel's framed prompt as additional context.

## Use it today: Add custom harness (no PR needed)

You don't have to wait for the builtin catalog entry below. Buzz already
lets you register any ACP-speaking command as a harness:

1. Install the bridge in a terminal (see below for the pre-merge
   variant):

   ```sh
   cargo install --git https://github.com/block/buzz.git muse-acp
   ```

2. In Buzz, open **Add custom harness** (from New agent, or Settings →
   Harness catalog) and fill in the form like this:

   ![Add custom harness filled in for Muse](custom-harness.png)

   | Field | Value |
   |---|---|
   | Name | `Meta` |
   | ID | `meta` (auto-derived from the name) |
   | Command | `muse-acp` |
   | Arguments | none |
   | Env vars | none |
   | Docs URL | `https://github.com/block/buzz/tree/main/crates/muse-acp` |
   | Install hint | `cargo install --git https://github.com/block/buzz.git muse-acp` |

3. Hit **Save**. The Command badge reads **Found on PATH** once the
   binary is installed, and Meta becomes a selectable harness.

Two things worth knowing about this form: the Install hint is guidance
text shown in the catalog when the binary isn't detected — it is not an
auto-installer, so run the command in a terminal first. And custom
harnesses resolve their command from `PATH` with no auth probe, so
sign-in state is handled by the bridge itself (see below), not by Buzz.

Until this PR merges, the `--git` command above installs from `main`,
which doesn't have this crate yet — check out this branch instead and
run `cargo install --path crates/muse-acp` from the repo root.

## Install (after this PR merges)

From the Buzz desktop app, pick **Meta** in the agent harness dropdown and
hit **Install**. That runs:

```sh
cargo install --git https://github.com/block/buzz.git muse-acp
```

which builds this crate from source (a Rust toolchain is required) and
places `muse-acp` on `PATH`. There are no prebuilt binaries yet; the
install script story can graduate to that once releases exist.

Manual equivalent:

```sh
cargo install --path crates/muse-acp
```

## First run: sign in once

The bridge launches Chromium with a persistent profile at
`~/.muse-acp` (override with `MUSE_ACP_PROFILE`). On first run, with no
saved session, it opens a **visible** browser window and waits for you to
sign in to Muse. Once the composer appears, the session is recorded and
later runs go headless. To force headed/headless:

```sh
MUSE_ACP_HEADLESS=0 muse-acp   # visible window (sign-in)
MUSE_ACP_HEADLESS=1 muse-acp   # headless (requires a saved session)
```

Check sign-in state without launching a browser (this is also the
desktop catalog's auth probe):

```sh
muse-acp auth status   # exit 0 = saved session, 1 = none
```

If the saved login expires, the next turn fails with a clear
`NotSignedIn` error and the following run waits for sign-in again — the
stale marker self-heals instead of stranding turns.

## How a turn flows

1. `buzz-acp` spawns `muse-acp` (no arguments) and sends ACP over stdio:
   `initialize` (protocol version 2), `session/new`, `session/prompt`,
   `session/cancel`.
2. On the first prompt of a session, the `systemPrompt` from `session/new`
   is prepended to the message. That prompt carries the channel context
   buzz-acp framed (channel identity, instructions, history pointers),
   forwarded verbatim — the bridge adds no paraphrase between the channel
   and Muse. Later turns send only the new message; the tab already holds
   the conversation.

## Side-chat routing: a tagged wake goes to that channel's side chat

A tagged wake should not land in yet another fresh Muse web chat. It
should land in the Muse side chat that already holds that channel's
conversation — the side chat a user would open by hand for that channel.

`muse-acp` derives the channel on every wake, from three sources in
order:

1. The prompt text's `Channel:` line (buzz-acp frames each wake with
   `Channel: MASKY-CMO (#<uuid>)`).
2. The `session/new` `_meta.sessionTitle` (`Agent · #channel`).
3. The `session/new` standing prompt's `Channel:` line, as a fallback.

It then, on the first turn for that channel in a tab, clicks the side
chat whose title matches, in order: an explicit
`MUSE_ACP_SIDE_CHAT_MAP` entry, `<channel> (buzz)`, then the bare
channel name. Matching is case-insensitive and by first text line, so a
list item that shows a title plus metadata still matches. Later turns
in the same channel skip the click — the tab is already there — and a
wake for a different channel re-routes the tab.

If no side chat matches, the default (`MUSE_ACP_SIDE_CHAT_MODE=auto`)
falls back to the tab's current conversation (fresh after the new-chat
step) and logs the chat-list titles it did see, so the mapping can be
fixed without the channel losing its reply. `strict` fails the turn
instead, and `off` disables routing entirely and restores the old
fresh-chat-only behaviour.

Caveat, stated plainly: the chat-list selectors are, like the rest of
`src/selectors.rs`, unverified against the live logged-in muse.ai app
until a signed-in run confirms them. The selection script is built to
fail diagnostically — on a miss it reports the candidate titles it saw
— rather than silently clicking the wrong chat. Until that run happens,
treat side-chat routing as implemented and mock-tested, not
live-verified (see "The DOM model" below).

### Mock simulation (no Meta login needed)

The routing can be exercised end to end without a Muse account, by
pointing the bridge at a local page that mimics the shape of muse.ai —
a sidebar of titled chat buttons, a composer, and an assistant message
element — and feeding it ACP by hand:

```sh
# Serve a mock page (sidebar buttons, composer, assistant element)
python3 -m http.server 8765 --directory /tmp/mock-muse

# Build the bridge, then run it against the mock page
cargo build -p muse-acp
MUSE_ACP_MUSE_URL=http://127.0.0.1:8765/ \
MUSE_ACP_CHROME=/opt/meta-chromium/chrome \
MUSE_ACP_HEADLESS=1 \
MUSE_ACP_PROFILE=/tmp/muse-acp-mock-profile \
  ./target/debug/muse-acp <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"_meta":{"sessionTitle":"Muse · #MASKY-CMO"}}}
{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"<id from step 2>","prompt":[{"type":"text","text":"Channel: MASKY-CMO (#3043c06d-6cf1-42f8-b1b0-cb2d61711ea1)\nMessage: hello from buzz"}]}}
EOF
```

A successful run streams a reply whose text names the selected side
chat (`Reply from MASKY-CMO (buzz) ...` on the mock page) — proof the
click happened before the submit, not just that a reply arrived.
`MUSE_ACP_MUSE_URL` is a test seam only; production always uses
https://muse.ai/.
3. The bridge submits the message in the tab (composer + send, or Enter),
   polls the reply, and emits each text delta as an ACP
   `session/update` `agent_message_chunk`. It tracks message identity so
   the previous reply is never re-emitted, the user's echoed prompt is
   never relayed as Muse's reply, and a re-render resets the baseline
   instead of duplicating text.
4. When the reply settles (no stop control, no text growth for three
   consecutive polls), the turn answers `stopReason: "end_turn"`.
   `session/cancel` stops the reply and answers `"cancelled"`.

## ACP surface

Implemented: `initialize`, `session/new`, `session/prompt`,
`session/cancel` (notification). Everything else answers `-32601`.
Advertised capabilities: protocol version 2, no `loadSession`, no image /
audio / embedded-context prompt capabilities.

Not forwarded (by design): `mcpServers` from `session/new` (Muse web has
its own tools), model/effort/provider settings (Muse web exposes no such
controls to this bridge). Turns are serialized per session; a second
concurrent prompt on one session is rejected as "session busy".

## The DOM model: the part that needs a logged-in run

`src/selectors.rs` is the bridge's model of the Muse web app's DOM
(composer, send/stop controls, assistant messages). It was written from
the standard shape of a chat web app, **not** verified against the live
logged-in app, and it says so in its own header. It is built to fail
loudly: probes report which candidate selector matched, and a turn that
cannot find the composer or a response ends with a `WebDom` error naming
what was looked for.

To verify or fix it: run headed (`MUSE_ACP_HEADLESS=0`) against a
signed-in session, watch stderr for the selector diagnostics, and replace
the candidate lists with the real hooks the app uses. Nothing else in the
bridge depends on those strings.

## Configuration

| Variable | Purpose |
|---|---|
| `MUSE_ACP_PROFILE` | Bridge profile dir / Chromium `--user-data-dir` (default `~/.muse-acp`) |
| `MUSE_ACP_CHROME` | Chromium binary path (default: `PATH` + well-known locations) |
| `MUSE_ACP_HEADLESS` | `0`/`1` to force headed/headless (default: headed until a session is saved) |
| `MUSE_ACP_LOG` | Log filter, e.g. `debug` (default `info`); logs go to stderr, never stdout |
| `MUSE_ACP_SIDE_CHAT_MODE` | `auto` (default: side chat, fall back to fresh chat on a miss), `strict` (fail the turn on a miss), `off` (never select a side chat) |
| `MUSE_ACP_SIDE_CHAT_MAP` | Comma-separated `channel=title` overrides, e.g. `MASKY-CMO=My CMO chat` (default titles are `<channel> (buzz)`, then `<channel>`) |
| `MUSE_ACP_MUSE_URL` | Test seam only: override https://muse.ai/ with a mock page (see mock simulation above) |

## Limitations (v1)

- Desktop only; the mobile app has no harness catalog for this to plug into.
- CDP traffic is serialized through one connection mutex, so turns on
  different tabs share the protocol layer (see `src/chrome.rs`).
- The bridge reuses the user's interactive Muse session: turns consume
  whatever plan/rate limits that session has, and Muse's replies are
  whatever the web app returns — there is no separate API quota.
- DOM selectors are unverified (see above). Until a logged-in run
  confirms them, treat this harness as experimental.
