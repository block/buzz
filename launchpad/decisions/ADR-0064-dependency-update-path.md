---
status: Accepted
date: 2026-08-13
issue: launchpad-26/buzz#64
decided_in: launchpad-26/buzz#64
supersedes: none
---

# ADR-0064 — Dependency update path for this fork

## Decision

The fork's dependency-update path is **native GitHub Dependabot version updates via
`.github/dependabot.yml`** (option 2 of #64 as filed), covering `cargo` (root), `npm`
(`desktop/`, `web/`), `pub` (`mobile/`), `docker` (root), and `github-actions` (root) —
six ecosystem entries, one per manifest location. Renovate (option 1) is **not chosen**,
and is currently **unreachable**, not merely undesirable — see Context. Neither option 3
(neutralize and rely on upstream) nor option 4 (both bots) is chosen, for the reasons
already given in #64 as filed.

`renovate.json` stays in the tree, unedited, and is declared **upstream-owned and inert**
by this ADR — the second resolution #62's own success criteria names as acceptable. It is
not deleted: as an upstream-tracked file, deletion would likely be silently reintroduced
on the next sync from `block/buzz` rather than stick. Its `"automerge": true` therefore
governs nothing on this fork; that must not be re-read as an active supply-chain risk here,
only on `block/buzz` itself.

This ADR decides the update path only. Building `.github/dependabot.yml` for real, and
demonstrating at least one live update PR per #62's success criteria, is #71 — not done
here, matching the split #63 already established between deciding and implementing.

## Context

Before this ADR, the fork had no dependency-update or alerting path of any kind, while
carrying `renovate.json` — inherited from upstream, `"automerge": true` — that reads as
though it does. Re-verified live rather than assumed from #64 as filed:

```
$ gh api repos/launchpad-26/buzz/dependabot/alerts
{"message":"Dependabot alerts are disabled for this repository.", ...} (HTTP 403)
```

No `.github/dependabot.yml` exists. `cargo-deny check` runs at `.github/workflows/ci.yml:900`
(filed against line 887 — the file has grown ~13 lines since #64 was written; the check
itself is unchanged, covering Rust advisories only).

#64 as filed left one thing explicitly unverified: "whether the Renovate app can be
installed on the `launchpad-26` org, and by whom." That question was resolved, not assumed,
before this decision:

```
$ gh api orgs/launchpad-26/memberships/benmitchell11
{"role": "member", ...}

$ gh api orgs/launchpad-26/members?role=admin
[{"login": "baradev", ...}, {"login": "jatin-puri-coder", ...}, {"login": "joshuavial", ...}]
```

Installing a GitHub App at the org level needs org **owner**, not repository admin — a
stricter bar than anything else this PRD has needed so far (compare ADR-0063's push
protection, which only needs repo admin). This account holds org role `member`, the same
constraint ADR-0065 already hit for GitHub App creation, applying here to GitHub App
*installation* instead.

Critically, this was checked against the actual person available to this cohort: the
project's PM holds **repository admin on `buzz` only, not organization owner**. The three
accounts holding org owner are unrelated to this cohort's chain of contacts, with one
exception — one of them is this cohort's course instructor, a materially different, heavier
ask than a routine dependency-tooling decision. Option 1 is therefore not "harder" than
option 2, it is **closed to this cohort as currently staffed**, the same way ADR-0065 found
a GitHub App unreachable for a different reason (org role, not admin-read scope). This ADR
does not rule out revisiting Renovate if that staffing changes; it rules it out as a choice
available today.

That leaves option 2, which #64 as filed already noted needs no admin for version updates
(only Dependabot's *alerts* feature is admin-gated, and this ADR does not enable that — it
remains #62's admin-settings track, tracked at #72). A draft six-entry `dependabot.yml`
(cargo, npm ×2, pub, docker, github-actions) was written and validated as structurally
correct (`yaml.safe_load` parses it, all six manifest directories confirmed to exist:
`Cargo.toml`, `desktop/package.json`, `web/package.json`, `mobile/pubspec.yaml`, three
root `Dockerfile*` files, `.github/workflows/`). It was not committed here — that draft,
and confirming whether Dependabot's `docker` ecosystem watches multiple differently-named
Dockerfiles in one directory or needs one entry per file, is #71's work.

Options 3 and 4 were not re-litigated: #64 as filed already gives a sufficient, honest
account of both (3 names the current inert state as exactly the failure #62 exists to
catch; 4 creates duplicate PR traffic against manifests the cohort already doesn't fully
own). Nothing tested here changes either conclusion.

## Consequences

**Good.** The fork gains a real, self-serve dependency-update path today, with no
dependency on anyone's availability or role beyond this cohort's own write access —
verified reachable, not assumed. `renovate.json` stops being silently misread as active
protection: this ADR is the written resolution #62's success criteria requires, on record
rather than left for the next reader to guess. The path covers every ecosystem #64
identified (Cargo, pnpm ×2, Flutter/pub, Docker, Actions), matching the coverage Renovate
would have offered, without the third-party App write-access grant Renovate would have
required.

**Bad, stated honestly.** Dependabot version updates do not deliver vulnerability
*alerts* — that gap stays open regardless of this decision and belongs to #72, not here.
PR volume against manifests this fork mostly doesn't own, on CI that runs 24–41 minutes, is
a real and unmeasured cost; #64 as filed named this risk and nothing here reduces it — if
anything, native Dependabot's weekly-per-ecosystem default (six entries) could generate
more individual PRs than a single tuned Renovate config would have, since the draft config
does not reuse Renovate's existing grouping rules (e.g. `renovate.json`'s Rust minor-version
isolation, redis/deadpool-redis pairing). Whoever builds #71 should decide grouping
explicitly rather than let six independent weekly runs become six independent PR floods.
Rejecting Renovate on access grounds also means the fork gives up the specific tuning
already written into `renovate.json` (cargo lockfile updates, digest-pinned Actions,
per-package pins for `evalexpr` and `tiptap`) — none of that carries over to Dependabot
automatically; #71 inherits the job of deciding which of it still matters.

## Provenance

Like ADR-0063, made directly while working #64 — `issue` and `decided_in` point to the
same place.

Verified live against this repository and this GitHub org, not assumed from #64 as filed:
- `renovate.json` content, including `"automerge": true`, re-read directly from the
  working tree.
- Dependabot alerts status via `gh api repos/launchpad-26/buzz/dependabot/alerts` —
  explicitly disabled, not an ambiguous 404.
- `cargo-deny check`'s current line number in `ci.yml`, confirming the one working
  advisory check is unchanged since #64 was filed.
- This account's org role (`member`) and the org's three actual owners, via
  `gh api orgs/launchpad-26/memberships/benmitchell11` and
  `gh api orgs/launchpad-26/members?role=admin` — resolving #64's own explicitly-named
  open question about who can install Renovate, rather than leaving it open.
- That the six proposed `dependabot.yml` ecosystem entries correspond to real manifest
  paths in this repository, and that the draft file parses as valid YAML.

Not verified: actual PR volume or noise from either path — #64 as filed already named this
as unmeasured, and nothing here measures it; that evidence can only come from #71 actually
running. Not verified: whether the PM's repository-admin role could be escalated to org
owner by request — only the PM's *current* role was established, not whether it could
change. Not verified: Dependabot's exact `docker` ecosystem behavior against multiple
differently-named Dockerfiles in one directory — left for #71.

This decision directly shapes #71 (builds the real `.github/dependabot.yml`, decides
grouping/scheduling the draft here left unresolved, and must produce the live update PR
#62's success criteria demands) and #72 (owns the Dependabot *alerts* admin-gated half this
ADR explicitly does not close).
