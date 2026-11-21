# Slack Connect Bridge

An operator-run reference bridge for teams adopting Buzz while customers or
partners still collaborate in Slack Connect.

It mirrors live text messages and threads between explicitly mapped channel
pairs:

```text
Slack Connect channel ←→ bridge bot identities ←→ Buzz channel
```

The bridge is deliberately outside `buzz-relay`. Slack credentials remain in a
separate process, Buzz keeps using signed events, and operators decide exactly
which channel pairs cross the organizational boundary.

## Safety model

- Every route is an explicit `(Slack team ID, Slack channel ID, Buzz channel
  UUID)` mapping. Wildcard or name-based routes do not exist.
- The Slack bot token must belong to the configured Slack workspace.
- Each Slack channel must report `is_ext_shared=true` at startup. Local Slack
  channels are rejected unless `allow_non_shared_channels` is explicitly set.
- `channel_unshared` pauses the corresponding route. `channel_shared` resumes
  it, and `channel_id_changed` follows Slack's private-to-shared ID migration.
- Slack callback signatures use HMAC-SHA256 and a five-minute replay window.
- Slack-origin messages are signed by the bridge's Buzz key and labeled
  `Name · Slack`; Buzz-origin messages are posted by the Slack app and labeled
  `Name · Buzz`. The bridge never impersonates users.
- Buzz text is escaped before entering Slack, so strings such as `<!channel>`
  cannot become cross-organization mass mentions.
- The listener defaults to loopback. Put it behind a TLS reverse proxy; never
  expose plaintext webhook traffic to the internet.
- Secrets come from environment variables, not the JSON mapping file.

## Supported behavior

| Behavior | Support |
| --- | --- |
| New text messages | Two-way |
| Replies whose root was bridged | Two-way, preserved as threads |
| Reply with a pre-bridge root | Mirrored at channel level with a visible warning |
| Slack Connect channel ID changes | Followed durably |
| Slack unshare/reshare | Route paused/resumed |
| Slack webhook retries | Idempotent through durable timestamp ↔ event-ID mappings |
| Buzz reconnect/replay | Bounded replay with durable deduplication |
| Files, edits, deletes, reactions, DMs, huddles | Not in this focused reference slice |

This is a live coexistence bridge, not a history importer. For workspace
migration, see the separate Slack import work in
[`block/buzz#2704`](https://github.com/block/buzz/pull/2704).

## 1. Create the Slack app

1. Copy [`slack-app-manifest.yaml`](slack-app-manifest.yaml).
2. Replace `YOUR_BRIDGE_HOST` with the public TLS hostname that forwards to the
   bridge listener.
3. Create an app from the manifest in the Slack API dashboard.
4. Install it in the workspace from whose perspective you access the Slack
   Connect channels.
5. Invite the app to every mapped shared channel.

The requested scopes are intentionally narrow:

- `chat:write` — mirror Buzz messages;
- `channels:history` / `groups:history` — receive new messages in public and
  private channels the app can access;
- `channels:read` / `groups:read` — validate public/private shared channels and
  receive share lifecycle events;
- `users:read` — display external members' names. Email access is not requested.

Copy the app's bot token and signing secret into your secret manager. Do not put
them in the bridge JSON.

## 2. Prepare the Buzz identity

Generate a dedicated Nostr key for the bridge. On an open relay, the bridge
best-effort self-adds to mapped public channels as a bot. On a private channel,
an owner/admin must add the bridge public key before startup.

For a closed relay, either:

- admit the bridge public key as a standalone relay member; or
- enable NIP-OA (`BUZZ_ALLOW_NIP_OA_AUTH=true`) and provide a `BUZZ_AUTH_TAG`
  valid for the bridge key.

The bridge publishes a kind `0` profile named **Slack Connect Bridge** and uses
ordinary kind `9` channel messages. It adds provenance tags for loop prevention
and auditability; it does not add a new Buzz event kind.

## 3. Configure channel pairs

Copy [`bridge.example.json`](bridge.example.json) and replace every placeholder.
Use IDs, never names: Slack Connect channel names can differ between connected
workspaces.

```json
{
  "listen_addr": "127.0.0.1:3100",
  "state_path": "./slack-connect-bridge-state.json",
  "allow_non_shared_channels": false,
  "replay_lookback_secs": 86400,
  "channels": [
    {
      "slack_team_id": "T0123456789",
      "slack_channel_id": "C0123456789",
      "buzz_channel_id": "018f2f7d-44f4-7df1-a2b5-001122334455"
    }
  ]
}
```

One Buzz channel may map to only one Slack channel. This prevents accidental
fan-out of a message into multiple external organizations.

`replay_lookback_secs` controls how far a restarted process re-reads Buzz
events, up to 30 days. A brand-new bridge starts at the current time and never
backfills old Buzz messages into Slack. On restart, durable mappings suppress
duplicates within the replay window.

Back up `state_path`. It contains no credentials, but it is the durable mapping
between Slack timestamps and Buzz event IDs. If you intentionally remap a Buzz
channel to a different Slack channel, start with a reviewed, separate state
file.

## 4. Run

Activate Buzz's pinned toolchain:

```bash
. ./bin/activate-hermit
```

Then start the bridge:

```bash
BUZZ_SLACK_BRIDGE_CONFIG=/etc/buzz/slack-connect.json \
BUZZ_RELAY_URL=wss://buzz.example.com \
BUZZ_SLACK_BRIDGE_PRIVATE_KEY='nsec1…' \
BUZZ_SLACK_BOT_TOKEN='xoxb-…' \
BUZZ_SLACK_SIGNING_SECRET='…' \
BUZZ_AUTH_TAG='["auth","…"]' \
cargo run --release -p buzz-slack-connect-bridge
```

`BUZZ_AUTH_TAG` is optional on relays where the bridge key can authenticate
directly.

Set `RUST_LOG=buzz_slack_connect_bridge=debug` for additional routing logs.
Logs contain IDs and error codes, but never tokens, signing secrets, or full
message bodies.

## 5. Expose the webhook safely

Terminate TLS at a reverse proxy and forward only `/slack/events` to the
configured listener. `/healthz` returns:

- `200` when Slack validation and the Buzz subscription are ready;
- `503` while the bridge is starting or reconnecting.

Slack callbacks are acknowledged only after the event has been handled. If the
bridge is unavailable, its queue is full, or processing exceeds Slack's
three-second window, it returns `503` so Slack retries. A late successful
attempt remains safe because Slack timestamps and deterministic Buzz events
deduplicate the retry.

Run exactly one bridge process for a given state file. The reference binary
does not provide distributed leader election.

## Manual verification

1. Start with one dedicated test channel pair.
2. Confirm `/healthz` returns `200`.
3. Send a Slack root message and reply; both should appear in one Buzz thread.
4. Send a Buzz root message and reply; both should appear in one Slack thread.
5. Restart the bridge and confirm neither side receives duplicate messages.
6. Temporarily use a non-shared Slack test channel and confirm startup rejects
   it while `allow_non_shared_channels` is `false`.
7. Review the first customer/partner channel's disclosure policy before
   enabling its route.

## Operational notes

- Slack may connect a shared channel to many organizations. Everyone in that
  channel can see bridged Buzz content.
- Slash commands and message actions are workspace-local and are intentionally
  not part of this bridge.
- External Slack profiles may expose less metadata than local profiles. The
  bridge falls back to the Slack user ID if `users.info` cannot resolve a name.
- Slack thread roots created before the bridge has no cross-system ID mapping.
  Their replies are mirrored at channel level with a visible warning instead of
  being silently dropped or attached to the wrong thread.
- File-only Slack messages are ignored in this slice. Text accompanying a file
  is bridged, but the file is not downloaded or re-hosted.
