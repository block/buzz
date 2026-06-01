# Mesh LLM local build prerequisites

Sprout embeds mesh-llm through the Rust SDK pinned in Cargo. mesh-llm's native
skippy/llama layer is linked into the relay and desktop binaries.

## Local Mac demo path

For the first local milestone, use mesh-llm's default native build path. On macOS
this compiles patched llama.cpp/ggml with Metal support the first time a Sprout
binary that depends on mesh is built. The result is cached under Cargo's git
checkout of mesh-llm, so subsequent builds are much faster.

Prerequisites:

```bash
xcode-select --install   # if Command Line Tools are not installed yet
brew install cmake       # if cmake is not already available
```

Then build normally:

```bash
cargo build -p sprout-relay --bin sprout-relay
cargo check --manifest-path desktop/src-tauri/Cargo.toml
```

Expect the first build to take several minutes while mesh-llm prepares and builds
patched llama.cpp. This is intentional for the local demo: there is no external
binary artifact to fetch and no separate dylib path to configure.

## CI / release path

CI should not rebuild llama.cpp from scratch on every job. CI and release now
prebuild the llama native libraries in a dedicated step (`prepare-llama.sh` +
`build-llama.sh -DCMAKE_OSX_DEPLOYMENT_TARGET=10.15`) and the Tauri build reuses
them via `SKIPPY_LLAMA_AUTO_BUILD=0` + `LLAMA_STAGE_BUILD_DIR`. That build is
cached with `actions/cache` keyed on the mesh-llm rev (resolved from
`Cargo.lock`), so a cache hit skips the rebuild and a dependency bump
invalidates the cache automatically — no workflow edit required.

A dynamic-link artifact path remains a possible future optimization. The
mesh-llm build script supports dynamic linking with:

```bash
export LLAMA_STAGE_LINK_MODE=dynamic
export LLAMA_STAGE_LIB_DIR=/path/to/prebuilt/llama/libs
```

Do not use dynamic-link locally unless you already have compatible `llama`,
`llama-common`, and `mtmd` dynamic libraries. The default static build is the
supported local path for M1.

## Current privacy limitation: public STUN

Sprout Desktop now refuses to start an embedded mesh node unless the active relay
advertises a Sprout-owned `iroh_relay_url`, and it passes a fresh NIP-98 bearer
to that relay. This prevents mesh-llm's empty-relay fallback to public iroh relay
URLs.

mesh-llm `bd16da4` still performs raw public STUN on startup to discover the
host's public IP (`stun.l.google.com`, `stun.cloudflare.com`, or
`stun.stunprotocol.org`) and may include that public address in its invite token.
That behavior is inside mesh-llm's host runtime and is not currently exposed as
an SDK option. Treat it as a v1 limitation until mesh exposes a disable-public-
STUN / relay-only-addressing knob.

## Mesh-compute e2e acceptance matrix

"Full e2e" for mesh-compute is necessarily layered: the Playwright harness
drives the **web build** of the desktop UI through a bridge, so it cannot
execute real Tauri mesh commands or real GGUF inference. We therefore split
coverage into three layers and are explicit about what is real vs mocked.

| # | What it proves | Where | Real / Mocked | Runs in CI? | How to run |
|---|----------------|-------|---------------|-------------|------------|
| 1 | serve node + client node + mesh routing + **real inference** | `crates/sprout-relay/examples/mesh_serve_client_smoke.rs` | **REAL** (loads a model, runs inference, joins a real mesh) | No — hardware-gated | `just mesh-e2e-hardware` (or `cargo run -p sprout-relay --example mesh_serve_client_smoke`) |
| 2 | admission **invariant**: relay membership is the only factor | `crates/sprout-relay/src/iroh_relay.rs` (`admission_from_membership` tests) | REAL policy logic, no I/O | **Yes** | `cargo test -p sprout-relay iroh_relay::tests::admission` |
| 2b | live db-membership admission + member/non-member status reads | `crates/sprout-test-client/tests/e2e_mesh_llm.rs` (`trust_*`) | REAL relay over ws | No — env-gated (`MEMBER_NSEC`/`STRANGER_NSEC`, live relay) | see that file's module docs |
| 3 | desktop UI contract: Share-compute start/stop, Run-on-relay-mesh preset, **ensure-before-spawn** order, membership-gated toggle | `desktop/tests/e2e/mesh-compute.spec.ts` | UI REAL, Tauri mesh commands MOCKED via the e2e bridge | **Yes** | `cd desktop && pnpm test:e2e:integration -- mesh-compute.spec.ts` |

`just mesh-e2e` runs the two CI-safe layers (2 + 3). Layer 1 is run on hardware.

### What "real" means per layer

- **Layer 1 is the only layer that proves inference.** It starts a real serve
  node on the GPU, a real client node that joins via the serve node's invite
  token, and asserts a chat completion *routed through the client* returns
  `finish_reason=stop` with non-empty content. Verified locally with
  SmolLM2-135M; point `MESH_SMOKE_MODEL` at a larger `.gguf` for scale.
  Note: even with mDNS discovery the join bootstraps through mesh's public iroh
  relay at the pinned rev — see the STUN limitation above.
- **Layer 2 proves the auth invariant without faking it.** The policy mapping
  (`MembershipDecision` → admit/deny) is exercised directly: member → allow,
  open relay → allow, non-member → deny, owner-delegation → deny (v1),
  error → deny. A valid NIP-98 identity or possession of dial metadata is, by
  itself, never sufficient. The membership-only gate is what matters **when
  membership enforcement is enabled** (`require_relay_membership = true`); an
  open relay (`OpenRelay`) intentionally admits any valid NIP-98 signer, so
  `open relay → allow` is the deliberate disabled-enforcement case, not a
  contradiction of the membership gate.
- **Layer 3 proves the UI contract, not inference.** The mesh Tauri commands
  are mocked, but the assertions are on real UI behavior and real command
  *ordering* (`mesh_ensure_client_node` is recorded before `create_managed_agent`),
  and on the membership gate (a non-member cannot enable relay-mesh at all).
