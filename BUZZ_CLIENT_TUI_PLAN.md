# Buzz Client and TUI Extraction Plan

## Status

Implementation in progress.

- [x] Phase 0: baseline and change-stack preparation
- [x] Phase 1: scaffold `buzz-client`
- [x] Phase 2: HTTP query, count, and pagination
- [x] Phase 3: event submission, WebSocket, and media
- [ ] Phase 4: migrate `buzz-cli`
- [ ] Phase 5: rebase and migrate `buzz-tui`
- [ ] Phase 6: external-consumer hardening
- [ ] Phase 7: extract `buzz-tui` as a standalone project
- [ ] Phase 8: remove the in-tree TUI

Phase 0 baseline recorded on 2026-07-25:

- `cargo test -p buzz-cli -p buzz-ws-client -p buzz-sdk -p buzz-tui`
  passed;
- `cargo build -p buzz-cli -p buzz-tui` passed;
- the existing `tui` change remains preserved at change
  `vulkntyulmou`;
- client work is based on `main`, beginning with the plan change at
  `yqvvznkznynm`;
- no `desktop/` path was modified.

This plan creates a reusable Rust `buzz-client` crate inside the Buzz
repository, validates it with `buzz-cli`, migrates the in-tree `buzz-tui` to
the new client, and only then prepares the TUI for extraction into a standalone
project.

## Hard Constraint: Do Not Touch Desktop

No phase in this plan may modify anything under `desktop/`.

That includes:

- `desktop/src/`;
- `desktop/src-tauri/`;
- Desktop manifests, lockfiles, tests, mocks, generated files, and build
  configuration;
- the Desktop TypeScript WebSocket client;
- the Desktop Tauri relay implementation.

Desktop may be considered conceptually when choosing names or avoiding an
obviously TUI-specific abstraction, but it is not an implementation target or
validation target for this work. Any future Desktop adoption requires a
separate plan and explicit approval.

Before completing every implementation change, verify that no Desktop path is
present:

```bash
jj diff --name-only
```

If any `desktop/` path appears, stop and remove that change from the current
work before continuing.

## Objectives

1. Introduce a focused `crates/buzz-client` library that owns common Buzz relay
   transport and authentication behavior.
2. Preserve the existing `buzz-cli` command-line interface, output formats,
   exit codes, and retry safety.
3. Move `buzz-tui` from its private relay transport implementation to
   `buzz-client` without changing its visible behavior.
4. Keep domain-specific TUI parsing, state, process supervision, persistence,
   and rendering outside `buzz-client`.
5. Make the TUI's eventual standalone dependency boundary explicit and
   revision-pinnable.
6. Keep `buzz-core`, `buzz-sdk`, and `buzz-ws-client` as the lower-level
   protocol crates rather than duplicating them.

## Non-Goals

- Changing relay endpoints or wire protocols.
- Adding a new HTTP endpoint.
- Migrating or editing Desktop.
- Moving TUI UI state or application models into `buzz-client`.
- Rewriting the TUI while migrating its transport.
- Folding `buzz-ws-client` into `buzz-client`.
- Publishing crates to crates.io during the first implementation.
- Extracting the standalone TUI repository before the client-backed TUI passes
  its existing tests.
- Changing `buzz-cli` JSON output, error JSON, or exit codes.

## Current State

### Existing lower-level crates

- `buzz-core` owns protocol constants, shared types, event verification,
  filters, engram support, and other zero-I/O domain behavior.
- `buzz-sdk` owns validated typed event builders and NIP-OA helpers.
- `buzz-ws-client` owns low-level NIP-42 WebSocket authentication, raw message
  parsing, event publication, and relay acknowledgements.

### Existing consumers

`buzz-cli` has a private `BuzzClient` in `crates/buzz-cli/src/client.rs`. It
already implements much of the desired shared behavior:

- URL normalization;
- NIP-98 request signing;
- NIP-OA header handling;
- HTTP query and count requests;
- query pagination;
- stored and ephemeral event submission;
- media upload and download;
- transient retry behavior;
- special handling for non-idempotent moderation events;
- relay error normalization.

`buzz-tui` has a separate `TuiRelayClient` in
`crates/buzz-tui/src/client/mod.rs`. It combines several different concerns:

- generic HTTP and WebSocket transport;
- authentication and event signing;
- Buzz-specific filter construction;
- high-level operations such as channels, messages, workflows, repos, and
  memory;
- event normalization into TUI models;
- live subscription handling;
- media validation and upload;
- TUI-specific caches.

The migration must separate these concerns incrementally. Moving the entire
TUI client module into `buzz-client` would create a TUI-shaped shared library
and is explicitly not the goal.

## Target Architecture

```text
                         +------------------+
                         |    buzz-core     |
                         +------------------+
                                  ^
                                  |
                         +------------------+
                         |    buzz-sdk      |
                         +------------------+
                                  ^
                                  |
+--------------+         +------------------+         +------------------+
|   buzz-cli   | ------> |   buzz-client    | ------> | buzz-ws-client   |
+--------------+         +------------------+         +------------------+
                                  ^
                                  |
                         +------------------+
                         |    buzz-tui      |
                         +------------------+
```

The dependency arrows show library dependencies, not ownership:

- `buzz-client` uses `buzz-core`, `buzz-sdk`, and `buzz-ws-client`.
- `buzz-cli` and `buzz-tui` use `buzz-client`.
- `buzz-tui` may continue depending directly on `buzz-core` and `buzz-sdk`
  where it uses domain constants, decryption, or event builders directly.
- `buzz-tui` should no longer depend directly on `buzz-ws-client` once its live
  subscription migration is complete.
- `buzz-acp` and `buzz-dev-mcp` remain runtime executables, not
  `buzz-client` dependencies.

## Proposed `buzz-client` Responsibility

`buzz-client` should own:

- relay HTTP and WebSocket URL normalization;
- parsed client credentials and NIP-OA auth-tag injection;
- NIP-98 request signing, including a nonce for replay protection;
- authenticated `POST /query`;
- authenticated `POST /count`;
- authenticated `POST /events`;
- stored-event pagination using the relay's composite cursor;
- stored event submission responses;
- safe retry classification;
- ambiguous-delivery handling for non-idempotent commands;
- authenticated Blossom upload and download transport;
- relay information retrieval when needed for transport limits;
- ephemeral WebSocket event publication;
- generic authenticated subscriptions built on `buzz-ws-client`;
- typed transport and protocol errors.

`buzz-client` should not own:

- CLI argument parsing, printing, output normalization, or exit codes;
- TUI models such as `Channel`, `Message`, `Workflow`, `Reminder`, or
  `RepoProject`;
- feature-specific filter construction unless the filter is part of a generic
  transport primitive;
- TUI caches or read-state behavior;
- TUI identity files, workspace files, or keyring policy;
- ACP process supervision;
- moderation UI behavior;
- Desktop code or Desktop-specific policy.

## Initial Public API Shape

The exact Rust spelling can evolve during implementation, but the first API
should remain small and explicit.

```rust
pub struct BuzzClientConfig {
    pub relay_url: String,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    pub retry_policy: RetryPolicy,
}

pub struct BuzzIdentity {
    keys: nostr::Keys,
    auth_tag: Option<nostr::Tag>,
    auth_tag_json: Option<String>,
}

pub struct BuzzClient {
    // Private fields.
}

impl BuzzClient {
    pub fn new(
        config: BuzzClientConfig,
        identity: BuzzIdentity,
    ) -> Result<Self, ClientError>;

    pub fn with_http_client(
        config: BuzzClientConfig,
        identity: BuzzIdentity,
        http: reqwest::Client,
    ) -> Result<Self, ClientError>;

    pub fn public_key(&self) -> nostr::PublicKey;
    pub fn relay_http_url(&self) -> &str;
    pub fn relay_ws_url(&self) -> &str;
    pub fn sign_event(
        &self,
        builder: nostr::EventBuilder,
    ) -> Result<nostr::Event, ClientError>;

    pub async fn query_events(
        &self,
        filters: &[serde_json::Value],
    ) -> Result<Vec<nostr::Event>, ClientError>;

    pub async fn query_values(
        &self,
        filters: &[serde_json::Value],
    ) -> Result<Vec<serde_json::Value>, ClientError>;

    pub async fn query_paginated(
        &self,
        filter: serde_json::Value,
        limit: Option<u32>,
    ) -> Result<Vec<serde_json::Value>, ClientError>;

    pub async fn count(
        &self,
        filters: &[serde_json::Value],
    ) -> Result<u64, ClientError>;

    pub async fn submit_event(
        &self,
        event: nostr::Event,
    ) -> Result<SubmitEventResponse, ClientError>;

    pub async fn publish_ephemeral(
        &self,
        event: nostr::Event,
    ) -> Result<SubmitEventResponse, ClientError>;

    pub async fn subscribe(
        &self,
        subscription_id: &str,
        filters: &[nostr::Filter],
    ) -> Result<RelaySubscription, ClientError>;
}
```

Do not add a public method until an immediate CLI or TUI migration needs it.
All public APIs require documentation.

### Identity rules

`BuzzIdentity` should parse and validate the optional NIP-OA auth tag once.
It should retain:

- the signing keys;
- the parsed `nostr::Tag` used for event injection and WebSocket AUTH;
- the original or canonical JSON used for the `x-auth-tag` HTTP header.

All normal event signing should pass through one method that guarantees:

- no caller-supplied duplicate `auth` tag;
- exactly one injected auth tag when configured;
- no injected tag when one is not configured.

Operations that intentionally carry a content-level `auth` tag must use a
clearly named exceptional method, not silently bypass the invariant.

### Error model

Use a typed error enum rather than CLI-formatted strings:

```rust
pub enum ClientError {
    InvalidUrl(String),
    InvalidKey(String),
    InvalidAuthTag(String),
    Signing(String),
    Network(reqwest::Error),
    WebSocket(buzz_ws_client::WsClientError),
    Relay {
        status: u16,
        message: String,
        retry_after: Option<Duration>,
    },
    Rejected {
        event_id: String,
        message: String,
    },
    Protocol(String),
    Serialization(serde_json::Error),
    Timeout,
    DeliveryUnknown {
        event_id: String,
        reason: String,
    },
}
```

The precise variants may change, but callers must be able to distinguish:

- local validation;
- network failure;
- relay rejection;
- rate limiting;
- malformed protocol responses;
- timeouts;
- ambiguous delivery of non-idempotent operations.

`buzz-cli` remains responsible for mapping these variants to its existing JSON
errors and exit codes. `buzz-tui` remains responsible for mapping them to
status-line and panel messages.

## Phase 0: Baseline and Change Stack

### Goal

Record current behavior and arrange the JJ history so client work is based on
`main`, while preserving the existing `tui` change.

### Work

1. Inspect the current graph and the exact `main`/`tui` divergence.
2. Keep the existing `tui` change intact until the shared client and CLI
   migration are ready.
3. Start the client work as new changes based on `main`.
4. Use a small stack rather than one large implementation change:

   ```text
   main
     client-01: scaffold and authentication
       client-02: HTTP query and submission
         client-03: CLI migration
           tui change rebased here
             tui-client-01: HTTP migration
               tui-client-02: WebSocket and cleanup
   ```

5. Before rebasing `tui`, inspect the source revset and destination. Do not
   rewrite unrelated bookmarks.

Illustrative JJ commands, to be adjusted after inspecting the graph:

```bash
jj log
jj new main
jj describe -m "feat(client): add shared Buzz client foundation"

# Only after the client and CLI changes are complete and their revsets have
# been inspected:
jj rebase -s tui -d <client-stack-tip>
```

### Baseline validation

Run from the repository's Nix environment:

```bash
cargo test -p buzz-cli
cargo test -p buzz-ws-client
cargo test -p buzz-sdk
cargo test -p buzz-tui
cargo build -p buzz-cli -p buzz-tui
```

Capture any pre-existing failures in the relevant change description rather
than weakening later validation.

### Exit criteria

- The starting graph and base revisions are known.
- Baseline CLI and TUI results are recorded.
- The existing TUI change is preserved.
- No `desktop/` path is modified.

## Phase 1: Scaffold `buzz-client`

### Goal

Create a compilable library crate with configuration, identity, URL, and error
types but no consumer migrations.

### Work

- [x] Add `crates/buzz-client` to the root Cargo workspace.
- [x] Add dependencies only as required:
   - `buzz-core`;
   - `buzz-sdk`;
   - `buzz-ws-client`;
   - `nostr`;
   - `reqwest`;
   - `serde` and `serde_json`;
   - `tokio`;
   - `url`;
   - `base64`, `sha2`, `hex`, and `uuid` for NIP-98;
   - `thiserror`;
   - `bytes` and `infer` only when media support is added.
- [x] Add crate-level documentation, `#![deny(unsafe_code)]`, and
   `#![warn(missing_docs)]`.
- [x] Implement:
   - `BuzzClientConfig`;
   - `BuzzIdentity`;
   - `BuzzClient`;
   - `ClientError`;
   - HTTP/WS URL normalization.
- [x] Extract NIP-98 signing from the CLI implementation.
- [x] Preserve nonce generation so repeated requests in the same second do not
   generate replay-identical NIP-98 events.
- [x] Parse and validate the NIP-OA auth tag at construction time.
- [x] Add tests for:
   - HTTP-to-WebSocket and WebSocket-to-HTTP normalization;
   - trailing slash handling;
   - loopback URLs;
   - malformed URLs;
   - invalid keys;
   - invalid auth tags;
   - auth-tag injection count;
   - NIP-98 method, URL, payload hash, and nonce tags.

### Exit criteria

```bash
cargo fmt --check
cargo test -p buzz-client
cargo clippy -p buzz-client --all-targets -- -D warnings
```

- The crate builds without CLI or TUI dependencies.
- Authentication invariants are tested.
- No consumer behavior has changed.
- No `desktop/` path is modified.

## Phase 2: HTTP Query, Count, and Pagination

### Goal

Move the first complete transport slice into `buzz-client`.

### Work

- [x] Implement authenticated `POST /query`.
- [x] Implement authenticated `POST /count`.
- [x] Support multiple OR-ed Nostr filters.
- [x] Confirm both current consumers require the JSON-value return path; defer
   a typed-event path until a consumer needs it.
- [x] Move the CLI's composite pagination behavior into the client:
   - page size bounds;
   - `(until, before_id)` cursor advancement;
   - duplicate boundary prevention;
   - finite limit and query-all modes;
   - deterministic termination on short pages.
- [x] Implement structured relay error parsing.
- [x] Parse `retry in Ns` hints with a defensive maximum.
- [x] Preserve raw relay messages inside typed errors where callers need them for
   compatible user-facing output.
- [x] Add an in-process stub relay test suite covering:
   - successful query;
   - multi-filter serialization;
   - successful count;
   - pagination across equal timestamps;
   - malformed success bodies;
   - 401/403 authentication failures;
   - 429 with and without retry hints;
   - 502/503/504 transient responses;
   - request timeouts;
   - response body transfer failure where practical.

### Exit criteria

- Query and count behavior is fully tested without external infrastructure.
- Pagination cannot loop or skip equal-timestamp events.
- The client exposes typed errors rather than CLI output.
- No TUI migration has started yet.
- No `desktop/` path is modified.

## Phase 3: Event Submission, WebSocket, and Media

### Goal

Complete the transport surface required by both CLI and TUI.

### Event submission work

- [x] Implement stored event submission through `POST /events`.
- [x] Return a typed response containing:
   - event ID;
   - accepted flag;
   - relay message.
- [x] Reject an `accepted: false` response with a typed rejection.
- [x] Port the CLI's operation-safety classification:
   - retry idempotent stored event submissions on safe transient failures;
   - do not blindly retry non-idempotent moderation command kinds;
   - return `DeliveryUnknown` when the server may have executed a
     non-idempotent operation before the response was lost;
   - permit retry only when the failure proves the relay did not receive the
     operation.
- [x] Test every retry category with a stub relay that records request counts.

### WebSocket work

- [x] Keep `buzz-ws-client` as the wire-level implementation.
- [x] Add a `buzz-client` facade for:
   - authenticated connection;
   - NIP-OA auth-tag forwarding;
   - ephemeral event publication;
   - subscription request creation;
   - subscription event iteration;
   - `EOSE`, `CLOSED`, `NOTICE`, `AUTH`, and `OK` handling;
   - bounded disconnect.
- [x] Do not duplicate WebSocket frame parsing from `buzz-ws-client`.
- [x] Ensure subscriptions can be cancelled and do not leak tasks.
- [x] Test authentication, subscription, event delivery, close, timeout, and
   rejection against an in-process WebSocket relay.

### Media work

- [x] Move generic authenticated upload/download transport into `buzz-client`.
- [x] Keep caller-facing file selection and UI behavior outside the client.
- [x] Retain:
   - NIP-98/Blossom signing;
   - SHA-256 calculation;
   - safe media path validation;
   - MIME and size information;
   - bounded upload timeouts;
   - legacy endpoint fallback only if still required by current behavior.
- [x] Treat MIME allowlists as product policy retained by callers; the shared
   client validates MIME syntax and owns transport metadata only:
   - protocol-required validation belongs in `buzz-client`;
   - CLI- or TUI-specific restrictions remain with the caller.

### Exit criteria

```bash
cargo test -p buzz-client
cargo test -p buzz-ws-client
cargo clippy -p buzz-client --all-targets -- -D warnings
```

- All shared transport capabilities needed by the CLI and TUI exist.
- Retry safety is pinned by tests.
- WebSocket code delegates wire handling to `buzz-ws-client`.
- No `desktop/` path is modified.

## Phase 4: Migrate `buzz-cli`

### Goal

Validate `buzz-client` with the smaller existing consumer while preserving the
CLI contract.

### Work

1. Add `buzz-client` as a dependency of `buzz-cli`.
2. Replace the private CLI transport implementation in vertical slices:
   1. client construction and credentials;
   2. query and count;
   3. pagination;
   4. stored event submission;
   5. ephemeral WebSocket publication;
   6. media upload/download.
3. Keep in `buzz-cli`:
   - argument parsing;
   - command dispatch;
   - feature-specific filter and event construction;
   - output normalization;
   - stdout/stderr formatting;
   - exit-code mapping;
   - CLI environment-variable semantics.
4. Add a conversion from `buzz_client::ClientError` to the existing CLI error
   categories.
5. Remove old private helpers only after the corresponding command tests use
   the shared implementation.
6. Do not combine the migration with unrelated command refactoring.

### Compatibility tests

Pin the following before and after migration:

- help and usage behavior;
- JSON and compact output;
- sig stripping;
- create-response ID augmentation;
- relay/network/auth/error JSON;
- exit codes 0 through 5;
- retry behavior;
- non-idempotent delivery-unknown behavior;
- environment-variable timeout overrides;
- authenticated stored writes;
- ephemeral presence and agent operations;
- media upload and download.

### Exit criteria

```bash
cargo test -p buzz-client -p buzz-cli
cargo build -p buzz-cli
cargo clippy -p buzz-client -p buzz-cli --all-targets -- -D warnings
```

- `buzz-cli` no longer contains a second generic relay transport.
- Its public behavior remains unchanged.
- The shared client has a real consumer independent of the TUI.
- No `desktop/` path is modified.

## Phase 5: Rebase and Migrate `buzz-tui`

### Goal

Move the TUI to `buzz-client` incrementally without rewriting its application
behavior.

### Preparation

1. Inspect the client stack and `tui` source change with `jj log`.
2. Rebase only the intended `tui` change onto the completed client/CLI stack.
3. Resolve root workspace, lockfile, Justfile, and flake conflicts carefully.
4. Verify that the rebased TUI still passes its baseline before changing its
   client implementation.

### Slice 1: Construction, identity, and signing

1. Replace duplicate URL and credential parsing with `BuzzClient`.
2. Preserve the TUI's identity-file and keyring behavior outside the client.
3. Route normal signing through the shared auth-tag invariant.
4. Keep managed-agent identities explicit; do not silently substitute the
   logged-in user's identity.

Validation:

- identity import and generation tests;
- auth-tag verification tests;
- public key display;
- workspace relay switching;
- managed-agent signing tests.

### Slice 2: Generic HTTP queries

1. Route `query_values` and generic relay information calls through
   `buzz-client`.
2. Keep feature filters and response normalization in the TUI.
3. Migrate one feature family at a time:
   1. profiles and contacts;
   2. channels and messages;
   3. reactions and read state;
   4. workflows;
   5. repos, issues, patches, and pull requests;
   6. notes and social;
   7. reminders;
   8. memory and agent metrics;
   9. moderation and relay administration.
4. After each family, run its focused tests before moving on.

### Slice 3: Stored writes

1. Replace the TUI's generic submit/sign helpers with `buzz-client`.
2. Continue using `buzz-sdk` builders for typed writes.
3. Preserve the relay's command-response parsing behavior.
4. Preserve event-size checks using relay information.
5. Verify edits, deletes, reactions, membership commands, workflows,
   moderation commands, and app-data writes.

### Slice 4: Media

1. Move upload/download transport to `buzz-client`.
2. Keep TUI path input, composer attachment state, progress/status messages,
   and rendering local.
3. Preserve MIME, size, timeout, and relay-message-limit behavior.

### Slice 5: Live WebSocket subscriptions

1. Adapt `live.rs` to the `buzz-client` subscription facade.
2. Preserve TUI-specific subscription IDs, refresh logic, active-channel
   selection, and event-to-model normalization.
3. Migrate:
   - workspace subscriptions;
   - active-channel subscriptions;
   - typing and presence;
   - mentions;
   - app-data changes;
   - custom emoji and contact changes.
4. Preserve reconnect and polling fallback behavior.
5. Remove the direct `buzz-ws-client` dependency only when no TUI source file
   imports it.

### Slice 6: Delete duplicate transport

After all call sites use `buzz-client`:

1. Remove duplicate NIP-98 signing.
2. Remove duplicate URL normalization.
3. Remove duplicate generic HTTP request/response code.
4. Remove duplicate upload/download transport.
5. Remove duplicate generic WebSocket connection code.
6. Retain in the TUI:
   - feature operations;
   - filters;
   - parsers and normalization;
   - TUI model definitions;
   - caches;
   - reminders and read-state semantics;
   - UI-facing error messages.
7. Consider splitting the remaining `client/mod.rs` by feature, but do not make
   that cleanup a prerequisite for transport migration.

### TUI validation

```bash
cargo fmt --check
cargo test -p buzz-client -p buzz-tui
cargo build -p buzz-tui
cargo clippy -p buzz-client -p buzz-tui --all-targets -- -D warnings
```

Also run focused stub-relay tests for:

- authenticated startup;
- channel history and paging;
- live event delivery;
- message send and relay rejection;
- command response parsing;
- file upload;
- graceful subscription close;
- graceful ACP shutdown unaffected by client changes.

### Exit criteria

- TUI-visible behavior is unchanged.
- Generic transport is provided by `buzz-client`.
- TUI-specific models and behavior remain in `buzz-tui`.
- `buzz-tui` has no direct `buzz-ws-client` dependency.
- No `desktop/` path is modified.

## Phase 6: External-Consumer Hardening

### Goal

Make the Buzz crates usable from a standalone repository before moving the
TUI.

### Work

- [x] Give `buzz-client` complete package metadata:
   - description;
   - license;
   - correct repository;
   - Rust version;
   - documentation.
- [x] Correct inherited repository metadata needed by the affected shared crates.
- [ ] Ensure `buzz-client`, `buzz-core`, `buzz-sdk`, and `buzz-ws-client` can be
   resolved together from one exact Buzz Git revision.
- [x] Avoid path references that escape each crate's expected workspace.
- [x] Run package checks without publishing.
- [x] Document the supported dependency method:

   ```toml
   buzz-client = {
     git = "https://github.com/block/buzz",
     rev = "<exact-commit>",
   }
   ```

- [ ] Pin all Buzz Git dependencies to the same revision to prevent incompatible
   duplicate protocol versions.
- [x] Decide which public types are stable enough for a first version. Reduce the
   API rather than prematurely guaranteeing unused methods.
- [ ] Add a minimal external-consumer fixture outside the Cargo workspace that:
   - depends on the exact Git revision;
   - constructs a client;
   - builds a filter;
   - performs a compile-only query call;
   - compiles without workspace path assumptions.

### Exit criteria

- The shared crate graph works from outside the Buzz workspace.
- Public APIs have documentation.
- The external fixture compiles.
- No crates.io publication is required.
- No `desktop/` path is modified.

## Phase 7: Extract `buzz-tui` as a Standalone Project

### Goal

Move the already client-backed TUI into its own repository without changing
behavior.

### Work

1. Create a standalone Cargo project from `crates/buzz-tui`.
2. Replace all `workspace = true` package and dependency fields with explicit
   values.
3. Add exact-revision Git dependencies for:
   - `buzz-client`;
   - `buzz-core`, if still used directly;
   - `buzz-sdk`, if still used directly.
4. Add a standalone `Cargo.lock`.
5. Port only the TUI-specific portions of:
   - the root Justfile;
   - the Nix flake;
   - CI configuration;
   - release packaging.
6. Give the project its own:
   - README;
   - license;
   - contribution instructions;
   - changelog;
   - version;
   - issue tracker;
   - release artifacts.
7. Preserve the external `--acp-bin` and `--mcp-command` configuration.
8. Decide how releases obtain compatible sidecars:
   - bundle pinned `buzz-acp` and `buzz-dev-mcp` binaries; or
   - install them separately from the same Buzz revision and validate their
     versions at startup.
9. Test Linux, macOS, and Windows keyring feature combinations.
10. Verify a headless `--no-default-features` build.

### Standalone validation

```bash
cargo fmt --check
cargo test
cargo build --release
cargo clippy --all-targets -- -D warnings
```

Run smoke tests against:

- a local relay built from the pinned Buzz revision;
- authentication with and without a NIP-OA tag;
- one query;
- one stored write;
- one live subscription;
- one ACP harness start and graceful stop.

### Exit criteria

- The standalone project builds without a Buzz checkout.
- It depends on a single exact Buzz revision.
- Its release packaging accounts for runtime sidecars.
- Behavior matches the final in-tree client-backed TUI.

## Phase 8: Remove the In-Tree TUI

This phase begins only after the standalone repository and release build are
verified.

### Work

1. Remove `crates/buzz-tui` from the Buzz workspace.
2. Remove only TUI-specific root Justfile commands.
3. Remove only TUI-specific flake packages and development helpers.
4. Update Buzz documentation to link to the standalone project.
5. Keep `buzz-client`, `buzz-core`, `buzz-sdk`, and `buzz-ws-client` in Buzz.
6. Verify the root lockfile changes contain only the expected TUI dependency
   removal.
7. Retire the `tui` bookmark only after its history is preserved and the
   standalone repository is confirmed.

### Exit criteria

```bash
cargo fmt --check
cargo test -p buzz-client -p buzz-cli
cargo build -p buzz-cli
jj diff --name-only
jj status
jj log
```

- Buzz no longer contains the TUI application.
- Buzz continues to own the shared client and protocol crates.
- CLI behavior remains unchanged.
- The standalone TUI release is reproducible.
- No `desktop/` path was modified during the project.

## Test Strategy

### Unit tests

Use pure tests for:

- URL normalization;
- auth-tag parsing and injection;
- NIP-98 event construction;
- payload hashing;
- retry-hint parsing;
- retry classification;
- cursor advancement;
- response parsing;
- error conversion.

### Stub HTTP relay

Use an in-process Axum server to test:

- request paths, methods, headers, and bodies;
- NIP-98 signatures;
- NIP-OA headers;
- pagination;
- status handling;
- malformed bodies;
- delayed responses;
- disconnects;
- retry request counts;
- ambiguous non-idempotent delivery.

### Stub WebSocket relay

Use an in-process WebSocket server to test:

- NIP-42 challenges;
- auth success and rejection;
- subscription requests;
- event delivery;
- EOSE;
- CLOSED and NOTICE;
- event OK responses;
- timeouts;
- clean disconnect.

### Consumer contract tests

`buzz-cli` tests pin command behavior. `buzz-tui` tests pin feature behavior.
Shared client tests pin transport behavior. Do not rely only on end-to-end
tests, because failures must identify which contract changed.

### Full validation

Before any PR, run the applicable repository quality gates from the Nix
environment. Because Desktop is out of scope, do not make code changes to
Desktop in response to unrelated pre-existing Desktop failures. Report such
failures separately.

## Implementation Guardrails

1. No `unsafe` code.
2. No new production `unwrap()` or `expect()`.
3. Document every new public API.
4. Do not expose `reqwest::Response` or WebSocket implementation types unless
   a consumer genuinely needs them.
5. Do not leak CLI formatting into shared errors.
6. Do not leak Ratatui or TUI models into `buzz-client`.
7. Do not add feature-specific high-level methods merely to shorten a TUI call
   site.
8. Preserve non-idempotent retry safety.
9. Pin Git dependencies to an exact revision, never a moving branch.
10. Keep each migration change buildable and testable.
11. Inspect JJ revsets before rebasing or rewriting.
12. Preserve unrelated working-copy changes.
13. Do not modify any `desktop/` path.

## Risk Register

### Risk: the client becomes TUI-specific

Mitigation:

- migrate the CLI first;
- expose transport operations rather than feature screens;
- keep TUI models and filters local;
- require two consumer call sites before generalizing convenience APIs.

### Risk: retry changes duplicate non-idempotent commands

Mitigation:

- port the CLI's delivery-safety distinctions before migrating writes;
- test request counts and dropped-response scenarios;
- surface `DeliveryUnknown` instead of guessing.

### Risk: auth tags are duplicated or omitted

Mitigation:

- parse credentials once;
- centralize normal event signing;
- count auth tags after signing;
- test HTTP, WebSocket, owner, and managed-agent identities separately.

### Risk: pagination skips equal-timestamp events

Mitigation:

- preserve the relay's composite cursor;
- add multi-page fixtures with identical timestamps and ordered IDs;
- assert complete, duplicate-free results.

### Risk: a large TUI migration obscures regressions

Mitigation:

- migrate one feature family at a time;
- run focused tests after each slice;
- delete old transport only after all call sites have moved;
- avoid simultaneous UI refactoring.

### Risk: standalone builds accidentally require the Buzz workspace

Mitigation:

- add an external-consumer fixture before extraction;
- replace every workspace-inherited manifest field;
- build the standalone project from a clean directory in CI.

### Risk: sidecar versions drift

Mitigation:

- build or install `buzz-acp` and `buzz-dev-mcp` from the same pinned Buzz
  revision;
- record compatible revisions in releases;
- add startup version diagnostics if Buzz binaries expose machine-readable
  versions.

### Risk: Desktop is modified accidentally

Mitigation:

- make Desktop a hard non-goal;
- inspect `jj diff --name-only` in every phase;
- reject any change containing `desktop/`;
- do not run automated rewrite commands over the whole repository.

## Completion Definition

The project is complete when:

1. `buzz-client` is a documented shared crate in Buzz.
2. `buzz-cli` uses it without observable CLI regressions.
3. `buzz-tui` uses it for generic HTTP and WebSocket transport.
4. TUI-specific domain and UI behavior remains outside the shared crate.
5. The client-backed TUI builds as a standalone project against one exact Buzz
   revision.
6. Runtime sidecars have a reproducible compatibility and packaging story.
7. The in-tree TUI is removed only after standalone verification.
8. No Desktop file was modified.
