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
