# Mesh LLM local build prerequisites

Buzz embeds mesh-llm through the Rust SDK pinned in Cargo. mesh-llm's native
skippy/llama layer is linked into the relay and desktop binaries.

## Local Mac demo path

For the first local milestone, use mesh-llm's default native build path. On macOS
this compiles patched llama.cpp/ggml with Metal support the first time a Buzz
binary that depends on mesh is built. The result is cached under Cargo's git
checkout of mesh-llm, so subsequent builds are much faster.

Prerequisites:

```bash
xcode-select --install   # if Command Line Tools are not installed yet
brew install cmake       # if cmake is not already available
```

Then build normally:

```bash
cargo build -p buzz-relay --bin buzz-relay
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

## Connectivity model: public iroh relays off, raw STUN on (WAN)

Buzz Desktop starts the embedded mesh node with `disable_iroh_relays(true)`
(mesh-llm fork rev `bc2f1106`, `RelayPolicy::ExplicitlyDisabled`): **no public
iroh relays** (no `*.iroh.link` traffic, no relay transport), so there is no
empty-relay fallback to public infrastructure.

Raw STUN **remains on** under this policy: the node discovers its public address
(via `stun.l.google.com` / `stun.cloudflare.com`) and injects it into the invite
token / `EndpointAddr`. That address rides the relay-signed `24621`/`24622`
call-me-now exchange, and peers hole-punch directly over UDP — so this works over
**WAN**, not just LAN. STUN is a "what's my public IP" lookup, not a relay or a
data path; the Buzz relay performs the address-exchange coordination.

Residual limit (intentional v1): with iroh relays off there is **no relay
transport fallback**, so two peers both behind **symmetric NATs** may fail to
hole-punch. Works for the common cases (≥1 side cone-NAT / port-forwarded /
server). A hosted private-iroh-relay fallback (relay advertises an
`iroh_relay_url` + NIP-98 auth) is the planned follow-up once the relay side is
wired end-to-end.

## Mesh-compute e2e acceptance matrix

"Full e2e" for mesh-compute is necessarily layered: the Playwright harness
drives the **web build** of the desktop UI through a bridge, so it cannot
execute real Tauri mesh commands or real GGUF inference. We therefore split
coverage into three layers and are explicit about what is real vs mocked.

| # | What it proves | Where | Real / Mocked | Runs in CI? | How to run |
|---|----------------|-------|---------------|-------------|------------|
| 1 | serve node + client node + mesh routing + **real inference** | `crates/buzz-relay/examples/mesh_serve_client_smoke.rs` | **REAL** (loads a model, runs inference, joins a real mesh) | No — hardware-gated | `just mesh-e2e-hardware` (or `cargo run -p buzz-relay --example mesh_serve_client_smoke`) |
| 2 | admission **invariant**: relay membership is the only factor | `crates/buzz-relay/src/handlers/mesh_signaling.rs` (`*_admitted` / `denied_is_not_admitted` tests) | REAL policy logic, no I/O | **Yes** | `cargo test -p buzz-relay mesh_signaling` |
| 2b | live db-membership admission + member/non-member status reads | `crates/buzz-test-client/tests/e2e_mesh_llm.rs` (`trust_*`) | REAL relay over ws | No — env-gated (`MEMBER_NSEC`/`STRANGER_NSEC`, live relay) | see that file's module docs |
| 3 | desktop UI contract: Share-compute start/stop, Run-on-relay-mesh preset, **ensure-before-spawn** order, membership-gated toggle | `desktop/tests/e2e/mesh-compute.spec.ts` | UI REAL, Tauri mesh commands MOCKED via the e2e bridge | **Yes** | `cd desktop && pnpm test:e2e:integration -- mesh-compute.spec.ts` |

`just mesh-e2e` runs the two CI-safe layers (2 + 3). Layer 1 is run on hardware.
`just mesh-e2e-hardware` prepares a matching MeshLLM native runtime in a Buzz-controlled local cache before starting the real serve/client smoke, so it does not depend on a manually preseeded `MESH_LLM_NATIVE_RUNTIME_CACHE_DIR`.

### What "real" means per layer

- **Layer 1 is the only layer that proves inference.** It starts a real serve
  node on the GPU, a real client node that joins via the serve node's invite
  token, and asserts a chat completion *routed through the client* returns
  `finish_reason=stop` with non-empty content. Verified locally with
  SmolLM2-135M; point `MESH_SMOKE_MODEL` at a larger `.gguf` for scale.
  The just target first runs `scripts/ensure-mesh-native-runtime.sh`, which
  builds/packages/installs the MeshLLM native runtime matching the pinned SDK
  into `.cache/mesh-llm-native-runtime` and exports `MESH_LLM_NATIVE_RUNTIME_CACHE_DIR`
  for the smoke. Note: with `disable_iroh_relays(true)` the join bootstraps via
  STUN-discovered public addresses + relay-signed call-me-now (no public iroh
  relay) — see the connectivity model above.
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
