# Releasing Buzz Desktop

## Quick Start

```sh
# Regular release (next patch version)
just release

# Patch release
just release patch

# Minor release
just release 0.4.0

# Any explicit version
just release 1.0.0
```

This creates a `version-bump/<version>` PR that bumps all version manifests, regenerates lockfiles, and appends a changelog entry. Merge the PR to trigger the build automatically.

Re-running `just release` with the same version is safe — it detects the existing branch and PR, resets to current `main`, regenerates the changelog with any new commits, and updates the PR in place.

---

## How It Works

1. **`just release`** runs locally on `main` — computes the next version, creates (or reuses) a `version-bump/<version>` branch, bumps versions in all manifests, regenerates lockfiles, generates a changelog entry, commits, pushes, and opens (or updates) a PR.

2. **Merge the PR** — the `auto-tag-on-release-pr-merge` workflow detects the `version-bump/*` branch merge and pushes a `v<version>` tag.

3. **Tag triggers `release.yml`** — the existing release workflow builds, signs, notarizes, and publishes the desktop app for macOS and Linux.

---

## Release Types

| Command | Version | Example |
|---------|---------|---------|
| `just release` | Next patch | `0.3.0` → `0.3.1` |
| `just release patch` | Next patch | `0.3.0` → `0.3.1` |
| `just release 0.4.0` | Explicit minor | `0.3.1` → `0.4.0` |
| `just release 1.0.0` | Explicit | `1.0.0` |

---

## Version Files

`just bump-version <version>` updates these files:

| File | Field |
|------|-------|
| `desktop/package.json` | `"version"` |
| `desktop/src-tauri/tauri.conf.json` | `"version"` |
| `desktop/src-tauri/Cargo.toml` | `version` (under `[package]`) |
| `mobile/pubspec.yaml` | `version:` (preserves build number) |

It also regenerates `pnpm-lock.yaml`, `desktop/src-tauri/Cargo.lock`, and `mobile/pubspec.lock`.

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

### `just release` fails with "must be on main branch"
Switch to `main` and pull latest before running `just release`.

### `just release` fails with "working tree is dirty"
Commit or stash your changes before running `just release`.

### New commits merged after creating the release PR
Re-run `just release` from an up-to-date `main`. It resets the branch to current `main`, regenerates the changelog and PR body to include the new commits, and force-pushes the updated branch.

### Build fails at "Validate version"
The version string must be valid semver: `MAJOR.MINOR.PATCH` with an optional pre-release suffix. Do not include a `v` prefix.

### Auto-updater reports "no update available"
Verify that the `buzz-desktop-latest` release exists and contains a
valid `latest.json`. The auto-updater manifest currently lists
`darwin-aarch64` only, so Intel and Linux users do not yet receive
auto-updates — they download new versions manually from the release
page. (Adding their entries to `latest.json` is a follow-up.)
