# Selective port: PR #6780 canvas writer-discipline — provenance record

Branch: `integration/canvas-writer-discipline` @ `f33a5232a` (pushed to mfethe1/buzz)
Base: `upstream/main` (block/buzz) @ `583af0229` ("fix(cli): preserve signatures in event reads (#6884)")
Ported: 11 non-merge commits of upstream PR #6780 `duncan/canvas-version-history`,
range `66ca4bbfd..d01dced8f`, **cherry-picked with `-x` provenance, zero conflicts**.

## What this is (and is not)

FORK.md forbids merging raw community PR heads. The prior `integration/canvas-history-6780`
branch held the raw PR merge head (including two `origin/main` merges). This branch is the
**clean selective port**: every canvas-content change of #6780, replayed one-by-one onto
current upstream/main, original authors and commit messages preserved via `cherry-pick -x`.

## Commit map (original → ported)

| #6780 original | This branch |
|---|---|
| 66ca4bbfd feat: canvas version history w/ optimistic concurrency | 4c40f2a6e |
| 45fd7fb99 fix(canvas): enforce head advancement, tighten tags | a98d100ea |
| 72b4b9ebd fix(canvas): bound history limit, SDK v3 writer discipline | 69a3d2534 |
| 29850adca fix(canvas): clippy int_plus_one | 0a84d013a |
| c93c0fe0c fix(canvas): validate SDK inputs, harden CLI restore/query | 58bccd726 |
| 215fd1f41 feat(desktop): history, restore, conflict-checked save | f38894791 |
| a11106811 fix(canvas): review nits | c772d3a12 |
| d508818c2 refactor: concurrency check client-side, drop relay/DB enforcement | 8d21ad75b |
| 21aa7f179 docs: advisory concurrency | 00baa439e |
| bcdc01cac fix(canvas): writer discipline for CLI set, timestamp helper | 65747da04 |
| d01dced8f test(buzz-cli): pin canvas set discipline | f33a5232a |

Raw merge commits 48af9c1e5 / 5bde10e2a deliberately excluded.

## Content-equivalence proof (live, this run)

`git diff <port> integration/canvas-history-6780` restricted to canvas surfaces is **empty**:
`desktop/src/features/channels/*` (CanvasHistoryPanel, canvasConflict, canvasHooks,
EditSnapshot/EmptyExistence tests), `desktop/src/shared/api/` (tauriCanvas, canvasTypes),
`desktop/src-tauri/src/commands/canvas.rs`. Residual diff vs the raw head is exactly
upstream's 6 newer non-canvas commits (replaceable-store refactor #6777, CLI signature
preservation #6884, desktop link-paste/sidebar fixes) — nothing canvas-related dropped.

## Gates (live, this run, CARGO_TARGET_DIR=/tmp/bz/buzz/target, Hermit)

| Check | Result |
|---|---|
| `cargo fmt --check` | EXIT=0 clean |
| `cargo test -p buzz-cli` (incl. 4 new canvas seam tests + 172-line discipline pin) | 371 passed, 0 failed |
| `cargo test -p buzz-sdk` (incl. canvas builder contract) | 266 passed, 0 failed |
| `cargo test -p buzz-db` | 111 passed, 0 failed |
| `cargo clippy -p buzz-cli -p buzz-sdk --all-targets` | 0 warnings |
| desktop `pnpm test` (node --test, full suite) | 5563 passed, 0 failed |
| Negative control: `git checkout 583af0229 -- crates/buzz-cli crates/buzz-sdk crates/buzz-db` → canvas filter | 0 canvas tests remain (port genuinely owns them); restored, 4/4 pass |

## Inherited baseline failure (recorded, not owned)

`cargo test -p buzz-relay` → `api::mesh_demo::tests::demo_join_forwarded_arm_round_trips_echo`
FAILS (assertion left: 504, right: 200 at mesh_demo.rs:339). **Proven inherited**: pristine
detached checkout of upstream/main @ 583af0229 (untouched by this port, `git diff` on
buzz-relay = 0 lines) fails identically 10s into the same run. Source is byte-identical
to upstream; the failure is a timing/baseline property of current upstream tip, not of
this branch. Per FORK.md this is routed, not silently fixed.

## Next

Stack S4 workstream-board ADAPT work (per `s4-discovery.md` §6) on top of this branch,
replacing `loganj-ws-` discovery with CML-driven discovery. Workstream-board PRs (#6184…#6373)
remain OPEN upstream; watch the 4 overlapping files (`buzz-sdk/src/builders.rs`,
`src-tauri/src/events.rs`, `testing/e2eBridge.ts`, `tests/helpers/bridge.ts`) when porting.
