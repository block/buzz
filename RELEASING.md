# Releasing Buzz

Buzz has three independent release lanes, each driven by a release PR — no human
ever pushes a git tag:

| Lane | Recipe | Artifact |
|------|--------|----------|
| Desktop | `just release-desktop` | Signed desktop app (macOS/Linux) |
| Relay | `just release-relay` | `ghcr.io/block/buzz` container image |
| Mobile | `just release-mobile` | Buzz mobile app (tag is the `sprout_ref` for the internal build) |

The three lanes version independently: the desktop version lives in
`desktop/package.json`, the relay version in `crates/buzz-relay/Cargo.toml`, and
the mobile version in `mobile/pubspec.yaml`.

The mobile lane publishes a `mobile-v<version>` tag that is consumed
**manually**, cross-repo, as the `sprout_ref` input to the internal
`buzz-releases` Buildkite pipeline (iOS dogfood → Block Comp Portal, App Store →
TestFlight — see [Internal Releases](#internal-releases)). The OSS lane is
tag-only **by design**: OSS `block/buzz` CI cannot trigger CI in the private
`buzz-releases` repo (infosec), so a human cuts the internal build from the tag
rather than auto-dispatching across that boundary.

## Quick Start

```sh
# Desktop release (next patch version)
just release-desktop

# Desktop patch / minor / explicit
just release-desktop patch
just release-desktop 0.4.0
just release-desktop 1.0.0

# Relay release (same argument forms)
just release-relay
just release-relay 0.4.0

# Mobile release (same argument forms)
just release-mobile
just release-mobile 0.4.0
```

`just release-desktop` creates a `version-bump/<version>` PR; `just
release-relay` creates a `relay-release/<version>` PR; `just release-mobile`
creates a `mobile-release/<version>` PR. Each bumps its own version manifest,
regenerates lockfiles, and appends a changelog entry. Merge the PR to trigger
the build automatically (the mobile tag is instead the `sprout_ref` a human
feeds the internal build — see above).

Re-running any of these recipes with the same version is safe — it detects the
existing branch and PR, resets to current `main`, regenerates the changelog
with any new commits, and updates the PR in place.

---

## How It Works

All three lanes share one engine; they differ only in which version manifest
they bump, which branch prefix they use, and what the merge triggers.

The merge workflow creates tags with a short-lived installation token from the
dedicated `buzz-release-bot` GitHub App. Release-tag rules allow that App to
create matching tags and prevent other actors from creating, moving, or
deleting them. The workflow's default `GITHUB_TOKEN` is read-only.

### Desktop

1. **`just release-desktop`** runs locally on `main` — computes the next
   version, creates (or reuses) a `version-bump/<version>` branch, bumps the
   desktop manifests, regenerates lockfiles, generates a changelog
   entry in `CHANGELOG.md`, commits, pushes, and opens (or updates) a PR.
2. **Merge the PR** — the `auto-tag-on-release-pr-merge` workflow detects the
   `version-bump/*` branch merge and pushes a `v<version>` tag.
3. **Tag triggers `release.yml`** — builds, signs, notarizes, and publishes the
   desktop app for macOS and Linux.

### Relay

1. **`just release-relay`** runs locally on `main` — computes the next relay
   version, creates (or reuses) a `relay-release/<version>` branch, bumps
   `crates/buzz-relay/Cargo.toml`, regenerates `Cargo.lock`, generates a
   changelog entry in `crates/buzz-relay/CHANGELOG.md`, commits, pushes, and
   opens (or updates) a PR.
2. **Merge the PR** — the `auto-tag-on-release-pr-merge` workflow detects the
   `relay-release/*` branch merge and pushes a `relay-v<version>` tag.
3. **Tag triggers `docker.yml`** — the `relay-v<version>` push triggers
   `docker.yml`, which builds the multi-arch relay
   image and publishes `ghcr.io/block/buzz:<version>` (plus `:<major>.<minor>`,
   `:<major>`, and `:latest` for stable releases). Prereleases
   (`relay-v<version>-rc.1`) publish only the prerelease tag and do **not**
   move `:latest`. GitHub runs the tag trigger because the tag is created by
   the dedicated GitHub App rather than the workflow's `GITHUB_TOKEN`.

Every push to `main` continues to build and publish `:main` + `:sha-<7>` tags
(the rolling development image). The `:latest` tag tracks the latest **stable**
relay release only — it does not move on main pushes or prereleases.

### Mobile

1. **`just release-mobile`** runs locally on `main` — computes the next mobile
   version, creates (or reuses) a `mobile-release/<version>` branch, bumps
   `mobile/pubspec.yaml` (preserving the `+build` number), regenerates
   `mobile/pubspec.lock`, generates a changelog entry in `mobile/CHANGELOG.md`,
   commits, pushes, and opens (or updates) a PR.
2. **Merge the PR** — the `auto-tag-on-release-pr-merge` workflow detects the
   `mobile-release/*` branch merge and pushes a `mobile-v<version>` tag.
3. **The tag is consumed manually, cross-repo** — nothing in OSS `block/buzz`
   builds on the tag (OSS CI must not trigger CI in the private `buzz-releases`
   repo — infosec). A human feeds the `mobile-v<version>` tag as the
   `sprout_ref` input to the internal `buzz-releases` Buildkite pipeline, which
   builds and ships iOS to Block Comp Portal (dogfood) and TestFlight (App
   Store, opt-in). See [Internal Releases](#internal-releases).

---

## Release Types

The argument forms below apply to `release-desktop`, `release-relay`, and
`release-mobile`:

| Command | Version | Example |
|---------|---------|---------|
| `just release-desktop` | Next patch | `0.3.0` → `0.3.1` |
| `just release-desktop patch` | Next patch | `0.3.0` → `0.3.1` |
| `just release-desktop 0.4.0` | Explicit minor | `0.3.1` → `0.4.0` |
| `just release-desktop 1.0.0` | Explicit | `1.0.0` |

---

## Version Files

`just bump-desktop-version <version>` (desktop lane) updates these files:

| File | Field |
|------|-------|
| `desktop/package.json` | `"version"` |
| `desktop/src-tauri/tauri.conf.json` | `"version"` |
| `desktop/src-tauri/Cargo.toml` | `version` (under `[package]`) |

It also regenerates `pnpm-lock.yaml` and `desktop/src-tauri/Cargo.lock`.

`just bump-relay-version <version>` (relay lane) updates
`crates/buzz-relay/Cargo.toml` (`version` under `[package]`) and regenerates the
workspace `Cargo.lock`.

`just bump-mobile-version <version>` (mobile lane) updates
`mobile/pubspec.yaml` (`version:`, preserving the `+build` number) and
regenerates `mobile/pubspec.lock`.

---

## Signed macOS Canary

Use the manual **Signed macOS Canary** workflow when you need an Apple Silicon
build of current `main` for explicit testing without publishing a release:

```sh
gh workflow run signed-macos-canary.yml --repo block/buzz --ref main
```

The workflow derives a `-test.<run-number>` version, signs and notarizes the
DMG, verifies it with Gatekeeper, and uploads it as a short-lived Actions
artifact with seven-day retention. Because this is a public repository, any
signed-in GitHub user can download that artifact while it exists; it is
unpublished, not private. The workflow has no release permissions, does not
create or move tags, and cannot update `buzz-desktop-latest` or `latest.json`.

Download the artifact from the completed run:

```sh
gh run download <run-id> --repo block/buzz --name <artifact-name>
```

The workflow intentionally accepts only `main`. Use the normal release process
for distributable builds or builds from an immutable release tag.

---

## Manual Release Retry

The **Release** workflow's manual dispatch is only a retry mechanism for an
existing immutable `v<version>` tag. Select that tag in the ref picker and
provide the matching semver version without the `v` prefix. It cannot build
from `main` or another caller-selected source ref.

---

## Internal Releases

After the OSS release ships, trigger an internal build via the
[sprout-releases Buildkite pipeline](https://buildkite.com/runway/sprout-releases).
See the [buzz-releases README](https://github.com/squareup/buzz-releases#cutting-a-release)
for the full step-by-step instructions and input field reference.

---

## What Gets Published

Each release produces two GitHub releases:

1. **`v<version>`** — the user-facing release with the `.dmg` installer
   (macOS).

2. **`buzz-desktop-latest`** — a rolling pre-release for the Tauri
   auto-updater containing `latest.json` and each platform's signed
   updater artifact plus its `.sig` signature (`.tar.gz` on macOS,
   `.AppImage` on Linux, and `_alpha-unsigned.exe` on Windows).

---

## Platform Support

The release workflow builds **two separate macOS DMGs** — Apple
Silicon (`darwin-aarch64`, the `release` job) and Intel
(`darwin-x86_64`, the `release-macos-x64` job) — plus Linux `.deb` and
`.AppImage`. Both macOS DMGs are codesigned, notarized, and attached to
the same `v<version>` release. Intel users download the `_x64.dmg`.

The Linux AppImage is post-processed by `desktop/scripts/fix-appimage.sh`,
which strips infra libraries over-bundled by linuxdeploy (they crash on
Mesa 25+ / GLib 2.88 distros — see
[tauri-apps/tauri#15665](https://github.com/tauri-apps/tauri/issues/15665))
and re-signs the artifact. As a result the AppImage relies on the
host's Wayland/GStreamer/graphics stack and requires GLib >= 2.72
(Ubuntu 22.04 or newer). The `release-linux` job builds inside a
`ubuntu:22.04` container for broad GLIBC compatibility.

---

## Prerequisites

- **Write access** to the `block/buzz` GitHub repository
- **`gh` CLI** authenticated (`gh auth status`)
- The following **GitHub Actions secrets** must be configured:

  | Secret | Purpose |
  |--------|---------|
  | `BUZZ_UPDATER_PUBLIC_KEY` | Tauri updater public key (minisign) |
  | `TAURI_SIGNING_PRIVATE_KEY` | Tauri updater private key |
  | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for the private key |

---

## Troubleshooting

### `just release-desktop` fails with "must be on main branch"
Switch to `main` and pull latest before running the release recipe.

### `just release-desktop` fails with "working tree is dirty"
Commit or stash your changes before running the release recipe.

### New commits merged after creating the release PR
Re-run the release recipe (`just release-desktop`, `just release-relay`, or `just release-mobile`) from an up-to-date `main`. It resets the branch to current `main`, regenerates the changelog and PR body to include the new commits, and force-pushes the updated branch.

### Build fails at "Validate version"
The version string must be valid semver: `MAJOR.MINOR.PATCH` with an optional pre-release suffix. Do not include a `v` prefix.

### Auto-updater reports "no update available"
Verify that the `buzz-desktop-latest` release exists and contains a
valid `latest.json`. The manifest covers all four platform keys
(`darwin-aarch64`, `darwin-x86_64`, `linux-x86_64`,
`windows-x86_64`); a missing entry usually means that platform's
release job failed — check the workflow run.
