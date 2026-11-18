# Releasing Buzz

This repository has two independent release lanes. Relay uses release PRs.
Mobile uses immutable release-candidate tags cut from remote `main` by default,
with an optional stable source branch:

| Lane | Entry point | Artifact |
|------|-------------|----------|
| Relay | `just release-relay` | `ghcr.io/block/buzz` container image |
| Mobile | `scripts/mobile-release.sh candidate X.Y.Z` | Exact `mobile-vX.Y.Z-rc.N` source identity |

The desktop app is released from
[block/buzz-app](https://github.com/block/buzz-app), not from this repository.

The lanes version independently. Relay reads its crate manifest, and mobile
derives both source and marketing version from the
exact candidate tag. Mobile candidate tags are pushed by a human release
manager, not by CI or a GitHub App. The handoff to the private `buzz-releases`
pipeline remains manual because OSS CI cannot trigger private CI.

## Quick Start

```sh
# Relay release
just release-relay
just release-relay 0.4.0

# Publish the next mobile candidate from the exact current remote main commit
# (requires bypass of the mobile tag ruleset, see Mobile below)
scripts/mobile-release.sh candidate 0.5.0
```

Relay uses a metadata PR. Mobile does not. Each `mobile-vX.Y.Z-rc.N` tag is an immutable
candidate and the artifact of record.
Optional stable branches use `mobile-vMAJOR.MINOR`. There is no
stable mobile tag alias, finalization step,
or mobile GitHub Release.

---

## How It Works

### Relay

1. **`just release-relay`** runs locally on `main`, creates or updates a
   `relay-release/<version>` PR, bumps `crates/buzz-relay/Cargo.toml`,
   regenerates `Cargo.lock`, and updates the relay changelog.
2. **Merge the PR.** `auto-tag-on-release-pr-merge` pushes
   `relay-v<version>`.
3. **The tag triggers `docker.yml`.** Stable releases update the version
   aliases and `latest`; prereleases do not. Each release also publishes an
   optimized, symbol-bearing image under matching `debug-` tags (for example,
   `debug-0.3.0` and `debug-latest`) for native profiling. The ordinary tags
   remain stripped and are the default for deployments that do not need it.

Every push to `main` that changes the image build context continues to
publish the rolling relay `:main` and `:sha-<7>` tags, plus matching
`:debug-main` and `:debug-sha-<7>` variants. Pushes that only touch paths
`.dockerignore` excludes (such as `mobile/` and `docs/`) publish
nothing.

### Mobile

1. **Publish a candidate.** From a clean checkout whose `origin` is the
   canonical `block/buzz` repository, run
   `scripts/mobile-release.sh candidate X.Y.Z`. The script resolves and fetches
   a snapshot of `origin/main`, derives the next number from exact
   remote tags for that marketing version, and publishes an annotated
   `mobile-vX.Y.Z-rc.N` tag at that snapshot with `git tag -a` (your own tag
   signing configuration applies) and pushes exactly that one tag to `origin`.
   The push is allowed only because you are a bypass actor of the dedicated
   mobile tag ruleset [`26588677`](https://github.com/block/buzz/rules/26588677)
   for `refs/tags/mobile-*`; bypass is granted to the mobile release managers.
   The `buzz-release-bot` App and its key are not involved in mobile. The script never uses the operator's
   checked-out commit and never moves an existing candidate. To select a
   historical commit, add `--commit FULL_SHA`. The script requires that commit
   to be in the selected source branch's history. Later branch advances do not
   change or invalidate the tag. If the push is rejected and origin has no
   such tag, the script deletes the local tag and fails; check that you are a
   bypass actor of the ruleset. If the candidate may already be on GitHub, the
   script keeps the local tag and says so: use the published tag rather than
   rerunning.
2. **Build the exact tag.** Enter the candidate tag as `mobile_ref` in the
   private Buzz mobile Buildkite pipeline. OSS CI deliberately cannot trigger
   that private pipeline. The tag supplies both source commit and release
   version. Flutter receives clean marketing version `X.Y.Z`; Buildkite's
   monotonically increasing build number supplies the platform build number.
3. **Promote tested artifacts.** Promote the already-built signed artifact for
   each platform through its store workflow. Record the exact tag with the
   build or rollout record. No source ref is changed and no final build is cut.

The iOS and Android artifacts for one marketing version may come from different
RC tags. For example, iOS can ship `mobile-v0.5.0-rc.2` while Android ships
`mobile-v0.5.0-rc.3`. Each platform's exact candidate tag is its source record.
There is intentionally no single selected or final candidate for the marketing
version.

### Optional stable release branches

For a release that needs isolation from ongoing development, create a branch
such as `mobile-v0.25` from an explicitly selected, verified source commit.
Publish candidates from its current remote tip:

```bash
scripts/mobile-release.sh candidate 0.25.0 --source-branch mobile-v0.25
```

The branch must match the marketing version's major and minor components:
`mobile-v0.25` accepts 0.25.x, but rejects 0.26.x and 1.25.x. Without the option,
the source remains `main`. The script validates the source;
a missing branch or a commit outside its history fails rather than falling back
to `main`. `--commit FULL_SHA` also works with a stable source branch. Its
review and check protections bind everyone except the mobile release managers,
who bypass them: a release manager can push an unreviewed commit to a stable
branch and tag it as a candidate.
The private pipeline still builds only the resulting immutable candidate tag.

For the October 2026 stabilization period, all distributed mobile builds,
including Company Portal dogfood builds, must come from `mobile-v0.25` until
after the conference. Do not upload builds from `main` or a later minor version
that would displace the stable dogfood build. The branch starts from
`mobile-v0.24.0-rc.2`, the source of uploaded iOS 0.24.0 (7).

General fix PRs must target `main` and land there first. At each fix landing,
ask the release owner whether to
backport the merged commit to the stable branch. For each explicitly approved
fix, cherry-pick with `-x --signoff` onto a backport branch and open a backport-only
PR targeting the stable branch. Require review and passing checks before merging
that PR. Preserve the original commit provenance and validate the resulting
stable source. A correction for a regression introduced by an earlier backport
may target the stable branch directly; identify the faulty backport and keep the
correction scoped to that regression. The same review and checks apply.
Never merge `main` wholesale.
Remote agents and App Clip are post-conference work. There is no preselected
fix list and no requirement to create a stable branch for every minor release.

The initial 0.25.0 candidate pins that baseline with `--commit`. The second
candidate uses an empty commit above that baseline to verify
that the existing pipeline builds a commit outside `main`. Verify identical
source trees, absence of that commit from remote `main`, and the exact tag,
commit, marketing version and build number in the uploaded artifacts. Upload
and external TestFlight review do not establish production App Store approval.

`mobile/pubspec.yaml` keeps `0.0.0+1` only as a valid, visibly non-release
fallback for local development and validation builds. Release jobs always
inject both version fields. `mobile/CHANGELOG.md` is retained as historical
release data. It is not a release ledger for this flow.

---

## Version Sources

| Lane | Release version authority |
|------|---------------------------|
| Relay | `crates/buzz-relay/Cargo.toml` |
| Mobile | Exact `mobile-vX.Y.Z-rc.N` remote tag |

`just bump-relay-version <version>` updates the relay crate and regenerates
`Cargo.lock`. Mobile has no bump recipe or
release-metadata PR.

---

## Release Retry

Mobile intentionally has no branch or arbitrary-ref fallback. The private
Buildkite pipeline accepts only an exact candidate tag.

---

## Internal Releases

For mobile, trigger the private
[Release Mobile pipeline](https://buildkite.com/runway/buzz-mobile-releases) with
an exact RC tag for the platform build being cut. See the
[buzz-releases README](https://github.com/squareup/buzz-releases#cutting-a-release)
for the rest of the private pipeline contract.

---

## What Gets Published

Mobile publishes only annotated `mobile-vX.Y.Z-rc.N` git tags. Store artifacts
and rollout records retain the exact tag they used. Mobile does not publish a
GitHub Release or a stable `mobile-vX.Y.Z` alias.

---

## Prerequisites

- **Write access** to the `block/buzz` GitHub repository
- An `origin` remote whose configured URL is the canonical `block/buzz`
  repository
- `gh` CLI authenticated with permission to push the release branch and open
  its pull request
- Release tag ruleset [`14378754`](https://github.com/block/buzz/rules/14378754)
  active for all tags (`~ALL`), with creation, update, deletion, and
  non-fast-forward protections and `buzz-release-bot` as its sole always-bypass
  actor. It must exclude `refs/tags/mobile-*`: rulesets stack, so otherwise it
  rejects release managers' mobile tag pushes. `scripts/mobile-release.sh`
  checks that exclusion before tagging.
- The `buzz-release-bot` App credentials configured for GitHub Actions (relay
  and chart lanes only; mobile does not use them)
- The following **GitHub Actions variables and secrets** configured for the
  PR-driven release lanes:

  | Name | Kind | Purpose |
  |------|------|---------|
  | `BUZZ_RELEASE_TAGGER_CLIENT_ID` | Variable | GitHub App client ID used to create protected release tags |
  | `BUZZ_RELEASE_TAGGER_PRIVATE_KEY` | Secret | GitHub App private key |

Mobile candidate publication requires being a bypass actor of the mobile tag
ruleset [`26588677`](https://github.com/block/buzz/rules/26588677) ("Restrict
Mobile Tags"), recorded as `MOBILE_RELEASE_TAG_RULESET_ID` in
`scripts/release-rulesets.sh`. Bypass is granted to the mobile release
managers. The ruleset must be active for `refs/tags/mobile-*` (or the narrower
`refs/tags/mobile-v*`), with creation, update, deletion, and non-fast-forward
protections and no exclusions. The `buzz-release-bot` App must not be a bypass
actor. On every publication the script checks the public policy and that the
caller's effective `current_user_can_bypass` is `always`; GitHub hides the bypass
actor list from non-admin tokens, so an administrator audits that list.
Stable sources additionally require active branch ruleset
[`19321162`](https://github.com/block/buzz/rules/19321162) covering
`refs/heads/mobile-v*`, with no exclusions, deletion and force-push prevention,
at least one approval, stale-approval dismissal, last-push approval, resolved
review threads, and strict required `Mobile` and `DCO Check` checks. The mobile
release managers are also its bypass actors, so they can publish from a stable
branch; the ruleset still forces review and checks on everyone else. Because of
that bypass, a release manager can push an unreviewed commit to a stable branch
and publish it as a candidate; nothing else enforces review for them. The script
verifies the public policy on every publication but does not check bypass.

---

## Troubleshooting

### New commits land after publishing a mobile candidate

Run `scripts/mobile-release.sh candidate <version>` again after the intended
fix reaches the intended remote source branch. Use the same `--source-branch`
option for a stable release. The command publishes a new immutable RC tag at the new exact
remote commit. Continue referring to each tested or shipped platform artifact by
its own exact tag.

### The source branch advances during candidate publication

Publication remains pinned to the selected SHA. The script verifies that it
is an ancestor of a resolved source-branch snapshot, then creates, pushes, and
verifies an immutable annotated tag for that exact SHA. It verifies the tag,
not the branch's later tip. A newer commit does not require retrying or creating
another candidate. Missing or unverifiable ancestry still fails closed.

### A mobile candidate command selects the wrong RC number

Do not retry by moving or deleting a tag. Inspect the exact remote `mobile-v*`
tags and resolve the unexpected state. Candidate numbers are monotonically
increasing remote identities.

### A mobile candidate publication is rejected by repository rules

Confirm you are a bypass actor of the active mobile tag ruleset `26588677`
(the mobile release managers). The script removes its local tag after a rejected
push, so retry after an administrator fixes your access. Do not add yourself to
the bypass list, grant the App bypass, or weaken update or deletion protection. Existing
candidate tags must remain immutable.
