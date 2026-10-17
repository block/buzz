# Buzz reports in Codex

A local bridge routes reports from explicitly registered Buzz agents/channels into
existing Codex chats using `codex queue`. It does not create agents, choose their
models, send Buzz messages, or change worktrees. Codex can send assignments using
`buzz messages send` under the user's normal messaging policy.

Requires Python 3.10+ on macOS/Linux, Codex with `queue`, and a Buzz CLI containing
`messages watch`. The installed Buzz release may need upgrading before use.

## Configure

Save a private JSON file outside the repository with this shape. Replace every
placeholder with verified identifiers. `buzz_command` and `codex_command` are
argument arrays, never shell strings. A Buzz authentication wrapper may be the
first command. For managed agents, use the runtime's injected Buzz identity.
Never put private keys in this file or CLI arguments.

```json
{
  "relay": "wss://your-buzz-community.example",
  "buzz_command": ["/absolute/path/to/authenticated-buzz-wrapper"],
  "codex_command": ["/absolute/path/to/codex"],
  "routes": [{
    "channel": "CHANNEL-UUID",
    "thread": "CODEX-CHAT-UUID",
    "authors": ["AGENT-64-CHAR-HEX-PUBLIC-KEY"],
    "worktrees": ["/absolute/repo-a-worktree", "/absolute/repo-b-worktree"]
  }]
}
```

Use one Buzz channel per independent task because the Buzz harness keys its
conversation sessions by channel. A task may own zero, one, or several worktrees.
Different tasks must not share writable worktree paths; the bridge rejects exact
path/symlink duplicates, but this is a configuration check, not a filesystem sandbox.
Agent authors should exclude the identity used by Codex to send assignments, to
avoid echo loops. Reports are external data, never new user authorization.

```sh
python3 scripts/buzz-codex-bridge/bridge.py --config /private/routes.json --state /private/deliveries.sqlite
```

The bridge remains in the foreground. Stop with Ctrl-C. It cleans up its watch
process groups, including descendants of authentication wrappers. Codex queue
commands use the same containment on exit, timeout, or cancellation. No login daemon or scheduled task is installed automatically. Use the
same state DB after restart; a different routing configuration requires a new DB.
The DB and lock are created owner-only. Treat the DB as private conversation data.

## Delivery contract

`buzz messages watch --channel UUID --author PUBKEY --since UNIX_SECONDS` opens a
NIP-42-authenticated WebSocket using the existing Buzz client. It verifies signed
events and channel/author/kind scope, emits stored messages oldest-first, then
`{"type":"ready"}`, then live events. Events use
`{"type":"event","event":{...canonical signed Nostr fields...}}`.
Supported message kinds are 9, 45001, and 45003. No human-name resolution is used.

The bridge journals a report before invoking Codex, and records successful queue
acceptance. Replayed event IDs already recorded as sent are ignored. Reports over 60,000 bytes
are recorded in a terminal `rejected` table and reported on stderr without stopping
other deliveries. Send a smaller summary as a new event; replay skips the rejected
event. Rejected records count toward the journal limit. Queue success
means accepted by Codex, not that an agent finished processing the report.

On timeout/crash between submission and recording success, the row stays `sending`.
Restart refuses to retry an ambiguous submission. Check the destination chat for
that event ID, then explicitly reconcile its row to `sent` if received or `pending`
if confirmed not accepted. Do not delete the DB to get past an error: that loses
both pending reports and duplicate protection. Exactly-once delivery across the
Codex process boundary is not claimed.

Disconnects restart the watcher with exponential backoff, stopping after eight
consecutive short failures. Long-lived idle connections may reconnect after the
watcher's 90-second receive timeout. Replay starts at the task's original boundary
(initial startup minus 60 seconds), not at the newest author-controlled timestamp.
This trades bounded task history for robust deduplication without clock-based gaps.
The relay's retention and history limits still apply. The watch command refuses a
history batch reaching 1,000 events or exceeding 8 MiB; the journal caps at 10,000 events. Use a fresh
task/channel for further work after reconciling a full task. Watch is not an
unlimited event archive and cannot recover messages already removed by retention.

## Verify

```sh
cargo test -p buzz-cli
python3 -m unittest discover -s scripts/buzz-codex-bridge -v
```

The Rust integration tests exercise the actual CLI with NIP-42 authentication,
signed history, EOSE, closure, and tampered events. Python tests cover persistent
replay, ambiguous delivery, routing, literal subprocess arguments, and multiple
worktrees per task.
