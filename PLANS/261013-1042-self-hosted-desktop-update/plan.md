# Self-hosted desktop build and auto-update

Status: build flow done (2026-10-13, tag desktop-v0.5.28-mf.5). Next: Wasabi upload + updater manifest.
Date: 2026-10-13

## Outcome

Buzz Desktop builds from fork `kevinle128/buzz` check for updates only from our Wasabi bucket.
A release tag triggers a build on our own runners. The build signs the updater archives with our own Tauri key and uploads installers, signatures and `latest.json` to Wasabi.

## Decisions (confirmed by user)

| Topic | Decision |
|---|---|
| CI | GitHub Actions on fork `kevinle128/buzz`, self-hosted runners only |
| Linux runner | New runner on server `103.82.28.49`, separate dir, registered to the fork (existing `vtc-ai` runner stays on org `oceanlabs-holding`) |
| macOS runner | User's Mac |
| Platforms | macOS (arm64 + x64) and Linux AppImage. Windows dropped |
| Storage | Wasabi `s3.ap-southeast-1.wasabisys.com`, bucket `game-mobile-registry`, prefix `buzz-desktop/`, public-read on that prefix only |
| Apple signing | None. Ad-hoc signature only, no notarization. First install needs "Open Anyway" or `xattr -cr` |
| Version | `X.Y.Z-mf.N` in code and tags (`desktop-vX.Y.Z-mf.N`); shown as `X.Y.Z.N`. `X.Y.Z` = upstream base; `N` = our release count on that base, reset to 0 on upstream sync |
| Upstream sync | Follow upstream release tags `desktop-v*`, not `main` (Archon integration later, out of scope) |
| Build notification to Buzz channel | Dropped |
| Build workspace | Cleaned after every build |
| Runner notes | Vault `obsidian-mind/reference/Buzz Desktop Build Runners.md` |

## Non-goals

- Archon sync workflow and Buzz PR review (later phase).
- Windows builds, Apple Developer ID, mobile.
- Channel notification.

## Facts verified in source

- Endpoint is injected by `desktop/scripts/build-release-config.mjs` from `BUZZ_UPDATER_ENDPOINT` / `BUZZ_UPDATER_PUBLIC_KEY`; `release.yml` hardcodes the upstream GitHub URL (`release.yml:90` and 9 other places).
- macOS signing uses `block/apple-codesign-action` with Block secrets (`release.yml:173`) — unusable for us.
- Version regex already accepts pre-release (`scripts/prepare-desktop-release.sh:6`, `release.yml:34`).
- `release.yml:919` marks any `*-*` version as prerelease and the rolling manifest is promoted only for stable versions. Every `-mf.N` version must be treated as a promotable release.
- `release.yml` requires a non-empty `## vX.Y.Z-mf.N` block in `CHANGELOG.md` (`release.yml:916`).
- Server has Node; `cargo`, `pnpm`, `aws` CLI are missing. 16 CPU, 31 GB RAM, 62 GB free disk.
- Wasabi key in relay `.env` can list the bucket but cannot read/set bucket policy; anonymous access returns 403.

## Phases

### Phase 1 — Infrastructure (user + me)
- [ ] Wasabi admin adds bucket policy: public `s3:GetObject` on `arn:aws:s3:::game-mobile-registry/buzz-desktop/*`. Verify with anonymous `curl` on a test object.
- [ ] Create a dedicated Wasabi key with write access limited to `buzz-desktop/*` (preferred over reusing the relay key). If not possible, reuse relay key — user decides.
- [x] Generate a new Tauri updater keypair (`pnpm tauri signer generate`). Store private key + password as GitHub secrets; public key as `BUZZ_UPDATER_PUBLIC_KEY`.
- [x] Register self-hosted runner on server in `/home/vtcm/actions-runner-buzz`, labels `self-hosted, Linux, X64, buzz`; install as systemd service. Install Rust toolchain (from `rust-toolchain.toml`), pnpm, Tauri Linux deps, `aws` CLI.
- [x] Register self-hosted runner on Mac mini (moved to `~/actions-runner-buzz`: TCC blocks launchd on external volumes) , labels `self-hosted, macOS, ARM64, buzz`; ensure Xcode CLT, Rust (+ `x86_64-apple-darwin` target), pnpm.
- [x] GitHub secrets on fork: `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, `BUZZ_UPDATER_PUBLIC_KEY`. Pending: `WASABI_ACCESS_KEY_ID`, `WASABI_SECRET_ACCESS_KEY`.

### Phase 2 — Workflow
- [x] New fork-only workflow `.github/workflows/mf-desktop-build.yml` (build + artifacts + cleanup). Upstream `release.yml` is gated to `block/buzz`, so it stays untouched.

Remaining items below apply to `mf-desktop-build.yml`, not `release.yml`:
- [ ] `runs-on`: Linux jobs → `[self-hosted, Linux, X64, buzz]`; macOS jobs → `[self-hosted, macOS, ARM64, buzz]`.
- [ ] Remove `release-windows` job and Windows entries in manifest assembly (lower `TRIPLES` minimum from 3 accordingly).
- [ ] Remove `block/apple-codesign-action` steps; keep ad-hoc signed `.app`/DMG, rebuild and `tauri signer sign` the updater tarball as today.
- [ ] Set `BUZZ_UPDATER_ENDPOINT=https://s3.ap-southeast-1.wasabisys.com/game-mobile-registry/buzz-desktop/latest.json` in one workflow-level `env` instead of 10 copies.
- [ ] Replace GitHub Release upload + `promote-oss-desktop-release` with Wasabi upload:
  - versioned artifacts to `buzz-desktop/vX.Y.Z-mf.N/`
  - `latest.json` URLs point at those versioned objects
  - upload `latest.json` last, with `Cache-Control: no-cache`, only after all artifacts succeed
- [ ] Treat `-mf.N` versions as normal releases (no prerelease skip of manifest promotion).
- [ ] Trigger stays `push: tags: desktop-v*`; restrict to the fork (`if: github.repository == 'kevinle128/buzz'`) so self-hosted runners never run fork PR code.
- [ ] Disable or delete upstream-only workflows that would target our runners or fail on missing Block secrets (`promote-oss-desktop-release.yml`, canaries, `auto-tag-on-release-pr-merge.yml` if it needs the Block GitHub App).

### Phase 3 — Version tooling
- [ ] Make `just release-desktop` default bump `X.Y.Z-mf.N` → `X.Y.Z-mf.(N+1)`, and `X.Y.Z` (fresh upstream sync) → `X.Y.Z-mf.0`.
- [ ] Tag creation: since the auto-tag workflow depends on Block's GitHub App, create the `desktop-v…` tag from the release script or a small fork workflow using `GITHUB_TOKEN`-independent PAT. Decide during implementation after reading `auto-tag-on-release-pr-merge.yml` in full.
- [ ] Display `X.Y.Z.N` in the About/Settings UI and update dialog (map `-mf.N` → `.N`).

### Phase 4 — Verification
- [ ] Dry run: tag `desktop-v0.5.26-mf.1`, confirm both runners pick up jobs, artifacts + `latest.json` appear on Wasabi, anonymous download works.
- [ ] Install `0.5.26-mf.1` on a clean Mac via DMG; confirm "Open Anyway" path works.
- [ ] Release `0.5.26-mf.2`; confirm the installed app detects, downloads and installs it. Record whether Keychain / mic prompts reappear after update.
- [ ] Same update check on Linux AppImage.
- [ ] Confirm the updater rejects an archive signed with a different key.

## Risks

- Ad-hoc signature changes every build; macOS may re-prompt Keychain access (nsec stored via `keyring` apple-native, `desktop/src-tauri/Cargo.toml:65`) after each update. User accepted this.
- Self-hosted runner on a shared server runs build code with access to the server; mitigated by tag-only trigger on the fork.
- Users on upstream builds keep upstream's pubkey and endpoint; they must reinstall our build once.
- Disk: Rust target dirs are large; add cleanup on the server runner.

## Unresolved questions

1. Who has Wasabi admin rights to add the public-read policy and create a scoped key?
2. Should the new runner run as user `vtcm` or a separate low-privilege user?
