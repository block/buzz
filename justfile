# Sprout — development task runner

set dotenv-load := true

desktop_dir := "desktop"
desktop_tauri_manifest := "desktop/src-tauri/Cargo.toml"
web_dir := "web"

# List all available tasks
default:
    @just --list

# ─── Dev Environment ─────────────────────────────────────────────────────────

# Start Docker services, run migrations, install desktop deps
setup:
    ./scripts/dev-setup.sh

# ⚠️  Wipe ALL data and recreate a clean environment
[confirm("This will DELETE all local data. Continue? (y/N)")]
reset:
    ./scripts/dev-reset.sh --yes

# Stop all dev services (keep data)
down:
    docker compose down

# Show dev service status
ps:
    docker compose ps

# Tail all service logs
logs *ARGS:
    docker compose logs -f {{ARGS}}

# ─── Build & Check ───────────────────────────────────────────────────────────

# Build the Rust workspace
build:
    cargo build --workspace

# Build the Rust workspace in release mode
build-release:
    cargo build --workspace --release

# Rebuild Typesense docs for all kind:0 (user profile) events.
# Required once after deploying the indexer change that flattens kind:0 content
# for searchability; new/updated profiles are indexed correctly automatically.
# Safe to run repeatedly — Typesense upserts.
reindex-kind0:
    cargo run --release -p sprout-relay --bin sprout-reindex-kind0

# Run repo lint and formatting checks
check: fmt-check clippy desktop-check desktop-tauri-fmt-check web-check mobile-check

# Format all Rust code
fmt:
    cargo fmt --all

# Check formatting without modifying files
fmt-check:
    cargo fmt --all -- --check

# Run clippy with warnings as errors
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Install JS dependencies (pnpm workspace — installs all packages from root)
desktop-install:
    pnpm install

# Install JS dependencies reproducibly for CI (pnpm workspace)
desktop-install-ci:
    pnpm install --frozen-lockfile

# Run desktop lint and format checks
desktop-check:
    cd {{desktop_dir}} && pnpm check

# Run desktop TypeScript checks
desktop-typecheck:
    cd {{desktop_dir}} && pnpm typecheck

# Build desktop frontend assets
desktop-build:
    cd {{desktop_dir}} && pnpm build

# Format desktop Tauri Rust code
desktop-tauri-fmt:
    cargo fmt --manifest-path {{desktop_tauri_manifest}} --all

# Check desktop Tauri Rust formatting
desktop-tauri-fmt-check:
    cargo fmt --manifest-path {{desktop_tauri_manifest}} --all -- --check

# Ensure sidecar placeholder binaries exist (Tauri validates externalBin at compile time)
_ensure-sidecar-stubs:
    #!/usr/bin/env bash
    set -euo pipefail
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    mkdir -p desktop/src-tauri/binaries
    for bin in sprout-acp sprout-mcp-server sprout-agent sprout-dev-mcp git-credential-nostr sprout; do
        touch "desktop/src-tauri/binaries/${bin}-${TARGET}"
    done

# Check the desktop Tauri Rust crate compiles
desktop-tauri-check: _ensure-sidecar-stubs
    cargo check --manifest-path {{desktop_tauri_manifest}}

# Build the full desktop Tauri app locally (unsigned, for testing)
desktop-release-build target="aarch64-apple-darwin":
    #!/usr/bin/env bash
    set -euo pipefail
    TARGET={{target}}
    mkdir -p desktop/src-tauri/binaries
    touch "desktop/src-tauri/binaries/sprout-acp-$TARGET"
    touch "desktop/src-tauri/binaries/sprout-mcp-server-$TARGET"
    touch "desktop/src-tauri/binaries/sprout-agent-$TARGET"
    touch "desktop/src-tauri/binaries/sprout-dev-mcp-$TARGET"
    touch "desktop/src-tauri/binaries/git-credential-nostr-$TARGET"
    touch "desktop/src-tauri/binaries/sprout-$TARGET"
    pnpm install
    cd {{desktop_dir}} && pnpm tauri build --target {{target}}

# Run desktop checks suitable for CI / pre-push
desktop-ci: desktop-check desktop-tauri-fmt-check desktop-build desktop-tauri-check

# Seed deterministic channel data for desktop Playwright tests
desktop-e2e-seed:
    ./scripts/setup-desktop-test-data.sh

# Run desktop browser smoke tests
desktop-e2e-smoke:
    cd {{desktop_dir}} && pnpm test:e2e:smoke

# Run desktop relay-backed e2e tests
desktop-e2e-integration:
    cd {{desktop_dir}} && pnpm test:e2e:integration

# Run all checks suitable for CI / pre-push (no infra needed)
ci: check test-unit desktop-build desktop-tauri-check web-build mobile-test

# ─── Test ─────────────────────────────────────────────────────────────────────

# Run all tests (unit + integration)
test:
    ./scripts/run-tests.sh all

# Run unit tests only (no infra needed)
test-unit:
    #!/usr/bin/env bash
    if command -v cargo-nextest &>/dev/null; then
        cargo nextest run -p sprout-core -p sprout-auth --lib
    else
        ./scripts/run-tests.sh unit
    fi

# Run integration tests only (starts services if needed)
test-integration:
    ./scripts/run-tests.sh integration

# ─── Run ──────────────────────────────────────────────────────────────────────

# Start the relay server
relay:
    cargo run -p sprout-relay

# Start the relay with the built web UI served from it
relay-web:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ -d node_modules ]] || pnpm install
    pnpm -C web build
    SPROUT_WEB_DIR=./web/dist cargo run -p sprout-relay

# Start the relay server in release mode
relay-release:
    cargo run -p sprout-relay --release

# Start sprout-proxy (dev mode)
proxy:
    cargo run -p sprout-proxy

# Start sprout-proxy (release mode)
proxy-release:
    cargo run -p sprout-proxy --release

# Run the desktop Tauri app in dev mode (ports and identity derived from worktree)
dev *ARGS: _ensure-sidecar-stubs
    #!/usr/bin/env bash
    set -euo pipefail
    cd {{desktop_dir}}
    [[ -d node_modules ]] || pnpm install
    source ../scripts/instance-env.sh
    echo "Starting on Vite port ${SPROUT_VITE_PORT}, relay ${SPROUT_RELAY_URL}"
    pnpm exec tauri dev --config "$SPROUT_TAURI_CONFIG" {{ARGS}}

# Run the desktop app against the internal staging relay (installs deps + builds agent tools automatically)
staging *ARGS: _ensure-sidecar-stubs
    #!/usr/bin/env bash
    set -euo pipefail
    pnpm install
    cargo build --release -p sprout-acp -p sprout-mcp -p sprout-agent -p sprout-dev-mcp -p sprout-cli
    # Replace the 0-byte sidecar stub with the real CLI binary so tauri dev picks it up.
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    cp target/release/sprout "desktop/src-tauri/binaries/sprout-${TARGET}"
    chmod +x "desktop/src-tauri/binaries/sprout-${TARGET}"
    cd {{desktop_dir}}
    source ../scripts/instance-env.sh
    export SPROUT_RELAY_URL="wss://sprout-oss.stage.blox.sqprod.co"
    echo "Starting staging on Vite port ${SPROUT_VITE_PORT}, relay ${SPROUT_RELAY_URL}"
    pnpm exec tauri dev --config "$SPROUT_TAURI_CONFIG" {{ARGS}}

# Run the desktop frontend dev server (port derived from worktree)
desktop-dev:
    #!/usr/bin/env bash
    set -euo pipefail
    cd {{desktop_dir}}
    [[ -d node_modules ]] || pnpm install
    source ../scripts/instance-env.sh
    echo "Starting frontend dev server on Vite port ${SPROUT_VITE_PORT}, relay ${SPROUT_RELAY_URL}"
    pnpm exec vite --port "${SPROUT_VITE_PORT}" --strictPort

# Run the desktop Tauri app (alias for dev)
desktop-app *ARGS:
    just dev {{ARGS}}

# ─── Web ─────────────────────────────────────────────────────────────────────

# Run the web frontend dev server (port derived from worktree to avoid collisions)
web:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ -d node_modules ]] || pnpm install
    source scripts/instance-env.sh
    export VITE_PORT=$((SPROUT_VITE_PORT + 100))
    export VITE_RELAY_URL="${SPROUT_RELAY_URL}"
    echo "Starting web dev server on port ${VITE_PORT}, relay ${SPROUT_RELAY_URL}"
    cd {{web_dir}}
    pnpm exec vite --port "${VITE_PORT}" --strictPort

# Install web JS dependencies (pnpm workspace — installs all packages from root)
web-install:
    pnpm install

# Install web JS dependencies reproducibly for CI (pnpm workspace)
web-install-ci:
    pnpm install --frozen-lockfile

# Run web lint and format checks
web-check:
    cd {{web_dir}} && pnpm check

# Run web TypeScript checks
web-typecheck:
    cd {{web_dir}} && pnpm typecheck

# Build web frontend assets
web-build:
    cd {{web_dir}} && pnpm build

# Run web browser smoke tests
web-e2e-smoke:
    cd {{web_dir}} && pnpm test:e2e:smoke

# ─── Mobile ──────────────────────────────────────────────────────────────────

mobile_dir := "mobile"

# Install mobile Flutter dependencies
mobile-install:
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && flutter pub get

# Run mobile lint and format checks
mobile-check:
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && dart format --output=none --set-exit-if-changed . && flutter analyze

# Run mobile tests
mobile-test:
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && flutter test

# ─── Database ─────────────────────────────────────────────────────────────────

# Apply schema migrations via pgschema
migrate:
    ./bin/pgschema apply --file schema/schema.sql --auto-approve

# ─── Utilities ────────────────────────────────────────────────────────────────

# Remove build artifacts
clean:
    cargo clean
    cargo clean --manifest-path desktop/src-tauri/Cargo.toml

# Check the Rust workspace compiles without producing binaries
check-compile:
    cargo check --workspace --all-targets

# ─── Agent Harness ────────────────────────────────────────────────────────────

# Run a goose agent connected to a Sprout relay (foreground)
goose relay="ws://localhost:3000" agents="1" heartbeat="0" prompt="" key="$SPROUT_PRIVATE_KEY":
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release -p sprout-acp -p sprout-mcp -p sprout-cli
    env_args=(
        SPROUT_RELAY_URL="{{relay}}"
        SPROUT_PRIVATE_KEY="{{key}}"
        SPROUT_ACP_AGENT_COMMAND=goose
        SPROUT_ACP_AGENT_ARGS=acp
        SPROUT_ACP_MCP_COMMAND=./target/release/sprout-mcp-server
        SPROUT_ACP_AGENTS="{{agents}}"
        GOOSE_MODE=auto
    )
    [[ -n "{{prompt}}" ]] && env_args+=(SPROUT_ACP_SYSTEM_PROMPT="{{prompt}}")
    if [[ "{{heartbeat}}" != "0" ]]; then
        env_args+=(SPROUT_ACP_HEARTBEAT_INTERVAL={{heartbeat}})
    fi
    exec env "${env_args[@]}" ./target/release/sprout-acp

# Run a goose agent in the background (screen session named 'goose-agent-N')
goose-bg relay="ws://localhost:3000" agents="1" heartbeat="0" prompt="" key="$SPROUT_PRIVATE_KEY":
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --release -p sprout-acp -p sprout-mcp -p sprout-cli
    env_args=(
        SPROUT_RELAY_URL="{{relay}}"
        SPROUT_PRIVATE_KEY="{{key}}"
        SPROUT_ACP_AGENT_COMMAND=goose
        SPROUT_ACP_AGENT_ARGS=acp
        SPROUT_ACP_MCP_COMMAND=./target/release/sprout-mcp-server
        SPROUT_ACP_AGENTS="{{agents}}"
        GOOSE_MODE=auto
    )
    [[ -n "{{prompt}}" ]] && env_args+=(SPROUT_ACP_SYSTEM_PROMPT="{{prompt}}")
    if [[ "{{heartbeat}}" != "0" ]]; then
        env_args+=(SPROUT_ACP_HEARTBEAT_INTERVAL={{heartbeat}})
    fi
    screen -dmS goose-agent-{{agents}} bash -c "$(printf '%q ' env "${env_args[@]}") ./target/release/sprout-acp"
    echo "Agent running in screen session 'goose-agent-{{agents}}'. Attach with: screen -r goose-agent-{{agents}}"
