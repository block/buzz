# NIP-BW P2M source handoff

This is implementation evidence, not a revision of NIP-BW and not an activation
or deployment authorization. Core has no I/O, wall clock or dispatch. SDK drafts
are unsigned. `buzz bw` runs before the CLI's relay/key setup. Productive BW
publication returns an error unconditionally; there is no activation flag.
The activation prerequisite remains complete P3C readback.

## Contract binding

- P1 baseline: `529136c39aa42db64af61f811fc33dac73d16dd1`
- Fixture SHA-256 (unchanged): `b489fb188b45e073e81783e8d591f24891a9c73bc56d8d2fc56a881606640e8a`
- Current document SHA-256: `86184a747dda4692c64db65c216f4cdb6dfe69e765755abd1eabba893df55d29`
- Host counterpart supplied by Jari: `b5efd8ec4368e99d8117e29d77dcd8ce05d7ebcc`.
  Its independent PASS and Relay readback are supplied evidence. Its full suite
  reportedly has 561 passes and 35 pre-existing failures. No Host code is read or
  changed by this implementation. The cross-implementation export comparison is
  still a separate handoff, not a claimed joint P2 PASS.

The compiled closed `bw/schema.json` contains only the unchanged `schemas`,
`types` and `arrays` from P1. Tests compare those tables byte-semantically and pin
the fixture and current document digests. No fixture event, test expectation or
alias is compiled into the consumer. Test-only inputs use the existing public
fictitious test keys.

## Public namespace recheck: 2026-09-09 UTC

A fresh read of the official [NIP index](https://github.com/nostr-protocol/nips/blob/master/README.md),
[kind registry](https://github.com/nostr-protocol/registry-of-kinds/blob/master/schema.yaml)
and [Buzz constants](https://github.com/block/buzz/blob/main/crates/buzz-core/src/kind.rs)
found no assignment of 46100. This is not a global reservation. Local source adds
`KIND_BUZZ_WORKFLOW_RECORD` to the workflow family and the duplicate-detection
inventory; it remains a regular event and is not an engine command/trigger.

## Offline API and commands

`buzz_core::bw::Consumer::new(Trust, Evidence, now)` starts a fresh history.
`ingest(raw_utf8_bytes)` returns actual `event_id`, `outcome`, `stage`, `code` and
`projection`. `observe(evidence, now)` replaces observations; `projection()` then
recomputes from the entire supplied history. `archived_inputs()` retains original
input bytes, including rejected inputs. A rejected signature cannot poison an ID
for a later genuine event. This API accepts no implicit key or transport.

A validation file contains:

```json
{
  "now": 1800001000,
  "trust": {"community": "https://relay.example.invalid", "repo": "30617:<owner>:<slug>", "owner": "<pubkey>"},
  "external": {
    "git_readbacks": [], "git_ancestry": [], "provider_readbacks": [],
    "downloads": [], "host_authorization": {"allowed": false, "current_policy": "<id>"}
  },
  "events": []
}
```

Events are full signed objects or strings containing original raw event JSON.
String inputs also allow an offline caller to inspect malformed wire JSON without
losing lexical information in a containing document. External observations are
trusted caller inputs, never values inferred from an event's signed claims.

```sh
buzz bw validate --input history.json
buzz bw show --input history.json
buzz bw dry-run --input unsigned-draft.json
buzz bw corpus --input docs/nips/NIP-BW.fixtures.json --output p2m-results.json
buzz bw publish  # always fails: full P3C readback required
```

Dry-run input has only `kind`, `tags`, `content`. SDK `bw::record(RecordType, tags,
content)` inserts the typed discriminator and checks the same closed shapes.
`bw::Draft::new` also supports the separate artifact, root and existing assignment
profiles. It does not sign or return a transport. Existing standard SDK builders
retain their existing behavior. `verify_readback` is pure: all seven signed fields
must match and both ID and signature must verify. A private test transport checks
acceptance, missing readback, mismatched readback and transport rejection.

`bw corpus` verifies the fixture digest and executes every case from empty history
with its explicit trust/evidence/time, without consulting `expected` or `crypto`.
It exports computed results in corpus case/step order. JSON object maps and ID
sets are sorted. The Rust corpus test separately checks all six outcomes, stages,
codes and every specified partial projection, including `artifact_verdicts` and
`issue_fields`. Unspecified projection keys remain outside the P1 comparison.

Projection maps preserve issue acceptance separately from concrete artifact
feedback and technical conflicts. `sets`/`handoffs` are keyed by real freeze IDs;
`set`/`handoff` provide the latest authored set's display summary, without resolving
competing claims. `dispatch_count` is an offline count of eligible unique requests,
not evidence of a dispatch. There is no actual dispatcher. Host authorization is
reported separately and cannot create a historical BW role or human acceptance.

## Validation commands

Use the repository's activated Hermit toolchain. Normal debug/test compilation is
required; no release build, installation, deployment or live BW event is involved.

```sh
python3 scripts/check-nip-bw-fixtures.py --bip340-reference /tmp/nip-bw-bip340-reference.py
python3 scripts/test-nip-bw-fixtures.py
BW_RESULT_EXPORT=/tmp/p2m-results.json cargo test -p buzz-core
cargo test -p buzz-sdk
cargo test -p buzz-cli
cargo fmt --all -- --check
cargo clippy -p buzz-core -p buzz-sdk -p buzz-cli --all-targets -- -D warnings
```

The Python checks are structural/cryptographic checks only. The Rust corpus test
executes 98 cases / 1,804 steps. Additional tests cover reversed delivery of every
case, malformed lexical JSON, independent negative inputs, evidence correction,
forged/genuine same-ID order, historical branches and the shared query matcher.
Final command results and exact source/Remote SHA belong in the delivery receipt.
Live persistence, live pagination and installed-consumer equivalence remain P3
proof obligations. P2M does not reopen P1, begin P3 or grant product acceptance.

## Relay capability evidence

The local test `handlers::ingest::tests::bw_records_reach_regular_storage`
executed the existing `required_scope_for_kind` predicate with signed P1 records.
Before the change it failed with both
`(46100, Err("restricted: unknown event kind"))` and
`(1063, Err("restricted: unknown event kind"))`. Ingest returns that error before
storage. This is a reproduced source capability gap, not a live Relay probe.

The focused correction admits the two regular kinds with `MessagesWrite` scope
and classifies their repository profiles as channel-less. Normal authentication,
community fences and event verification remain in the ingest path. The existing
regular-event branch calls `insert_event_with_thread_metadata`; neither kind is
replaceable/addressable, ephemeral or an execution command. No BW trigger,
legacy review action or consumer-role bypass is added.

REQ source is unchanged. Explicit kinds avoid the global kindless p-gate; IDs,
time bounds and limit are pushed into `EventQuery`. `#a` uses the existing
NIP-01 result matcher. Core tests execute that same matcher for 46100 and 1063
with kind/repository/ID/time filters and reject a foreign repository or an earlier
page bound. These tests prove classification and filter behavior, not database
persistence. In particular, generic-tag filtering occurs after the bounded SQL
candidate page: live repository pagination/completeness must be checked in P3.
No empty response or this source inspection is presented as a persistence proof.

Because ingest source changed, the complete `cargo test -p buzz-relay` suite and
relay Clippy lane are also required. Environment-dependent failures, if any, are
reported individually in the final receipt rather than relabeled as PASS.

The pre-commit complete Relay run produced 874 passes, 43 ignored tests and these
8 failures. The new BW regression passed. No attempt was made to start services
or rewrite unrelated tests to make this suite green:

- `api::admin::tests::feedback_attachment_rejects_unknown_feedback`: 500 instead of 404.
- `api::admin::tests::report_detail_rejects_unknown_report`: 500 instead of 404.
- `api::media::tests::media_read_accepts_range_header_only_after_auth`: `Sqlx(PoolTimedOut)`.
- `api::media::tests::media_read_rejects_upload_verb_wrong_server_and_wrong_x`: `Sqlx(PoolTimedOut)`.
- `api::media::tests::media_read_with_valid_server_scoped_token_reaches_sidecar_gate`: `Sqlx(PoolTimedOut)`.
- `api::media::tests::media_reads_reject_unauthenticated_get_and_head_before_sidecar_gate`: `Sqlx(PoolTimedOut)`.
- `api::media::tests::upload_concurrency_limit_is_scoped_by_community`: `Sqlx(PoolTimedOut)`.
- `api::media::tests::upload_rate_limiter_is_scoped_by_community`: `Sqlx(PoolTimedOut)`.

The Admin tests also use a lazy Postgres pool; their 500s are consistent with the
unavailable database, but are reported as observed failures, not a demonstrated
baseline PASS. These remain open package/integration evidence. The final run uses
`--no-fail-fast` so that failures in the Relay test binary do not suppress the
other package targets or doc tests.
