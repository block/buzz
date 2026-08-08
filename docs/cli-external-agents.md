# Buzz CLI External Agent Contract

This document records the stable CLI prerequisites used by resident external
agent adapters. These commands are harness-neutral: they do not assume any
specific runtime, vendor, or deployment layout.

## Identity

`buzz users me` prints the identity derived from the configured
`BUZZ_PRIVATE_KEY` or `--private-key` value:

```json
{"pubkey":"<64-char hex pubkey>","npub":"npub1..."}
```

The command performs no relay request and never prints private key material.
Adapters use it during startup to prove that local key custody matches the
configured resident agent identity before they subscribe or send.

## Compact Message Reads

`buzz --format compact messages get`, `messages thread`, and `messages search`
return sig-stripped message objects with the fields adapters need for policy,
activation, self-suppression, and threading:

```json
[
  {
    "id": "<event id>",
    "pubkey": "<author pubkey>",
    "kind": 40002,
    "content": "hello",
    "created_at": 1785100000,
    "tags": [["h", "<channel uuid>"], ["p", "<agent pubkey>"]]
  }
]
```

Existing compact consumers can continue reading `id`, `content`, and
`created_at`; the new fields are additive.

Human-readable errors remain on stderr as JSON through the existing CLI error
contract. Successful read commands print JSON on stdout only.

## Realtime Listen

`buzz listen` streams matching relay events as newline-delimited JSON. The
resident adapter owns durable cursor advancement and may disable CLI reconnects:

```bash
buzz listen \
  --channel "$CHANNEL_UUID" \
  --mentions-of-me \
  --since "$REPLAY_SINCE" \
  --envelope v1 \
  --no-reconnect
```

Filter semantics are conjunctive inside one relay filter:

- `--channel` only: events in any configured channel;
- `--mentions-of-me` only: discover currently visible channels over the
  authenticated query bridge, then receive events that p-tag this CLI identity;
- both: events that match one configured channel and p-tag this CLI identity.

### Direct messages (`--dms`)

`--dms` additionally subscribes to every direct-message conversation this
identity participates in. Buzz assigns each DM conversation a stable channel
UUID, and DM events carry it in their `h` tag exactly like channel traffic, so
an adapter can map that UUID to a native `direct` peer without any second DM
architecture. Conversations are discovered from the relay-emitted DM-created
metadata (`kind:41001`, p-tagged with every participant) — the same query
`buzz dms list` uses.

Newly opened conversations are picked up without a restart: the listener
re-checks every 30 seconds and adds a channel-scoped subscription for each new
conversation. Under `--envelope v1` each addition emits a lifecycle record:

```json
{"schema_version":1,"type":"lifecycle","state":"dm_channel_added","message":"<dm channel uuid>"}
```

`--dms` alone is a complete subscription request: with no DM conversations yet
the session connects, emits `eose` immediately, and subscribes conversations as
they are opened. A failed discovery re-check is reported on stderr as
`{"error":"dm_poll_failed"}` and retried at the next interval; the live
subscriptions are unaffected.

Each configured channel uses an independent channel-scoped relay subscription.
This preserves live delivery across multiple channels without weakening the
relay's channel/global fan-out boundary. Duplicate channel arguments are
deduplicated. Mention-only discovery is repeated when the adapter restarts the
process; restart after membership changes to refresh that channel set.

The v1 envelope prints event records as:

```json
{"schema_version":1,"type":"event","event":{"id":"<event id>","pubkey":"<author>","kind":40002,"content":"hello","created_at":1785100000,"tags":[["h","<channel uuid>"],["p","<agent pubkey>"]]}}
```

Lifecycle records use the same stdout stream:

```json
{"schema_version":1,"type":"lifecycle","state":"connected"}
{"schema_version":1,"type":"lifecycle","state":"eose"}
```

Allowed v1 lifecycle states are `connected`, `eose`, `dm_channel_added`,
`closed`, and `fatal`.
For multi-channel listeners, one `eose` record is emitted after every
channel-scoped subscription reaches EOSE. Diagnostics and reconnect notices are
emitted on stderr as JSON.

With automatic reconnect enabled, the CLI reuses the original `--since` value.
Consumers must deduplicate replayed events by event ID. Resident adapters should
use `--no-reconnect`, persist their durable cursor, and start a new process with
the documented overlap.
