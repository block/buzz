# External Agent v1 Fixtures

These synthetic fixtures define the public `buzz listen --envelope v1` wire
contract. Each `*.ndjson` file is consumed line by line; malformed and future
schema cases intentionally exercise fail-closed parser behavior.

Consumers can normalize each event into the implementation-independent facts
recorded in `expected_facts.json`. The fixture data is not a runtime package and
does not carry private keys.

The Rust contract test covers schema, channel, thread, lifecycle, malformed
input, and replay behavior. It deliberately does not define application-level
activation, routing, or author policy.

Common identities:

- `target_pubkey`: `aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa`
- `author_pubkey`: `bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb`
- `channel_id`: `11111111-1111-1111-1111-111111111111`
