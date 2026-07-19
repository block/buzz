# Buzz — development task runner

set dotenv-load := true

desktop_dir := "desktop"
desktop_tauri_manifest := "desktop/src-tauri/Cargo.toml"
web_dir := "web"

# Opt-in mesh-llm. Off by default so `just dev`/`just staging` skip ~420 extra
# crates + the llama.cpp native runtime build and stay fast to iterate on.
# Turn on to test mesh compute features: `just mesh=1 dev` / `just mesh=1 staging`.
mesh := ""

# Reset only the current standalone desktop instance before launch.
# Usage: `just fresh=1 desktop-standalone`.
fresh := ""

# List all available tasks
default:
    @just --list

# ─── Dev Environment ─────────────────────────────────────────────────────────

# Install required dev tools via Hermit and create .env (safe to re-run)
bootstrap:
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    # Hermit's bin/ symlinks auto-download pinned tool versions on first use.
    # Running each tool once triggers the download if not already cached.
    echo "Ensuring toolchain via Hermit..."
    cargo --version &
    node --version &
    pnpm --version &
    wait
    if ! command -v docker &>/dev/null; then
        echo "Error: Docker is required but not installed."
        echo "Install it from https://docs.docker.com/get-docker/"
        exit 1
    fi
    if [[ ! -f .env ]]; then
        cp .env.example .env
        echo "Created .env from .env.example — review it before running just dev."
    fi

# Start Docker services, run migrations, install desktop deps
setup: bootstrap
    ./scripts/dev-setup.sh

# Install git hooks via lefthook (dispatches from the shared .git/hooks dir so all
# linked worktrees inherit the same hooks without a worktree-relative .hooks path)
hooks:
    #!/usr/bin/env bash
    set -euo pipefail
    # --path-format=absolute guarantees an absolute path from every invocation context:
    # without it, --git-common-dir returns ".git" from the main checkout and a
    # relative hooksPath would break linked-worktree dispatch just like .hooks did.
    HOOKS_DIR="$(git rev-parse --path-format=absolute --git-common-dir)/hooks"
    git config --local core.hooksPath "$HOOKS_DIR"
    lefthook install --force

# Wipe development state and recreate a clean environment. Installed Buzz is preserved.
[confirm("This will DELETE all development data and preserve installed Buzz. Continue? (y/N)")]
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

# Run repo lint and formatting checks
check: fmt-check clippy desktop-check desktop-tauri-fmt-check desktop-tauri-clippy web-check mobile-check

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

# Fix desktop lint and format issues
desktop-fix:
    cd {{desktop_dir}} && pnpm exec biome check --write . && pnpm check:file-sizes

# Run desktop TS helper unit tests
desktop-test:
    cd {{desktop_dir}} && pnpm test

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

# Format all code (Rust + Tauri Rust + Dart)
fmt-all: fmt desktop-tauri-fmt mobile-fmt

# Fix all formatting and lint issues
fix-all: fmt desktop-tauri-fmt desktop-fix web-fix mobile-fix

# Ensure sidecar placeholder binaries exist (Tauri validates externalBin at compile time)
# Sidecar binary list must stay in sync with desktop-release-build below.
_ensure-sidecar-stubs:
    #!/usr/bin/env bash
    set -euo pipefail
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    mkdir -p desktop/src-tauri/binaries
    for bin in buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz; do
        touch "desktop/src-tauri/binaries/${bin}-${TARGET}"
    done

# Ensure Docker dev services (Postgres, Redis, etc.) are running and healthy
_ensure-services:
    #!/usr/bin/env bash
    set -euo pipefail
    pg=$(docker inspect --format '{{"{{"}}.State.Health.Status{{"}}"}}' buzz-postgres 2>/dev/null || echo "not_found")
    redis=$(docker inspect --format '{{"{{"}}.State.Health.Status{{"}}"}}' buzz-redis 2>/dev/null || echo "not_found")
    if [[ "$pg" == "healthy" && "$redis" == "healthy" ]]; then
        echo "Services already healthy"
        exit 0
    fi
    echo "Starting services..."
    docker compose up -d || true
    echo -n "Waiting for services"
    for i in $(seq 1 40); do
        pg=$(docker inspect --format '{{"{{"}}.State.Health.Status{{"}}"}}' buzz-postgres 2>/dev/null || echo "not_found")
        redis=$(docker inspect --format '{{"{{"}}.State.Health.Status{{"}}"}}' buzz-redis 2>/dev/null || echo "not_found")
        if [[ "$pg" == "healthy" && "$redis" == "healthy" ]]; then
            echo " ready"
            exit 0
        fi
        echo -n "."
        sleep 3
    done
    echo " timed out"
    exit 1

# Apply database migrations and seed the local dev community if the dev database is running
_ensure-migrations: _ensure-services
    cargo run -p buzz-admin -- migrate
    ./scripts/seed-local-community.sh

# Run clippy on the desktop Tauri Rust crate
desktop-tauri-clippy: _ensure-sidecar-stubs
    cargo clippy --manifest-path {{desktop_tauri_manifest}} --all-targets -- -D warnings

# Check the desktop Tauri Rust crate compiles
desktop-tauri-check: _ensure-sidecar-stubs
    cargo check --manifest-path {{desktop_tauri_manifest}}

# Run desktop Tauri Rust unit tests
desktop-tauri-test: _ensure-sidecar-stubs
    cd desktop/src-tauri && cargo test

# Verify compiled-flag behavior under both compile states (clean + internal).
# Runs the observer_archive focused test twice with independently supplied
# expected values; build.rs rerun-if-env-changed triggers recompilation.
desktop-tauri-test-compiled-flags: _ensure-sidecar-stubs
    #!/usr/bin/env bash
    set -euo pipefail
    cd desktop/src-tauri
    echo "=== Clean build (no flag) → expect false ==="
    env -u BUZZ_BUILD_OBSERVER_ARCHIVE_DEFAULT \
      BUZZ_TEST_EXPECTED_OBSERVER_ARCHIVE_DEFAULT=false \
      cargo test observer_archive_default_enabled_matches_expected -- --ignored --nocapture
    echo "=== Internal build (flag set) → expect true ==="
    BUZZ_BUILD_OBSERVER_ARCHIVE_DEFAULT=1 \
      BUZZ_TEST_EXPECTED_OBSERVER_ARCHIVE_DEFAULT=true \
      cargo test observer_archive_default_enabled_matches_expected -- --ignored --nocapture
    echo "Both compiled states verified."

# Build the full desktop Tauri app locally (unsigned, for testing)
# Sidecar binary list must stay in sync with _ensure-sidecar-stubs above.
# pnpm install is unconditional here: release builds must start from a clean dep tree.
desktop-release-build target="aarch64-apple-darwin":
    #!/usr/bin/env bash
    set -euo pipefail
    TARGET={{target}}
    mkdir -p desktop/src-tauri/binaries
    touch "desktop/src-tauri/binaries/buzz-acp-$TARGET"
    touch "desktop/src-tauri/binaries/buzz-agent-$TARGET"
    touch "desktop/src-tauri/binaries/buzz-dev-mcp-$TARGET"
    touch "desktop/src-tauri/binaries/git-credential-nostr-$TARGET"
    touch "desktop/src-tauri/binaries/buzz-$TARGET"
    pnpm install
    cd {{desktop_dir}} && pnpm tauri build --features mesh-llm --target {{target}}

# Run desktop checks suitable for CI / pre-push
desktop-ci: desktop-check desktop-test desktop-tauri-fmt-check desktop-build desktop-tauri-check desktop-tauri-test

# Seed deterministic channel data for desktop Playwright tests
desktop-e2e-seed: _ensure-migrations
    ./scripts/setup-desktop-test-data.sh

# Run desktop browser smoke tests
desktop-e2e-smoke:
    cd {{desktop_dir}} && pnpm test:e2e:smoke

# Run desktop relay-backed e2e tests
desktop-e2e-integration: _ensure-migrations
    cd {{desktop_dir}} && pnpm test:e2e:integration

# Run only the e2e specs changed vs origin/main (both projects) before pushing
desktop-e2e-pre-push: _ensure-migrations
    git fetch origin main
    cd {{desktop_dir}} && pnpm build:e2e && pnpm exec playwright test --only-changed=origin/main

# Run all checks suitable for CI / pre-push (no infra needed)
ci: check test-unit desktop-test desktop-build desktop-tauri-check desktop-tauri-test web-build mobile-test

# ─── Test ─────────────────────────────────────────────────────────────────────

# Run all tests (unit + integration)
test:
    ./scripts/run-tests.sh all

# Run unit tests only (no infra needed)
test-unit:
    #!/usr/bin/env bash
    if command -v cargo-nextest &>/dev/null; then
        cargo nextest run -p buzz-core -p buzz-auth --lib
        # buzz-db migrator/lint tests: pure SQL-parsing unit tests (no infra).
        # They guard the embedded-migrator invariant (exactly the consolidated
        # 0001; cutover/backfill stays an operator script, not startup state)
        # and the tenant-scoping lints. The Postgres-backed buzz-db tests are
        # #[ignore]d, so --lib runs only the infra-free set. Without this gate a
        # stray file in migrations/ or a broken lint ships green.
        cargo nextest run -p buzz-db --lib
        # Multi-tenant conformance gate (buzz-conformance): the independent
        # replay checker + golden fixtures. No infra — pure in-process trace
        # replay — so it belongs in the unit job. Run all targets (lib + the
        # tests/replay_fixtures.rs integration test), not just --lib.
        cargo nextest run -p buzz-conformance
        # Gateway unit and black-box HTTP tests are infra-free. Postgres-backed
        # contract/race tests run in the dedicated CI job below.
        cargo nextest run -p buzz-push-gateway
    else
        ./scripts/run-tests.sh unit
    fi

# Run integration tests only (starts services if needed)
test-integration:
    ./scripts/run-tests.sh integration

# Buzz shared compute e2e: current desktop discovery/admission logic and
# Playwright UI coverage.
mesh-e2e:
    cargo test --manifest-path {{desktop_dir}}/src-tauri/Cargo.toml --features mesh-llm mesh_llm --lib
    cd {{desktop_dir}} && pnpm test:e2e:smoke -- mesh-compute.spec.ts

# Reset only development state, seed deterministic local channels, and launch
# the mesh-enabled desktop with the repository's public Tyler test identity.
# This is for local verification only; never point this identity at staging/prod.
[confirm("This will reset development data, preserve installed Buzz, then launch a seeded mesh dev app. Continue? (y/N)")]
mesh-dev-fresh:
    #!/usr/bin/env bash
    set -euo pipefail
    ./scripts/dev-reset.sh --yes
    ./scripts/setup-desktop-test-data.sh
    export BUZZ_PRIVATE_KEY="3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03"
    export BUZZ_REQUIRE_RELAY_MEMBERSHIP=true
    export BUZZ_ALLOW_NIP_OA_AUTH=true
    export RELAY_OWNER_PUBKEY="e5ebc6cdb579be112e336cc319b5989b4bb6af11786ea90dbe52b5f08d741b34"
    export BUZZ_RELAY_PRIVATE_KEY="0000000000000000000000000000000000000000000000000000000000000001"
    export BUZZ_RECONCILE_CHANNELS=true
    export BUZZ_RESET_WEBVIEW_STATE=1
    exec just mesh=1 dev

# Real serve->client->inference on this machine (not CI).
mesh-e2e-hardware:
    #!/usr/bin/env bash
    set -euo pipefail
    export MESH_LLM_NATIVE_RUNTIME_CACHE_DIR="$(./scripts/ensure-mesh-native-runtime.sh)"
    cargo run -p buzz-relay --example mesh_serve_client_smoke

# Three isolated node processes: trusted member joins and infers; stranger is rejected.
# Uses temp homes and explicit mesh owner keystores. Never reads the Buzz Keychain.
mesh-e2e-admission:
    #!/usr/bin/env bash
    set -euo pipefail
    export MESH_LLM_NATIVE_RUNTIME_CACHE_DIR="$(./scripts/ensure-mesh-native-runtime.sh)"
    cargo run -p buzz-relay --example mesh_admission_smoke

# Full hardware confidence suite: routing, owner admission, and real agent inference.
mesh-e2e-confidence:
    #!/usr/bin/env bash
    set -euo pipefail
    export MESH_LLM_NATIVE_RUNTIME_CACHE_DIR="$(./scripts/ensure-mesh-native-runtime.sh)"
    cargo build --release -p buzz-agent -p buzz-dev-mcp
    cargo run -p buzz-relay --example mesh_serve_client_smoke
    cargo run -p buzz-relay --example mesh_admission_smoke
    cargo run -p buzz-relay --example mesh_agent_e2e

# Take desktop screenshots using the mock bridge
desktop-screenshot *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    pnpm -C {{desktop_dir}} build:e2e
    cd {{desktop_dir}}
    if ! curl -sf http://127.0.0.1:4173/ >/dev/null 2>&1; then
        python3 -m http.server 4173 -d dist >/dev/null 2>&1 &
        trap "kill $! 2>/dev/null || true" EXIT
        for i in $(seq 1 20); do curl -sf http://127.0.0.1:4173/ >/dev/null && break; sleep 0.5; done
    fi
    node tests/helpers/screenshot.mjs {{ARGS}}

# ─── Run ──────────────────────────────────────────────────────────────────────

# Start the relay server (auto-starts Docker services if needed)
relay: bootstrap _ensure-migrations
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    cargo run -p buzz-relay

# Start the relay with the built web UI served from it
relay-web: bootstrap _ensure-migrations
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    [[ -d node_modules ]] || pnpm install
    pnpm -C web build
    BUZZ_WEB_DIR=./web/dist cargo run -p buzz-relay

# Build and run the private read-only admin dashboard
admin: bootstrap _ensure-migrations
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    [[ -d node_modules ]] || pnpm install
    pnpm -C admin-web build
    export BUZZ_ADMIN_HOST="${BUZZ_ADMIN_HOST:-admin.localhost:3000}"
    export BUZZ_ADMIN_WEB_DIR="${BUZZ_ADMIN_WEB_DIR:-{{justfile_directory()}}/admin-web/dist}"
    echo "Admin dashboard: http://${BUZZ_ADMIN_HOST}/reports"
    cargo run -p buzz-relay

# Seed deterministic reports and product feedback for local admin dashboard review
admin-seed: _ensure-migrations
    ./scripts/seed-admin-dashboard.sh

# Run focused relay and browser checks for the read-only admin dashboard
admin-check: fmt-check
    cargo check -p buzz-relay --all-targets
    cargo test -p buzz-relay api::admin
    cargo test -p buzz-relay router::tests
    pnpm -C admin-web check
    pnpm -C admin-web exec playwright test

# Start the relay server in release mode
relay-release: _ensure-migrations
    cargo run -p buzz-relay --release


# Run the desktop Tauri app in dev mode with a local relay (ports and identity derived from worktree)
dev *ARGS: bootstrap _ensure-sidecar-stubs _ensure-migrations
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    bind_addr="${BUZZ_BIND_ADDR:-0.0.0.0:3000}"
    relay_port="${bind_addr##*:}"; [[ -n "$relay_port" ]] || relay_port=3000
    health_port="${BUZZ_HEALTH_PORT:-8080}"
    metrics_port="${BUZZ_METRICS_PORT:-9102}"
    if command -v lsof >/dev/null 2>&1; then
        for spec in "relay:$relay_port" "health:$health_port" "metrics:$metrics_port"; do
            name="${spec%%:*}"; port="${spec##*:}"
            if lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1; then
                echo "Error: $name port $port is already in use; refusing to launch desktop against a stale relay." >&2
                lsof -nP -iTCP:"$port" -sTCP:LISTEN >&2 || true
                echo "Stop the process above (often a stale buzz-relay) and rerun: just dev" >&2
                exit 1
            fi
        done
    fi
    cargo build -p buzz-acp -p buzz-agent -p buzz-dev-mcp -p buzz-cli -p git-credential-nostr -p buzz-relay
    if [[ -n "{{mesh}}" ]]; then
        export MESH_LLM_NATIVE_RUNTIME_CACHE_DIR="$(./scripts/ensure-mesh-native-runtime.sh)"
    fi
    # Docker Desktop's forwarded MinIO port can stall under the deployment
    # probe's 32 concurrent writers. Keep the gate enabled in local dev, using
    # the bounded profile already used by the relay test launcher.
    export BUZZ_GIT_PROBE_WRITERS="${BUZZ_GIT_PROBE_WRITERS:-8}"
    export BUZZ_GIT_PROBE_ROUNDS="${BUZZ_GIT_PROBE_ROUNDS:-2}"
    ./target/debug/buzz-relay &
    RELAY_PID=$!
    cleanup() {
        [[ -n "${INSTANCE_ID:-}" ]] && ../scripts/cleanup-instance-agents.sh "$INSTANCE_ID" || true
        kill "$RELAY_PID" 2>/dev/null || true
    }
    trap cleanup EXIT
    relay_ready=false
    for _ in $(seq 1 120); do
        if ! kill -0 "$RELAY_PID" 2>/dev/null; then
            echo "Error: buzz-relay exited during startup; refusing to launch desktop." >&2
            wait "$RELAY_PID" || true
            exit 1
        fi
        if curl --silent --fail --max-time 1 "http://127.0.0.1:${health_port}/_readiness" >/dev/null; then
            relay_ready=true
            break
        fi
        sleep 0.5
    done
    if [[ "$relay_ready" != true ]]; then
        echo "Error: buzz-relay did not become healthy within 60 seconds; refusing to launch desktop." >&2
        exit 1
    fi
    cd {{desktop_dir}}
    [[ -d node_modules ]] || pnpm install
    source ../scripts/instance-env.sh
    INSTANCE_ID=$(node -e "console.log(JSON.parse(process.env.BUZZ_TAURI_CONFIG).identifier)")
    echo "Starting on Vite port ${BUZZ_VITE_PORT}, relay ${BUZZ_RELAY_URL}"
    FEATURES=(); [[ -n "{{mesh}}" ]] && FEATURES=(--features mesh-llm)
    pnpm exec tauri dev ${FEATURES[@]+"${FEATURES[@]}"} --config "$BUZZ_TAURI_CONFIG" {{ARGS}}

# Run only the desktop app. No relay, database, Docker, migrations, or .env are needed.
# The app opens normally and asks for a community before making a relay connection.
desktop-standalone *ARGS: _ensure-sidecar-stubs
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    cargo build -p buzz-acp -p buzz-agent -p buzz-dev-mcp -p buzz-cli -p git-credential-nostr
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    TARGET_DIR=$(cargo metadata --format-version 1 --no-deps | node -p "JSON.parse(require('fs').readFileSync(0, 'utf8')).target_directory")
    for bin in buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz; do
        cp "${TARGET_DIR}/debug/${bin}" "desktop/src-tauri/binaries/${bin}-${TARGET}"
        chmod +x "desktop/src-tauri/binaries/${bin}-${TARGET}"
    done
    cd {{desktop_dir}}
    [[ -d node_modules ]] || pnpm install
    unset BUZZ_PRIVATE_KEY BUZZ_SHARE_IDENTITY
    if [[ -n "{{fresh}}" ]]; then
        export BUZZ_RESET_WEBVIEW_STATE=1
    fi
    source ../scripts/instance-env.sh
    INSTANCE_ID=$(node -e "console.log(JSON.parse(process.env.BUZZ_TAURI_CONFIG).identifier)")
    export BUZZ_DEV_KEYRING_SERVICE="buzz-desktop-dev.${BUZZ_INSTANCE_SLUG:-main}"
    if [[ -n "{{fresh}}" ]]; then
        ../scripts/reset-desktop-standalone-state.sh "$INSTANCE_ID" "$BUZZ_DEV_KEYRING_SERVICE"
    fi
    trap '../scripts/cleanup-instance-agents.sh "$INSTANCE_ID" || true' EXIT
    echo "Starting standalone desktop on Vite port ${BUZZ_VITE_PORT}; no relay services were started"
    pnpm exec tauri dev --config "$BUZZ_TAURI_CONFIG" {{ARGS}}

# Run the desktop app against the internal staging relay (installs deps + builds agent tools automatically)
staging *ARGS: bootstrap _ensure-sidecar-stubs
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    pnpm install  # unconditional: staging must always start with a clean dep tree
    cargo build --release -p buzz-acp -p buzz-agent -p buzz-dev-mcp -p buzz-cli -p git-credential-nostr
    FEATURES=()
    if [[ -n "{{mesh}}" ]]; then
        FEATURES=(--features mesh-llm)
        export MESH_LLM_NATIVE_RUNTIME_CACHE_DIR="$(./scripts/ensure-mesh-native-runtime.sh)"
    fi
    # Replace the 0-byte sidecar stub with the real CLI binary so tauri dev picks it up.
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    TARGET_DIR=$(cargo metadata --format-version 1 --no-deps | node -p "JSON.parse(require('fs').readFileSync(0, 'utf8')).target_directory")
    cp "${TARGET_DIR}/release/buzz" "desktop/src-tauri/binaries/buzz-${TARGET}"
    chmod +x "desktop/src-tauri/binaries/buzz-${TARGET}"
    cd {{desktop_dir}}
    export BUZZ_RELAY_URL="wss://sprout-oss.stage.blox.sqprod.co"
    source ../scripts/instance-env.sh
    # Ctrl+C kills the Tauri app before its in-process sweep finishes, leaking
    # agent workers. Reap this instance's agents on exit as a backstop.
    INSTANCE_ID=$(node -e "console.log(JSON.parse(process.env.BUZZ_TAURI_CONFIG).identifier)")
    trap '../scripts/cleanup-instance-agents.sh "$INSTANCE_ID" || true' EXIT
    echo "Starting staging on Vite port ${BUZZ_VITE_PORT}, relay ${BUZZ_RELAY_URL}"
    pnpm exec tauri dev ${FEATURES[@]+"${FEATURES[@]}"} --config "$BUZZ_TAURI_CONFIG" {{ARGS}}

# Run the desktop frontend dev server (port derived from worktree)
desktop-dev:
    #!/usr/bin/env bash
    set -euo pipefail
    cd {{desktop_dir}}
    [[ -d node_modules ]] || pnpm install
    source ../scripts/instance-env.sh
    echo "Starting frontend dev server on Vite port ${BUZZ_VITE_PORT}, relay ${BUZZ_RELAY_URL}"
    pnpm exec vite --port "${BUZZ_VITE_PORT}" --strictPort

# ─── Web ─────────────────────────────────────────────────────────────────────

# Run the web frontend dev server (port derived from worktree to avoid collisions)
web:
    #!/usr/bin/env bash
    set -euo pipefail
    [[ -d node_modules ]] || pnpm install
    source scripts/instance-env.sh
    export VITE_PORT=$((BUZZ_VITE_PORT + 100))
    export VITE_RELAY_URL="${BUZZ_RELAY_URL}"
    echo "Starting web dev server on port ${VITE_PORT}, relay ${BUZZ_RELAY_URL}"
    cd {{web_dir}}
    pnpm exec vite --port "${VITE_PORT}" --strictPort

# Run web lint and format checks
web-check:
    cd {{web_dir}} && pnpm check

# Fix web lint and format issues
web-fix:
    cd {{web_dir}} && pnpm exec biome check --write . && pnpm check:file-sizes

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

# Format all Dart code
mobile-fmt:
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && dart format .

# Fix mobile formatting and run analysis
mobile-fix:
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && dart format . && flutter analyze

# Run mobile lint and format checks
mobile-check:
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && dart format --output=none --set-exit-if-changed . && flutter analyze && node ./scripts/check-file-sizes.mjs

# Run mobile tests
mobile-test:
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && flutter test

# Compile an unsigned Android debug APK
mobile-build-android:
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && flutter build apk --debug --no-pub

# Run the mobile app on iOS simulator
mobile-dev:
    #!/usr/bin/env bash
    set -euo pipefail
    if ! pgrep -x Simulator &>/dev/null; then
        open -a Simulator
        sleep 3
    fi
    cd {{mobile_dir}}
    unset GIT_DIR GIT_WORK_TREE
    flutter run

# ─── Database ─────────────────────────────────────────────────────────────────

# Apply database migrations
migrate: _ensure-migrations

# ─── Utilities ────────────────────────────────────────────────────────────────

# Remove build artifacts
clean:
    cargo clean
    cargo clean --manifest-path desktop/src-tauri/Cargo.toml

# Check the Rust workspace compiles without producing binaries
check-compile:
    cargo check --workspace --all-targets

# ─── Release ─────────────────────────────────────────────────────────────────

# Read the current desktop version from package.json
get-current-version:
    @node -p "require('./desktop/package.json').version"

# Read the current relay version from its crate manifest
get-current-relay-version:
    @grep -m1 '^version = ' crates/buzz-relay/Cargo.toml | sed -E 's/version = "(.*)"/\1/'

# Compute next minor version (e.g., 0.3.0 → 0.4.0)
get-next-minor-version:
    @python3 -c "v='$(just get-current-version)'.split('.'); print(f'{v[0]}.{int(v[1])+1}.0')"

# Compute next patch version (e.g., 0.3.0 → 0.3.1)
get-next-patch-version:
    @python3 -c "v='$(just get-current-version)'.split('.'); print(f'{v[0]}.{v[1]}.{int(v[2])+1}')"

# Compute next relay patch version (e.g., 0.3.0 → 0.3.1)
get-next-relay-patch-version:
    @python3 -c "v='$(just get-current-relay-version)'.split('.'); print(f'{v[0]}.{v[1]}.{int(v[2])+1}')"

# Read the current mobile version from pubspec.yaml (strips the +build suffix)
get-current-mobile-version:
    @grep -m1 '^version: ' mobile/pubspec.yaml | sed -E 's/version: ([^+]*).*/\1/'

# Compute next mobile patch version (e.g., 0.3.0 → 0.3.1)
get-next-mobile-patch-version:
    @python3 -c "v='$(just get-current-mobile-version)'.split('.'); print(f'{v[0]}.{v[1]}.{int(v[2])+1}')"

# Update version in desktop package manifests and regenerate lockfiles
bump-desktop-version version:
    #!/usr/bin/env bash
    set -euo pipefail
    # desktop/package.json
    cd desktop && npm pkg set "version={{ version }}" && cd ..
    # desktop/src-tauri/tauri.conf.json
    node -e "
        const fs = require('fs');
        const p = 'desktop/src-tauri/tauri.conf.json';
        const c = JSON.parse(fs.readFileSync(p, 'utf8'));
        c.version = '{{ version }}';
        fs.writeFileSync(p, JSON.stringify(c, null, 2) + '\n');
    "
    # JSON.stringify expands arrays/objects in a way biome rejects; reformat to match.
    (cd desktop && pnpm exec biome format --write src-tauri/tauri.conf.json)
    # desktop/src-tauri/Cargo.toml — only first version line (under [package])
    node -e "
        const fs = require('fs');
        const p = 'desktop/src-tauri/Cargo.toml';
        let t = fs.readFileSync(p, 'utf8');
        t = t.replace(/^version = \".*\"/m, 'version = \"{{ version }}\"');
        fs.writeFileSync(p, t);
    "
    # Regenerate lockfiles
    pnpm install --lockfile-only
    cargo update -p buzz-desktop --manifest-path desktop/src-tauri/Cargo.toml
    echo "Bumped desktop manifests to {{ version }} and regenerated lockfiles"

# Bump the relay crate version and regenerate the lockfile
bump-relay-version version:
    #!/usr/bin/env bash
    set -euo pipefail
    # buzz-relay carries its own `version =` (not version.workspace), so the
    # replace targets the package version line only.
    perl -i -pe 's/^version = ".*"/version = "{{ version }}"/' crates/buzz-relay/Cargo.toml
    cargo update -p buzz-relay
    echo "Bumped buzz-relay to {{ version }} and regenerated Cargo.lock"

# Bump the mobile pubspec version and regenerate the lockfile
bump-mobile-version version:
    #!/usr/bin/env bash
    set -euo pipefail
    # pubspec carries a `version: X.Y.Z+build`; preserve the `+build` convention
    # (a literal `+1`, matching the desktop lane's prior behavior).
    perl -i -pe 's/^version: .*/version: {{ version }}+1/' mobile/pubspec.yaml
    (unset GIT_DIR GIT_WORK_TREE; cd mobile && flutter pub get)
    echo "Bumped mobile to {{ version }} and regenerated pubspec.lock"

# Open or update the desktop release PR (signed desktop app)
release-desktop *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    ARG="{{ ARGS }}"
    if [[ -z "$ARG" || "$ARG" == "patch" ]]; then
        VERSION=$(just get-next-patch-version)
    else
        VERSION="$ARG"
    fi
    just _release-pr desktop "$VERSION"

# Open or update the relay release PR (ghcr.io/block/buzz image)
release-relay *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    ARG="{{ ARGS }}"
    if [[ -z "$ARG" || "$ARG" == "patch" ]]; then
        VERSION=$(just get-next-relay-patch-version)
    else
        VERSION="$ARG"
    fi
    just _release-pr relay "$VERSION"

# Open or update the mobile release PR (Buzz mobile app)
release-mobile *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    ARG="{{ ARGS }}"
    if [[ -z "$ARG" || "$ARG" == "patch" ]]; then
        VERSION=$(just get-next-mobile-patch-version)
    else
        VERSION="$ARG"
    fi
    just _release-pr mobile "$VERSION"

# Shared release-PR engine. One body, three lanes — the only lane-specific steps
# are the version-bump command and the file/tag/changelog identifiers selected
# in the `case` below. Everything else (git preflight, branch reset, changelog
# generation, commit, push, PR open/edit) is identical across lanes.
_release-pr lane version:
    #!/usr/bin/env bash
    set -euo pipefail
    VERSION="{{ version }}"
    if ! echo "$VERSION" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'; then
        echo "Error: '$VERSION' is not valid semver (expected X.Y.Z)"
        exit 1
    fi
    # Lane-specific identifiers. The bump command runs after the branch switch.
    case "{{ lane }}" in
        desktop)
            BRANCH_PREFIX="version-bump"
            TAG_FETCH='v*'
            TAG_MATCH='v[0-9]*'
            TAG_EXCLUDE='*-*'
            TAG_PREFIX="v"
            CHANGELOG="CHANGELOG.md"
            ADD_FILES=(desktop/package.json desktop/src-tauri/tauri.conf.json desktop/src-tauri/Cargo.toml desktop/src-tauri/Cargo.lock pnpm-lock.yaml CHANGELOG.md)
            LOG_PATHS=(desktop/ crates/buzz-core/ crates/buzz-persona/ crates/buzz-sdk/ crates/buzz-agent/)
            ARTIFACT="Buzz Desktop" ;;
        relay)
            BRANCH_PREFIX="relay-release"
            TAG_FETCH='relay-v*'
            TAG_MATCH='relay-v[0-9]*'
            TAG_EXCLUDE='relay-v*-*'
            TAG_PREFIX="relay-v"
            CHANGELOG="crates/buzz-relay/CHANGELOG.md"
            ADD_FILES=(crates/buzz-relay/Cargo.toml Cargo.lock crates/buzz-relay/CHANGELOG.md)
            LOG_PATHS=(crates/buzz-relay/ crates/buzz-core/ crates/buzz-db/ crates/buzz-auth/ crates/buzz-pubsub/ crates/buzz-search/ crates/buzz-audit/ crates/buzz-media/ crates/buzz-sdk/ crates/buzz-workflow/ crates/buzz-conformance/ migrations/)
            ARTIFACT="Buzz Relay" ;;
        mobile)
            BRANCH_PREFIX="mobile-release"
            TAG_FETCH='mobile-v*'
            TAG_MATCH='mobile-v[0-9]*'
            TAG_EXCLUDE='mobile-v*-*'
            TAG_PREFIX="mobile-v"
            CHANGELOG="mobile/CHANGELOG.md"
            ADD_FILES=(mobile/pubspec.yaml mobile/pubspec.lock mobile/CHANGELOG.md)
            LOG_PATHS=(mobile/)
            ARTIFACT="Buzz Mobile" ;;
        *)
            echo "Error: unknown release lane '{{ lane }}'"
            exit 1 ;;
    esac
    echo "Preparing ${ARTIFACT} release v${VERSION}..."
    # Must run on main with a clean, up-to-date tree.
    CURRENT_BRANCH=$(git symbolic-ref --short HEAD)
    if [[ "$CURRENT_BRANCH" != "main" ]]; then
        echo "Error: must be on main branch (currently on '$CURRENT_BRANCH')"
        exit 1
    fi
    git fetch origin refs/heads/main:refs/remotes/origin/main --no-tags
    # Release tags are remote-owned state; sync only this lane's tags so stale
    # local tags from older histories do not make release preflight fail.
    git fetch origin "+refs/tags/${TAG_FETCH}:refs/tags/${TAG_FETCH}"
    if [[ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]]; then
        echo "Error: local main is not up-to-date with origin/main. Run 'git pull' first."
        exit 1
    fi
    if ! git diff --quiet || ! git diff --cached --quiet; then
        echo "Error: working tree is dirty. Commit or stash changes first."
        exit 1
    fi
    # Switch to the release branch (create, or reset to main if it exists).
    BRANCH="${BRANCH_PREFIX}/${VERSION}"
    if git rev-parse --verify "refs/heads/$BRANCH" >/dev/null 2>&1; then
        echo "Branch '$BRANCH' already exists — resetting to origin/main..."
        git switch "$BRANCH"
        git reset --hard origin/main
    elif git ls-remote --exit-code --heads origin "$BRANCH" >/dev/null 2>&1; then
        echo "Branch '$BRANCH' exists on remote — checking out and resetting to origin/main..."
        git switch -c "$BRANCH" --track "origin/$BRANCH"
        git reset --hard origin/main
    else
        git switch -c "$BRANCH"
    fi
    # Lane-specific bump (the one diverging step).
    case "{{ lane }}" in
        desktop) just bump-desktop-version "$VERSION" ;;
        relay)   just bump-relay-version "$VERSION" ;;
        mobile)  just bump-mobile-version "$VERSION" ;;
    esac
    # Generate the changelog from commits since this lane's last release tag.
    LAST_TAG=$(git describe --tags --abbrev=0 --match "$TAG_MATCH" --exclude "$TAG_EXCLUDE" 2>/dev/null || echo "")
    REPO=$(git remote get-url origin | sed -E 's|.*github\.com[:/]||; s|\.git$||')
    format_log() {
        local range="$1"
        git log "$range" --format="%h %H %s" --no-merges -- "${LOG_PATHS[@]}" | while IFS=' ' read -r short full rest; do
            local pr subject
            pr=$(printf '%s' "$rest" | grep -oE '\(#[0-9]+\)$' | grep -oE '[0-9]+' || true)
            if [[ -n "$pr" ]]; then
                subject=$(printf '%s' "$rest" | sed -E 's/ \(#[0-9]+\)$//')
                printf -- '- %s ([#%s](https://github.com/%s/pull/%s)) ([`%s`](https://github.com/%s/commit/%s))\n' \
                    "$subject" "$pr" "$REPO" "$pr" "$short" "$REPO" "$full"
            else
                printf -- '- %s ([`%s`](https://github.com/%s/commit/%s))\n' \
                    "$rest" "$short" "$REPO" "$full"
            fi
        done
    }
    TMPFILE=$(mktemp)
    {
        echo "# Changelog"
        echo ""
        echo "## ${TAG_PREFIX}${VERSION}"
        echo ""
        if [[ -n "$LAST_TAG" ]]; then
            format_log "${LAST_TAG}..HEAD"
        else
            echo "- Initial release"
        fi
        echo ""
        if [[ -f "$CHANGELOG" ]]; then
            tail -n +2 "$CHANGELOG"
        fi
    } > "$TMPFILE"
    mkdir -p "$(dirname "$CHANGELOG")"
    mv "$TMPFILE" "$CHANGELOG"
    # Commit.
    git add "${ADD_FILES[@]}"
    RELEASE_MSG="chore(release): release ${ARTIFACT} version ${VERSION}"
    if [[ "$(git log -1 --format='%s' 2>/dev/null)" == "$RELEASE_MSG" ]]; then
        git commit --amend --no-edit
    else
        git commit -m "$RELEASE_MSG"
    fi
    # Push and open/update the PR.
    git push --force-with-lease -u origin "$BRANCH"
    PR_BODY="## ${ARTIFACT} release v${VERSION}"$'\n\n'
    if [[ -n "$LAST_TAG" ]]; then
        PR_BODY+="### Changes since ${LAST_TAG}:"$'\n\n'
        CHANGELOG_BODY=$(format_log "${LAST_TAG}..HEAD~1")
        MAX_LOG=62000
        if (( ${#CHANGELOG_BODY} > MAX_LOG )); then
            TRUNCATED=$(printf '%s' "$CHANGELOG_BODY" | awk -v max="$MAX_LOG" \
                'BEGIN{n=0} {line_len=length($0)+1; if(n+line_len>max) exit; n+=line_len; print}')
            SHOWN=$(printf '%s\n' "$TRUNCATED" | grep -c '^-' || true)
            TOTAL=$(printf '%s\n' "$CHANGELOG_BODY" | grep -c '^-' || true)
            SKIPPED=$(( TOTAL - SHOWN ))
            CHANGELOG_BODY="${TRUNCATED}"$'\n'"_… and ${SKIPPED} more commits — [compare ${LAST_TAG}…${TAG_PREFIX}${VERSION}](https://github.com/${REPO}/compare/${LAST_TAG}...${TAG_PREFIX}${VERSION})_"
        fi
        PR_BODY+="${CHANGELOG_BODY}"$'\n\n'
    else
        PR_BODY+="Initial release."$'\n\n'
    fi
    PR_BODY+="**To release:** merge this PR. The tag and build will happen automatically."
    PR_TITLE="chore(release): release ${ARTIFACT} version ${VERSION}"
    EXISTING_PR=$(gh pr list --head "$BRANCH" --json url --jq '.[0].url' 2>/dev/null || true)
    if [[ -n "$EXISTING_PR" ]]; then
        gh pr edit "$BRANCH" --title "$PR_TITLE" --body "$PR_BODY"
        PR_URL="$EXISTING_PR"
        echo ""
        echo "Updated existing release PR: ${PR_URL}"
    else
        PR_URL=$(gh pr create --title "$PR_TITLE" --body "$PR_BODY")
        echo ""
        echo "Release PR opened: ${PR_URL}"
    fi
    echo "Merge it to trigger the release build."

# ─── Agent Harness ────────────────────────────────────────────────────────────

# Run a goose agent connected to a Buzz relay (foreground)
goose relay="ws://localhost:3000" agents="1" heartbeat="0" prompt="" key="$BUZZ_PRIVATE_KEY":
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    source ./scripts/_goose-env.sh "{{relay}}" "{{key}}" "{{agents}}" "{{heartbeat}}" "{{prompt}}"
    exec env "${env_args[@]}" ./target/release/buzz-acp

# Run a goose agent in the background (screen session named 'goose-agent-N')
goose-bg relay="ws://localhost:3000" agents="1" heartbeat="0" prompt="" key="$BUZZ_PRIVATE_KEY":
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    source ./scripts/_goose-env.sh "{{relay}}" "{{key}}" "{{agents}}" "{{heartbeat}}" "{{prompt}}"
    screen -dmS goose-agent-{{agents}} bash -c "$(printf '%q ' env "${env_args[@]}") ./target/release/buzz-acp"
    echo "Agent running in screen session 'goose-agent-{{agents}}'. Attach with: screen -r goose-agent-{{agents}}"

# ─── Benchmarking ─────────────────────────────────────────────────────────────

# Run the Buzz orchestra benchmark — leaderboard-eligible by default (TB 2.1, k=5, Sonnet+Haiku). Stands up its own Docker stack; --gui opens a live spectator desktop app; other flags pass to benchmark.py (--dataset/--path, --include-task, --attempts, --manifest, --dry-run, ...)
benchmark *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    uv run --project benchmarks/harbor-buzz-orchestra/testbed \
        benchmarks/harbor-buzz-orchestra/scripts/benchmark.py {{ARGS}}

# Stop the benchmark Docker stack (state and channels are kept)
benchmark-down:
    docker compose --project-name buzz-benchmark down
