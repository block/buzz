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

# Rebuild Typesense docs for all kind:0 (user profile) events.
# Required once after deploying the indexer change that flattens kind:0 content
# for searchability; new/updated profiles are indexed correctly automatically.
# Safe to run repeatedly — Typesense upserts.
reindex-kind0:
    cargo run --release -p sprout-relay --bin sprout-reindex-kind0

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
_ensure-sidecar-stubs:
    #!/usr/bin/env bash
    set -euo pipefail
    TARGET=$(rustc -vV | sed -n 's|host: ||p')
    mkdir -p desktop/src-tauri/binaries
    for bin in sprout-acp sprout-mcp-server sprout-agent sprout-dev-mcp git-credential-nostr sprout; do
        touch "desktop/src-tauri/binaries/${bin}-${TARGET}"
    done

# Ensure Docker dev services (Postgres, Redis, etc.) are running and healthy
_ensure-services:
    #!/usr/bin/env bash
    set -euo pipefail
    pg=$(docker inspect --format '{{"{{"}}.State.Health.Status{{"}}"}}' sprout-postgres 2>/dev/null || echo "not_found")
    redis=$(docker inspect --format '{{"{{"}}.State.Health.Status{{"}}"}}' sprout-redis 2>/dev/null || echo "not_found")
    if [[ "$pg" == "healthy" && "$redis" == "healthy" ]]; then
        echo "Services already healthy"
        exit 0
    fi
    echo "Starting services..."
    docker compose up -d || true
    echo -n "Waiting for services"
    for i in $(seq 1 40); do
        pg=$(docker inspect --format '{{"{{"}}.State.Health.Status{{"}}"}}' sprout-postgres 2>/dev/null || echo "not_found")
        redis=$(docker inspect --format '{{"{{"}}.State.Health.Status{{"}}"}}' sprout-redis 2>/dev/null || echo "not_found")
        if [[ "$pg" == "healthy" && "$redis" == "healthy" ]]; then
            echo " ready"
            exit 0
        fi
        echo -n "."
        sleep 3
    done
    echo " timed out"
    exit 1

# Apply database migrations if pgschema is available
_ensure-migrations: _ensure-services
    #!/usr/bin/env bash
    set -euo pipefail
    if [[ -x bin/pgschema && -f schema/schema.sql ]]; then
        bin/pgschema apply --file schema/schema.sql --auto-approve || true
    fi

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
        cargo nextest run -p sprout-core -p sprout-auth --lib
    else
        ./scripts/run-tests.sh unit
    fi

# Run integration tests only (starts services if needed)
test-integration:
    ./scripts/run-tests.sh integration

# ─── Run ──────────────────────────────────────────────────────────────────────

# Start the relay server (auto-starts Docker services if needed)
relay: _ensure-migrations
    cargo run -p sprout-relay

# Start the relay with the built web UI served from it
relay-web: _ensure-migrations
    #!/usr/bin/env bash
    set -euo pipefail
    [[ -d node_modules ]] || pnpm install
    pnpm -C web build
    SPROUT_WEB_DIR=./web/dist cargo run -p sprout-relay

# Start the relay server in release mode
relay-release: _ensure-migrations
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

# Apply schema migrations via pgschema
migrate: _ensure-services
    ./bin/pgschema apply --file schema/schema.sql --auto-approve

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
    cargo update -p sprout-desktop --manifest-path desktop/src-tauri/Cargo.toml
    (unset GIT_DIR GIT_WORK_TREE; cd mobile && flutter pub get)
    echo "Bumped all manifests to {{ version }} and regenerated lockfiles"

# Create a release PR that bumps version and generates changelog
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
    # Create version-bump branch
    BRANCH="version-bump/${VERSION}"
    git switch -c "$BRANCH"
    # Bump versions and lockfiles
    just bump-version "$VERSION"
    # Generate changelog
    LAST_TAG=$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2>/dev/null || echo "")
    TMPFILE=$(mktemp)
    {
        echo "# Changelog"
        echo ""
        echo "## v${VERSION}"
        echo ""
        if [[ -n "$LAST_TAG" ]]; then
            git log "${LAST_TAG}..HEAD" --oneline --no-merges
        else
            echo "Initial release"
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
    git commit -m "chore(release): release version ${VERSION}"
    # Push and open PR
    git push -u origin "$BRANCH"
    # Build PR body
    PR_BODY="## Release v${VERSION}"$'\n\n'
    if [[ -n "$LAST_TAG" ]]; then
        PR_BODY+="### Changes since ${LAST_TAG}:"$'\n\n'
        PR_BODY+="$(git log "${LAST_TAG}..HEAD~1" --oneline --no-merges)"$'\n\n'
    else
        PR_BODY+="Initial release."$'\n\n'
    fi
    PR_BODY+="**To release:** merge this PR. The tag and build will happen automatically."
    PR_URL=$(gh pr create \
        --title "chore(release): release version ${VERSION}" \
        --body "$PR_BODY")
    echo ""
    echo "Release PR opened: ${PR_URL}"
    echo "Merge it to trigger the release build."

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
