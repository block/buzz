# Buzz — development task runner

set dotenv-load := true

desktop_dir := "desktop"
desktop_tauri_manifest := "desktop/src-tauri/Cargo.toml"
web_dir := "web"

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

# Install git hooks via lefthook
hooks:
    git config --local core.hooksPath .hooks
    lefthook install --force

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

# Apply database migrations if the dev database is running
_ensure-migrations: _ensure-services
    cargo run -p buzz-admin -- migrate

# Run clippy on the desktop Tauri Rust crate
desktop-tauri-clippy: _ensure-sidecar-stubs
    cargo clippy --manifest-path {{desktop_tauri_manifest}} --all-targets -- -D warnings

# Check the desktop Tauri Rust crate compiles
desktop-tauri-check: _ensure-sidecar-stubs
    cargo check --manifest-path {{desktop_tauri_manifest}}

# Run desktop Tauri Rust unit tests
desktop-tauri-test: _ensure-sidecar-stubs
    cd desktop/src-tauri && cargo test

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
    else
        ./scripts/run-tests.sh unit
    fi

# Run integration tests only (starts services if needed)
test-integration:
    ./scripts/run-tests.sh integration

# Mesh-compute e2e: the CI-safe layers (relay mesh signaling invariants + Playwright UI)
mesh-e2e:
    cargo test -p buzz-relay mesh_signaling
    cd {{desktop_dir}} && pnpm test:e2e:integration -- mesh-compute.spec.ts

# Mesh-compute Layer 1: REAL serve->client->inference on this machine (not CI)
mesh-e2e-hardware:
    #!/usr/bin/env bash
    set -euo pipefail
    export MESH_LLM_NATIVE_RUNTIME_CACHE_DIR="$(./scripts/ensure-mesh-native-runtime.sh)"
    cargo run -p buzz-relay --example mesh_serve_client_smoke

# Take desktop screenshots using the mock bridge
desktop-screenshot *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    just desktop-build
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

# Start the relay server in release mode
relay-release: _ensure-migrations
    cargo run -p buzz-relay --release

# Start buzz-proxy (dev mode)
proxy:
    cargo run -p buzz-proxy

# Start buzz-proxy (release mode)
proxy-release:
    cargo run -p buzz-proxy --release

# Run the desktop Tauri app in dev mode with a local relay (ports and identity derived from worktree)
dev *ARGS: bootstrap _ensure-sidecar-stubs _ensure-migrations
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    cargo build -p buzz-acp -p buzz-agent -p buzz-dev-mcp -p buzz-cli -p git-credential-nostr -p buzz-relay
    ./target/debug/buzz-relay &
    RELAY_PID=$!
    cleanup() {
        [[ -n "${INSTANCE_ID:-}" ]] && ../scripts/cleanup-instance-agents.sh "$INSTANCE_ID" || true
        kill "$RELAY_PID" 2>/dev/null || true
    }
    trap cleanup EXIT
    cd {{desktop_dir}}
    [[ -d node_modules ]] || pnpm install
    source ../scripts/instance-env.sh
    INSTANCE_ID=$(node -e "console.log(JSON.parse(process.env.BUZZ_TAURI_CONFIG).identifier)")
    echo "Starting on Vite port ${BUZZ_VITE_PORT}, relay ${BUZZ_RELAY_URL}"
    pnpm exec tauri dev --features mesh-llm --config "$BUZZ_TAURI_CONFIG" {{ARGS}}

# Run the desktop app against the internal staging relay (installs deps + builds agent tools automatically)
staging *ARGS: bootstrap _ensure-sidecar-stubs
    #!/usr/bin/env bash
    set -euo pipefail
    export PATH="{{justfile_directory()}}/bin:$PATH"
    pnpm install  # unconditional: staging must always start with a clean dep tree
    cargo build --release -p buzz-acp -p buzz-agent -p buzz-dev-mcp -p buzz-cli -p git-credential-nostr
    export MESH_LLM_NATIVE_RUNTIME_CACHE_DIR="$(./scripts/ensure-mesh-native-runtime.sh)"
    # Replace the 0-byte sidecar stub with the real CLI binary so tauri dev picks it up.
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    cp target/release/buzz "desktop/src-tauri/binaries/buzz-${TARGET}"
    chmod +x "desktop/src-tauri/binaries/buzz-${TARGET}"
    cd {{desktop_dir}}
    export BUZZ_RELAY_URL="wss://sprout-oss.stage.blox.sqprod.co"
    source ../scripts/instance-env.sh
    # Ctrl+C kills the Tauri app before its in-process sweep finishes, leaking
    # agent workers. Reap this instance's agents on exit as a backstop.
    INSTANCE_ID=$(node -e "console.log(JSON.parse(process.env.BUZZ_TAURI_CONFIG).identifier)")
    trap '../scripts/cleanup-instance-agents.sh "$INSTANCE_ID" || true' EXIT
    echo "Starting staging on Vite port ${BUZZ_VITE_PORT}, relay ${BUZZ_RELAY_URL}"
    pnpm exec tauri dev --features mesh-llm --config "$BUZZ_TAURI_CONFIG" {{ARGS}}

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
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && dart format --output=none --set-exit-if-changed . && flutter analyze

# Run mobile tests
mobile-test:
    unset GIT_DIR GIT_WORK_TREE; cd {{mobile_dir}} && flutter test

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

# Rebuild Typesense docs for all kind:0 (user profile) events.
# Required once after deploying the indexer change that flattens kind:0 content
# for searchability; new/updated profiles are indexed correctly automatically.
# Safe to run repeatedly — Typesense upserts.
reindex-kind0:
    cargo run --release -p buzz-relay --bin buzz-reindex-kind0

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

# Compute next minor version (e.g., 0.3.0 → 0.4.0)
get-next-minor-version:
    @python3 -c "v='$(just get-current-version)'.split('.'); print(f'{v[0]}.{int(v[1])+1}.0')"

# Compute next patch version (e.g., 0.3.0 → 0.3.1)
get-next-patch-version:
    @python3 -c "v='$(just get-current-version)'.split('.'); print(f'{v[0]}.{v[1]}.{int(v[2])+1}')"

# Update version in all package manifests and regenerate lockfiles
bump-version version:
    #!/usr/bin/env bash
    set -euo pipefail
    # Validate semver format
    if ! echo "{{ version }}" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'; then
        echo "Error: '{{ version }}' is not valid semver (expected X.Y.Z)"
        exit 1
    fi
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
    # mobile/pubspec.yaml — bump version but preserve build number
    sed -i '' "s/^version: .*/version: {{ version }}+1/" mobile/pubspec.yaml
    # Regenerate lockfiles
    pnpm install --lockfile-only
    cargo update -p buzz-desktop --manifest-path desktop/src-tauri/Cargo.toml
    (unset GIT_DIR GIT_WORK_TREE; cd mobile && flutter pub get)
    echo "Bumped all manifests to {{ version }} and regenerated lockfiles"

# Create or update a release PR that bumps version and generates changelog
release *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    # Determine target version
    ARG="{{ ARGS }}"
    if [[ -z "$ARG" ]]; then
        VERSION=$(just get-next-patch-version)
    elif [[ "$ARG" == "patch" ]]; then
        VERSION=$(just get-next-patch-version)
    else
        VERSION="$ARG"
    fi
    echo "Preparing release v${VERSION}..."
    # Ensure on main branch
    CURRENT_BRANCH=$(git symbolic-ref --short HEAD)
    if [[ "$CURRENT_BRANCH" != "main" ]]; then
        echo "Error: must be on main branch (currently on '$CURRENT_BRANCH')"
        exit 1
    fi
    # Ensure local main and release tags are up-to-date.
    git fetch origin refs/heads/main:refs/remotes/origin/main --no-tags
    # Release tags are remote-owned state; sync only v* tags so stale local
    # tags from older histories do not make release preflight fail.
    git fetch origin '+refs/tags/v*:refs/tags/v*'
    if [[ "$(git rev-parse HEAD)" != "$(git rev-parse origin/main)" ]]; then
        echo "Error: local main is not up-to-date with origin/main. Run 'git pull' first."
        exit 1
    fi
    # Ensure clean working tree
    if ! git diff --quiet || ! git diff --cached --quiet; then
        echo "Error: working tree is dirty. Commit or stash changes first."
        exit 1
    fi
    # Switch to version-bump branch (create if needed, reset to main if it exists)
    BRANCH="version-bump/${VERSION}"
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
    # Bump versions and lockfiles
    just bump-version "$VERSION"
    # Generate changelog
    LAST_TAG=$(git describe --tags --abbrev=0 --match 'v[0-9]*' --exclude '*-*' 2>/dev/null || echo "")
    REPO=$(git remote get-url origin | sed -E 's|.*github\.com[:/]||; s|\.git$||')
    format_log() {
        local range="$1"
        git log "$range" --format="%h %H %s" --no-merges | while IFS=' ' read -r short full rest; do
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
        echo "## v${VERSION}"
        echo ""
        if [[ -n "$LAST_TAG" ]]; then
            format_log "${LAST_TAG}..HEAD"
        else
            echo "- Initial release"
        fi
        echo ""
        if [[ -f CHANGELOG.md ]]; then
            tail -n +2 CHANGELOG.md
        fi
    } > "$TMPFILE"
    mv "$TMPFILE" CHANGELOG.md
    # Commit
    git add \
      desktop/package.json \
      desktop/src-tauri/tauri.conf.json \
      desktop/src-tauri/Cargo.toml \
      desktop/src-tauri/Cargo.lock \
      mobile/pubspec.yaml \
      mobile/pubspec.lock \
      pnpm-lock.yaml \
      CHANGELOG.md
    RELEASE_MSG="chore(release): release version ${VERSION}"
    if [[ "$(git log -1 --format='%s' 2>/dev/null)" == "$RELEASE_MSG" ]]; then
        git commit --amend --no-edit
    else
        git commit -m "$RELEASE_MSG"
    fi
    # Push and open PR
    git push --force-with-lease -u origin "$BRANCH"
    # Build PR body
    PR_BODY="## Release v${VERSION}"$'\n\n'
    if [[ -n "$LAST_TAG" ]]; then
        PR_BODY+="### Changes since ${LAST_TAG}:"$'\n\n'
        PR_BODY+="$(format_log "${LAST_TAG}..HEAD~1")"$'\n\n'
    else
        PR_BODY+="Initial release."$'\n\n'
    fi
    PR_BODY+="**To release:** merge this PR. The tag and build will happen automatically."
    EXISTING_PR=$(gh pr list --head "$BRANCH" --json url --jq '.[0].url' 2>/dev/null || true)
    if [[ -n "$EXISTING_PR" ]]; then
        gh pr edit "$BRANCH" \
            --title "chore(release): release version ${VERSION}" \
            --body "$PR_BODY"
        PR_URL="$EXISTING_PR"
        echo ""
        echo "Updated existing release PR: ${PR_URL}"
    else
        PR_URL=$(gh pr create \
            --title "chore(release): release version ${VERSION}" \
            --body "$PR_BODY")
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
