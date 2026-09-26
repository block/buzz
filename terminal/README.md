# Buzz terminal

A dedicated, editor-centered client for supervising existing Buzz agents.
Pi supplies terminal rendering and editing, not an agent runtime. Buzz remains
the source of truth for identities, channels, threads, and messages.

## Try it

From the repository root:

```sh
eval "$(./bin/hermit env --activate)"
pnpm install --frozen-lockfile --ignore-scripts
just terminal-demo
```

The clearly labelled demo is completely offline. It exercises the same UI,
conversation state, subscriptions, and extension registry with fixture data.
It does **not** verify access to a real Buzz community.

To connect, explicitly configure `BUZZ_RELAY_URL` and `BUZZ_PRIVATE_KEY` in your
environment, then run `just terminal`. The key must be your human identity,
not a managed agent's injected credentials. Hex and nsec are accepted. Do not
put private keys in command-line arguments, checked-in files, or bug reports.
The client does not read credentials from the desktop installation.

After `just terminal-build`, put this checkout's `target/debug` on `PATH`:

```sh
export PATH="$PWD/target/debug:$PATH"
buzz                       # create a new private channel every time
buzz join <channel-uuid>   # open exactly this existing channel
buzz --demo                # offline preview, no relay writes
```

The composer accepts a draft while setup is pending. Set `BUZZ_AGENT_PUBKEY`
to an existing agent's exact hex key for immediate routing. Otherwise the
client selects your last chosen owned agent, or the first owned agent sorted
by name (public key breaks ties). You can send without opening Ctrl+R; use it
only to switch recipients. A confirmed manual agent choice is remembered
across launches, separately for each relay URL and human identity, under
`~/.buzz/terminal/agents/`. These files contain public keys, never credentials.
Automatic defaults and environment overrides do not overwrite your last
manual choice. If that agent is no longer discovered with verified ownership,
the client falls back to the first owned agent. Discovery reads your owner-authored agent
directory as well as recent channel rosters, then verifies ownership from
each agent's signed profile. Agents do not need to share a channel with you
to appear. The recipient picker updates as profiles arrive without losing
your search or draft. A configured key also works outside the discovery window.
New channels invite the selected existing agent; this does not provision or
start an agent runtime. The agent must already be running to respond.

`buzz join` opens history when you are already a member, or requests membership
in an open channel. Private channels require an invitation. Missing channels,
denied access, and malformed IDs never fall back to creating a channel.
Joining does not invite a configured agent that is absent from that channel.
Automatic selection when joining only considers owned agents already in the channel.
`/retry-setup` retries the same channel and signed operation after a setup error.
Reconnects do not create channels. The resume command is shown on detach;
it restores relay history, not unsaved local drafts.

This is currently a source-development client, not a released standalone
bundle: Node 24, the installed `terminal/` dependencies, and the companion
Rust host are required. `BUZZ_TERMINAL_ENTRY` can point the CLI at
`terminal/src/main.ts`; packaged assets may live at `share/buzz/terminal` next
to the installation's `bin` directory. Existing scriptable subcommands and
the agent-facing `buzz_cli::run_from_args` library remain unchanged.

The host authenticates using NIP-42. `BUZZ_AUTH_TAG` is supported for an
explicitly configured delegated identity. Use `wss://` except for loopback
development. The relay signing key comes from its NIP-11 `self` field; an
explicit `BUZZ_RELAY_PUBKEY` can pin it. `BUZZ_TERMINAL_HOST` overrides the
default `target/debug/buzz-terminal-host` executable.

## Working in the terminal

| Key | Action |
| --- | --- |
| Ctrl+K | Search channels and open conversations |
| Ctrl+R | Select an exact recipient from the channel roster |
| Ctrl+G | Command palette, without disturbing your draft |
| Ctrl+T | Open a thread from recent channel messages |
| Alt+A | Toggle owner-only agent activity |
| Esc | Close the picker or return from activity |
| Enter | Send to the displayed recipient, signed as you |
| Shift+Enter / Ctrl+J | Insert a newline |
| PgUp / PgDn | Scroll the transcript |
| Ctrl+L | Jump to latest and resume following output |
| Ctrl+Shift+F | Search the visible transcript |
| Ctrl+Q / Ctrl+C | Detach; repeat to discard unsaved local state |

Slash commands mirror the palette. Use `//` to send a literal leading slash.
`/retry` resends the **identical signed event** after an uncertain result.
`/discard` explicitly forgets delivery tracking; it does not retract a message.
`/reconnect` retries after automatic reconnect attempts are exhausted.
`/close` closes a view only when it has no draft or pending send.

Drafts, recipients, and scroll positions belong to each conversation. A late
acknowledgement cannot clear another conversation's editor. Switching contexts
does not cancel work. Closing the terminal terminates only its local host,
never the remote harness.

## First milestone boundaries

- One explicitly configured community and human identity per process.
- Attach to existing agents. No provisioning, runtime configuration, or tool
  approval inbox. No fictitious universal sub-agent tree: activity identifies
  agent, channel, runtime session (when known), and turn.
- Drafts, read markers, and delivery tracking are in memory, not persisted.
  Restarting cannot retry a former process's uncertain send safely; inspect
  channel history first.
- Up to 24 open conversations, 100 discovered channels, 1,000 profiles, and
  200 recent messages per conversation (plus a pinned thread root). This is a
  recent-history window, not a full archive or gap-free historical sync.
- The host retains up to 256 signed sends per process. At capacity it refuses
  new sends rather than evicting identities needed for safe retries. Resolve
  pending deliveries before restarting; durable delivery storage is future work.
- Channel views show root messages. Open a thread to read and reply to its
  replies. Editing, deletion, reactions, attachments, and encrypted DMs are
  not implemented in this milestone.
- Observer telemetry is NIP-44-encrypted, owner-only, ephemeral, and bounded.
  Disconnect marks working status unknown. Missed telemetry is not recovered.
  `turn_completed` means ended, not necessarily succeeded.
- All identity labels and remote content are treated as untrusted terminal
  text. Executable escapes, directional controls, and generated hyperlinks are
  removed. Markdown, code blocks, and diff colors remain.

## Architecture and extensions

`main.ts` launches the UI and `buzz-terminal-host`. `transport.ts` handles
bounded JSONL RPC; `protocol.ts` defines the v1 wire contract. The Rust host
authenticates, verifies events and NIP-OA ownership, decrypts telemetry, signs
messages, and reconnects persistent subscriptions. Keys never appear in the
frontend protocol. The UI removes key variables from its environment after
spawning the host.

`session.ts` maintains subscriptions, including unfiltered-by-member roster
watches for already discovered channels so membership removals are observed.
`store.ts` owns community state and immutable send snapshots. The UI uses
Pi's alternate-screen renderer, multiline Editor, ScrollView, and Markdown.

`plugins.ts` is a small, typed extension API for **trusted, bundled code**:

```ts
const plugin = {
  id: "example.status",
  activate(context: PluginContext) {
    context.command({ name: "status", description: "Show status", run });
    context.view({ id: "status", title: "Status", entries });
    context.onChange(refresh);
    context.onDispose(cleanup);
  },
};
plugins.load(plugin);
```

The first-party activity view and diff renderer use this API. Unloading removes
commands, views, renderers, subscriptions, and registered resources. Partial
activation rolls back. There is no filesystem scanning, package auto-install,
or third-party plugin loader. In-process plugins have normal Node privileges;
this is not a sandbox. The API deliberately exposes no signing or arbitrary
host-request capability.

## Verification

```sh
just terminal-check terminal-test
```

Tests cover production state and keyboard paths, including rendering the real
TUI into xterm-headless, multiline drafts, narrow terminals, identity routing,
late acknowledgements, uncertain retries, replay deduplication, membership
loss, thread retention, observer completion, terminal injection, and plugin
cleanup. The Rust integration test starts the real host against an isolated
WebSocket fixture: NIP-42 auth, signature checks, subscription lifecycle,
disconnect before acknowledgement, resubscribe, identical retry, and EOF exit.

For a real-relay test, start a **disposable loopback dev relay** backed by its
own Postgres and Redis (see `TESTING.md`; never use your desktop database).
The test generates fresh human/agent identities, publishes owner policies and
attested profiles without shared channels, drives the actual TUI and signing
host through discovery/invitation/send, renders a signed test responder's
thread reply, and reopens the channel. It does not invoke an LLM or use your
credentials. The dev relay must admit these generated identities and NIP-OA
authentication. Run:

```sh
BUZZ_TEST_RELAY_URL=ws://127.0.0.1:3037 \
  cargo test -p buzz-terminal-host --test live_relay -- --ignored --nocapture
```

Before marking a PR ready, also test with a real Buzz community and two owned
agents. Send to one, switch to the other, leave a draft, return, then reconnect.
Confirm delivery once, correct recipients, retained drafts, and that agents
keep working after the terminal closes. Human confirmation is required.

Tracked by AGNTOPS-499.
