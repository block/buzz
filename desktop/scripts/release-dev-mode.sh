#!/usr/bin/env bash
# Cut a Buzz Developer Mode release (fork joahg/buzz-dev-mode) with Tauri
# auto-update artifacts.
#
# Usage: desktop/scripts/release-dev-mode.sh <version X.Y.Z> <notes-file.md>
#
# The version is the dev-mode series (tags dev-mode-vX.Y.Z), independent of
# upstream's desktop version. It MUST increase monotonically across dev-mode
# releases or installed apps will never see the update.
#
# What this does:
#   1. Rebuilds sidecars, then builds Buzz.app with the updater configured:
#      version override + updater pubkey + rolling-release endpoint baked in
#      via --config, createUpdaterArtifacts producing Buzz.app.tar.gz + .sig.
#   2. Packages a DMG via hdiutil (Tauri's bundle_dmg.sh Finder automation is
#      flaky locally, so the DMG is always staged manually).
#   3. Tags dev-mode-v<version>, pushes the tag, creates the GitHub prerelease
#      with DMG + updater archive + signature.
#   4. Regenerates latest.json and uploads it (clobber) to the rolling
#      `dev-mode-latest` release that installed apps poll every 6 hours.
#
# The updater private key lives at ~/.tauri/buzz-dev-mode-updater.key
# (override dir with BUZZ_DEVMODE_UPDATER_KEY). Never commit it. Losing it
# means shipping one more manual install.sh release with a fresh keypair.
set -euo pipefail

REPO="joahg/buzz-dev-mode"
FORK_REMOTE="fork"
ROLLING_TAG="dev-mode-latest"
ENDPOINT="https://github.com/${REPO}/releases/download/${ROLLING_TAG}/latest.json"
KEY_FILE="${BUZZ_DEVMODE_UPDATER_KEY:-$HOME/.tauri/buzz-dev-mode-updater.key}"
PUB_FILE="${KEY_FILE}.pub"
PLATFORM_KEY="darwin-aarch64"

DESKTOP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$DESKTOP_DIR/.." && pwd)"

VERSION="${1:?usage: release-dev-mode.sh <version X.Y.Z> <notes-file.md>}"
NOTES_FILE="${2:?usage: release-dev-mode.sh <version X.Y.Z> <notes-file.md>}"
TAG="dev-mode-v${VERSION}"

fail() {
  echo "Error: $*" >&2
  exit 1
}

[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail "version '$VERSION' is not plain X.Y.Z semver"
[[ -f "$NOTES_FILE" ]] || fail "notes file '$NOTES_FILE' not found"
[[ -f "$KEY_FILE" ]] || fail "updater private key missing at $KEY_FILE"
[[ -f "$PUB_FILE" ]] || fail "updater public key missing at $PUB_FILE"
command -v jq >/dev/null || fail "jq is required"
command -v gh >/dev/null || fail "gh is required"

cd "$REPO_ROOT"

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [[ "$BRANCH" != "joah/dev-mode-display-style" && -z "${BUZZ_DEVMODE_RELEASE_ANY_BRANCH:-}" ]]; then
  fail "on branch '$BRANCH', expected joah/dev-mode-display-style (set BUZZ_DEVMODE_RELEASE_ANY_BRANCH=1 to override)"
fi
[[ -z "$(git status --porcelain)" ]] || fail "working tree is dirty; commit or stash first"
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && fail "tag $TAG already exists"
gh release view "$TAG" -R "$REPO" >/dev/null 2>&1 && fail "release $TAG already exists on $REPO"

echo "--- Building sidecars"
cargo build --release -p buzz-acp -p buzz-agent -p buzz-dev-mcp -p git-credential-nostr -p buzz-cli
./scripts/bundle-sidecars.sh

echo "--- Writing devmode release config"
cd "$DESKTOP_DIR"
DEVMODE_CONF="src-tauri/tauri.devmode.conf.json"
jq -n \
  --arg version "$VERSION" \
  --arg pubkey "$(cat "$PUB_FILE")" \
  --arg endpoint "$ENDPOINT" \
  '{
    version: $version,
    bundle: { createUpdaterArtifacts: true },
    plugins: { updater: { pubkey: $pubkey, endpoints: [$endpoint] } }
  }' > "$DEVMODE_CONF"

echo "--- Building Buzz.app ($VERSION)"
TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_FILE")" \
TAURI_SIGNING_PRIVATE_KEY_PASSWORD="" \
  pnpm tauri build --bundles app --config "$DEVMODE_CONF"

BUNDLE_DIR="src-tauri/target/release/bundle"
APP="$BUNDLE_DIR/macos/Buzz.app"
[[ -d "$APP" ]] || fail "expected app bundle at $APP"

TAR_GZ="$(find "$BUNDLE_DIR/macos" -maxdepth 1 -name '*.app.tar.gz' | head -1)"
[[ -n "$TAR_GZ" && -f "${TAR_GZ}.sig" ]] || fail "updater archive or signature missing in $BUNDLE_DIR/macos (createUpdaterArtifacts)"

BUILT_VERSION="$(plutil -extract CFBundleShortVersionString raw "$APP/Contents/Info.plist")"
[[ "$BUILT_VERSION" == "$VERSION" ]] || fail "built app reports version $BUILT_VERSION, expected $VERSION"

echo "--- Packaging DMG"
DIST_DIR="$BUNDLE_DIR/devmode-dist"
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR"

DMG_NAME="Buzz_${TAG}_aarch64.dmg"
ARCHIVE_NAME="Buzz_${TAG}_aarch64.app.tar.gz"
STAGING="$(mktemp -d)/Buzz"
mkdir -p "$STAGING"
cp -R "$APP" "$STAGING/"
ln -s /Applications "$STAGING/Applications"
hdiutil create -volname "Buzz" -srcfolder "$STAGING" -ov -format UDZO "$DIST_DIR/$DMG_NAME"

cp "$TAR_GZ" "$DIST_DIR/$ARCHIVE_NAME"
cp "${TAR_GZ}.sig" "$DIST_DIR/${ARCHIVE_NAME}.sig"

echo "--- Tagging $TAG"
cd "$REPO_ROOT"
git tag "$TAG"
git push "$FORK_REMOTE" "refs/tags/$TAG"

echo "--- Creating release $TAG"
gh release create "$TAG" -R "$REPO" \
  --prerelease \
  --title "Buzz Developer Mode — v${VERSION}" \
  --notes-file "$NOTES_FILE" \
  "$DESKTOP_DIR/$DIST_DIR/$DMG_NAME" \
  "$DESKTOP_DIR/$DIST_DIR/$ARCHIVE_NAME" \
  "$DESKTOP_DIR/$DIST_DIR/${ARCHIVE_NAME}.sig"

echo "--- Refreshing $ROLLING_TAG (latest.json)"
gh release create "$ROLLING_TAG" -R "$REPO" \
  --prerelease --latest=false \
  --title "Buzz Developer Mode — Auto-Update" \
  --notes "Rolling release for the Tauri auto-updater. Do not download manually — use the newest dev-mode-vX.Y.Z release instead." \
  2>/dev/null || true

ARCHIVE_URL="https://github.com/${REPO}/releases/download/${TAG}/${ARCHIVE_NAME}"
"$DESKTOP_DIR/scripts/generate-oss-latest-json.sh" "$VERSION" \
  "${PLATFORM_KEY}:$DESKTOP_DIR/$DIST_DIR/${ARCHIVE_NAME}.sig:${ARCHIVE_URL}" \
  > "$DESKTOP_DIR/$DIST_DIR/latest.json"
gh release upload "$ROLLING_TAG" -R "$REPO" --clobber "$DESKTOP_DIR/$DIST_DIR/latest.json"

echo
echo "Released $TAG: https://github.com/${REPO}/releases/tag/${TAG}"
echo "Auto-update manifest: $ENDPOINT"
echo
echo "Remaining manual steps (per release checklist):"
echo "  - announce in #buzz-dev-mode--releases"
echo "  - changelog to #buzz-dev-mode (Buzz) and #joah-office (Slack, 🤖-prefixed)"
