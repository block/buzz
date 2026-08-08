# External Agent v1 Fixtures

These fixtures define the Buzz-owned external-agent ingress contract used by
resident adapters. Each `*.ndjson` file is consumed line by line. Most files
contain versioned `buzz listen --envelope v1` records; malformed and future
schema cases intentionally exercise fail-closed parser behavior.

Adapters should normalize each event into the facts recorded in
`expected_facts.json` and apply policy locally. The fixture data is not a shared
runtime package and does not carry private keys.

The Rust contract test contains the reference normalization order. Schema,
channel, and thread validation fail closed before activation; self-authored and
unauthorized events are ignored before a conversation lane is created. Replay
duplicates are byte-identical and use the event ID as their idempotency key.

Common identities:

- `agent_pubkey`: `aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa`
- `owner_pubkey`: `bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb`
- `allowlisted_pubkey`: `cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc`
- `non_owner_pubkey`: `dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd`
- `channel_id`: `11111111-1111-1111-1111-111111111111`
