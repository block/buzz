# Hula Buzz Mac desktop releases

Hula-owned Mac releases with in-app updates. This path never uses Block updater
endpoints or Block Tauri signing keys, and must never run `just release-desktop`
(that recipe opens PRs against `block/buzz`).

## User flow

1. **One-time install** — download the `.dmg` from a versioned GitHub Release
   (`hula-desktop-vX.Y.Z`), open it, drag **Hula Buzz** into Applications.
2. **Thereafter** — the app checks
   `https://github.com/jmfcamp/buzz/releases/download/hula-desktop-latest/latest.json`
   and applies updates in-app.

First launch may need **right-click → Open** until Apple notarization is wired
(updater signing is already in place; notarization is a follow-up).

## Secrets (jmfcamp/buzz)

| Name | Purpose |
|------|---------|
| `TAURI_SIGNING_PRIVATE_KEY` | Hula Tauri updater private key (minisign) |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for that key (empty string if unpassworded) |

Public key and updater endpoint live in
`desktop/src-tauri/tauri.conf.json` under `plugins.updater` (committed).

Rotate the keypair only with a new mint + `gh secret set` (never paste private
keys into chat, commits, or PR bodies).

## Cut a release

Hula desktop versions are **Block base + Hula sub-increment**, e.g.
`0.5.23-hula.2`. The release workflow runs `desktop/scripts/set-version-from-tag.mjs`
so `package.json`, `tauri.conf.json`, `Cargo.toml`, and the `buzz-desktop`
`Cargo.lock` entry all match the tag before build. About / Settings read that
same value via Tauri `getVersion()` (not a hardcoded string).


Preferred (workflow dispatch):

```sh
gh auth switch --user jmfcamp
gh workflow run hula-desktop-release.yml \
  --repo jmfcamp/buzz \
  --ref main \
  -f version=0.5.23-hula.2 \
  -f promote_updater=true
gh auth switch --user jchula   # or your usual account
```

Or from a clean checkout on the intended commit:

```sh
just hula-desktop-release 0.5.23-hula.2
```

That tags `hula-desktop-v0.5.23-hula.2` and pushes it, which also triggers the workflow.

## What the workflow publishes

- Versioned release `hula-desktop-vX.Y.Z` with:
  - `HulaBuzz_X.Y.Z_aarch64.dmg`
  - `HulaBuzz_X.Y.Z_aarch64.app.tar.gz` (+ `.sig`)
  - `latest.json` / `updater-manifest.json` (Mac aarch64 only)
- Rolling release `hula-desktop-latest` with `latest.json` (when promote is on)

## Scope / not done

- **Mac aarch64 only** for the first drop (no Windows/Linux; Intel Mac deferred).
- **No Apple code signing / notarization** yet — Gatekeeper friction on first open.
- Does not touch Block `release.yml`, `buzz-desktop-latest`, or `just release-desktop`.
