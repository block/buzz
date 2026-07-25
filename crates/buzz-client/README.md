# buzz-client

`buzz-client` is the shared authenticated transport library for Buzz relay
consumers. It owns relay URL normalization, Buzz identity validation, NIP-98
HTTP authentication, NIP-42 WebSocket authentication, retries, event
submission, subscriptions, and Blossom media transfer.

Feature-specific event kinds, filters, event builders, and user-interface
policy remain in the consuming application. Use `buzz-core` for shared
protocol types and `buzz-sdk` for typed Buzz event builders.

## Dependency

Buzz client crates are currently supported as Git dependencies, not as
independently published crates. Pin an exact commit:

```toml
[dependencies]
buzz-client = {
  git = "https://github.com/block/buzz",
  rev = "<exact-commit>",
}
```

If a consumer also uses `buzz-core`, `buzz-sdk`, `buzz-ws-client`, or
`buzz-pairing-client` directly, every Buzz Git dependency must use the same
repository URL and exact `rev`. Mixing revisions can create incompatible
copies of shared protocol types.

## Initial public API

The first supported API comprises:

- `BuzzIdentity`, `BuzzClientConfig`, and `RetryPolicy` for construction;
- `BuzzClient` operations for queries, counts, signing, submission,
  subscriptions, authenticated HTTP, and media transfer;
- the documented response, subscription, relay-message, and error types;
- `normalize_relay_ws_url` for consumers that must configure another
  WebSocket-speaking process.

The crate is pre-1.0. Public APIs can still evolve, but transport internals and
test-only dependency injection are intentionally not exposed.
