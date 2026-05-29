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

CI should not rebuild llama.cpp from scratch on every job. For CI/release we will
add a cached native build or a dynamic-link artifact path as a follow-up. The
mesh-llm build script supports dynamic linking with:

```bash
export LLAMA_STAGE_LINK_MODE=dynamic
export LLAMA_STAGE_LIB_DIR=/path/to/prebuilt/llama/libs
```

Do not use dynamic-link locally unless you already have compatible `llama`,
`llama-common`, and `mtmd` dynamic libraries. The default static build is the
supported local path for M1.
