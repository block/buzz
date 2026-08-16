---
status: Proposed
date: 2026-08-13
issue: launchpad-26/buzz#149
decided_in: launchpad-26/buzz#144
supersedes: none
---

# ADR-0005 — How Launchpad deployment diverges from upstream

## Decision

Launchpad deploys through a **wrapper under `launchpad/deploy/`** that delegates to
upstream's `deploy/compose/run.sh` unchanged. Upstream's Compose stack and deployment
runner are not forked, copied, or replaced.

Making that wrapper reach a Launchpad-built image requires upstream files to name
Launchpad rather than Block. **Five files are sanctioned to carry Launchpad values.** This
is a third deliberate exception to [`../AGENTS.md` §3](../AGENTS.md), alongside the two
already recorded there.

**This record precedes its implementation.** #144 makes these changes; at the time of writing none of them have landed on `launchpad`, so the table below states what each file is *sanctioned to carry*, not what it carries today. The check described under Enforcement reports that gap rather than hiding it, and will stay red until #144 merges. That is the intended behaviour: a sanctioned exception nobody is using is a claim this record should not be making silently.

| File | What it is sanctioned to carry | Why not an override |
|---|---|---|
| `deploy/compose/compose.yml` | `${BUZZ_IMAGE:?…}`, with the `ghcr.io/block/buzz:main` default **removed** | A Compose override file can add or replace a default; it cannot remove one. Structurally impossible elsewhere. |
| `.github/workflows/docker.yml` | Publication trigger (`launchpad`, not `main`) and target namespace | A parallel `launchpad-docker.yml` was considered and rejected — see Consequences |
| `deploy/compose/.env.example` | The checked-in example image reference | It is the file operators copy; a Launchpad-specific twin invites copying the wrong one |
| `Dockerfile` | OCI `source`, `url`, `documentation` labels | Labels are baked at build time by the file itself |
| `deploy/compose/README.md` | Points operators at the wrapper | Leaving upstream's instructions correct-looking but wrong is the failure this replaces |

Anything beyond those five lives under `launchpad/`. Upstream's own jobs that Launchpad
does not operate are **disabled in place** with `if: github.repository == 'block/buzz'`
rather than deleted, so upstream's copy stays intact and the diff stays reviewable.

## Enforcement

The list above is **machine-checked and intended to become a required status check**, not
a convention. `launchpad/scripts/adr_boundary_check.py` reads the table in this record,
requires this record's own prose count to agree with it, then requires `launchpad/AGENTS.md`
§3 and the filesystem to match. The workflow is `launchpad-adr-check.yml`, and the
enforcement task is #153.

Five properties of that check are decisions in their own right, because each one costs
somebody something:

- **This record is the source of truth.** The check does not hold its own copy of the
  list. A fourth copy would be the very defect being checked for. Adding a sixth file
  therefore requires editing this record, which is what "the list is closed" has to mean
  if it is to mean anything.
- **The check runs on every pull request, with no path filter.** A required check that is
  skipped stays pending forever and blocks the merge it was meant to guard, so filtering
  by path would turn "this pull request does not touch the boundary" into "this pull
  request can never merge".
- **It fails closed.** A missing checker, a missing record, or a partial checkout fails
  rather than passes. The cost is real and falls on everyone: a branch created before this
  record existed genuinely lacks these files and fails until it rebases onto `launchpad`.
- **It measures the boundary, not the presence of files.** An earlier version asserted only
  that the five files exist — and they exist upstream anyway, so it reported success on a
  tree where none of the exceptions were in use. Combined with §3 telling reviewers not to
  raise this in review, that removed the prompt to look and replaced it with a tick that
  measured nothing. The check now asserts that each sanctioned file no longer carries the
  upstream value it was sanctioned to replace, and a file listed here with no content
  assertion fails rather than passing quietly.
- **A deterministic check may gate; a model verdict may not.** #118 records agent judges
  scoring AUROC 0.48–0.64 against 6,642 human-verified labels on adversarial security
  claims. A required check that goes red on a model's opinion produces false blocks, and
  the remedy people find for a flaky required check is learning to route around it — which
  costs the gate entirely. Scripts gate. Model output annotates.

That last point reaches past this record. Whether the wider review process becomes required
CI, and what may turn such a check green, is a separate decision drafted at #154; this is
only its first instance, and it was chosen as the pilot because its subject is a fixed list
of five strings rather than a judgement.

## Context

`launchpad/AGENTS.md` §3 says everything cohort-specific lives under `launchpad/` and
upstream owns the rest. Read literally, that forbids this work — which is a problem,
because the alternative was worse.

Issue #141 found the fork's deployment path selecting `ghcr.io/block/buzz:main` in three
places. A clean checkout of this repository could start a stack running **someone else's
build**, with no signal that it had. Every relay change this cohort makes — including
membership gating and the hardening under #5 — was absent from what actually ran.

Fixing that means the fork must name itself somewhere. The question was only *where*, and
how little of upstream to disturb doing it.

The rejected alternative was a parallel `.github/workflows/launchpad-docker.yml` with
upstream's `docker.yml` gated off entirely. It respects §3 exactly and avoids conflicts on
that one file. It was rejected because it forks ~500 lines of carefully-commented build
matrix, provenance attestation and multi-arch manifest logic into a copy that will drift
silently from upstream's — trading a conflict that Git *shows you* for a divergence that
nothing does. The archived deployment attempt failed this way, and
`launchpad/deploy/AGENTS.md` records it: it "mixed fork-local automation with upstream
deployment files" and nobody could trace the result.

A conflict you must resolve is better than a copy you forget to.

## Consequences

**Good.** The wrapper is small enough to read in one sitting and owns only policy —
image namespace, immutability, Compose version. Upstream's orchestration keeps working
and keeps receiving upstream's fixes. Disabling rather than deleting upstream's
push-gateway jobs means a future sync shows a clean merge instead of a phantom deletion.

**Bad, and accepted.** Every upstream sync touching those five files conflicts.
`docker.yml` is the worst of them — the divergence is spread across roughly twenty comment
lines as well as the functional changes, so the conflict is larger than the behaviour
change warrants. Whoever runs the sync should expect it rather than discover it.

**Bad.** The wrapper is **advisory, not enforcement**. `deploy/compose/run.sh` still runs
standalone, and `compose.yml` enforces only that `BUZZ_IMAGE` is *set*, not that it is
immutable or Launchpad-owned. An operator following muscle memory bypasses every check.
The residual risk is accepted: #141's silent substitution on a clean checkout is closed,
which was the actual defect. What remains needs a deliberate act with a stale `.env`.

**Bad.** Five sanctioned files is a list, and lists rot. Adding a sixth is a change to
this record, not a judgement call in a pull request.

**Bad, and accepted.** Enforcing that mechanically taxes every contributor, not only the
ones touching deployment. The check runs on every pull request and fails closed, so a
branch that predates this record fails until it rebases — at the time of writing that is
#124, #144, #147 and #151. A cheaper check would run only on the files it guards, and
would be skipped exactly when someone deletes them.

## Provenance

Decided by @serina-mcfall on 2026-08-13 while reviewing #144, and recorded here
afterwards. The decision predates this record; #144 implements it.

**This ADR exists because its absence cost real work.** Two agent reviewers looked at #144
independently within four minutes of each other and both flagged the upstream-file
divergence — one sizing it "unavoidable given what #141 asked for", the other calling it a
merge blocker against §3. Neither could see the decision, because it was not written
anywhere, and §3 said the opposite of it. The disagreement was not about the code. It was
about a fact neither reviewer had access to.

That is the argument for the pointer added to §3 in the same change as this record: an ADR
nothing routes to is invisible to the next agent, and the next agent will raise the same
finding at the same cost.
