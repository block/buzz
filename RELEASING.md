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
3. **Auto-tag dispatches `docker.yml`** — the same workflow then triggers
   `docker.yml` with the version and tag ref, which builds the multi-arch relay
   image and publishes `ghcr.io/block/buzz:<version>` (plus `:<major>.<minor>`,
   `:<major>`, and `:latest` for stable releases). Prereleases
   (`relay-v<version>-rc.1`) publish only the prerelease tag and do **not**
   move `:latest`. (The dispatch — rather than relying on `docker.yml`'s
   `push: tags` trigger — is required because GitHub suppresses `on: push` for
   tags pushed by the workflow's own `GITHUB_TOKEN`; the desktop lane dispatches
   `release.yml` for the same reason.)

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

## Manual Fallback

If the automated flow isn't suitable (e.g., building from a non-main ref):

1. Go to **Actions > Release** in the GitHub UI
2. Click **Run workflow**
3. Provide the semver version (no `v` prefix) and the ref to build from

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
   auto-updater containing `latest.json`, the signed `.tar.gz` archive,
   and its `.sig` signature.

---

## Platform Support

The release workflow builds **two separate macOS DMGs** — Apple
Silicon (`darwin-aarch64`, the `release` job) and Intel
(`darwin-x86_64`, the `release-macos-x64` job) — plus Linux `.deb` and
`.AppImage`. Both macOS DMGs are codesigned, notarized, and attached to
the same `v<version>` release. Intel users download the `_x64.dmg`.

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
valid `latest.json`. The auto-updater manifest currently lists
`darwin-aarch64` only, so Intel and Linux users do not yet receive
auto-updates — they download new versions manually from the release
page. (Adding their entries to `latest.json` is a follow-up.)
