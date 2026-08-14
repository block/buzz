---
status: Proposed
date: 2026-08-15
issue: launchpad-26/buzz#65
decided_in: launchpad-26/buzz#65
supersedes: none
---

# ADR-0008 — What privilege the repository security audit runs with

## Decision

The repository security audit under #62 runs with **the bare `GITHUB_TOKEN` only —
Option 1 as filed**. Where a setting cannot be read at that privilege (today: whether
secret-scanning alerts are on, and whether branch protection is configured — both 404 at
this account's privilege level, which is ambiguous between "off" and "cannot ask"), the
audit reports `indeterminate` rather than guessing, and `indeterminate` must never render
as `pass`.

Option 2 (a long-lived fine-grained PAT with `administration: read`, stored as a repo
secret) and Option 4 (no attestation, settings recorded in a markdown file instead) are
both rejected, on the grounds already stated in the issue as filed: a PAT held by a person
outlives that person and is a standing liability on a public repo, and a written-down
setting drifts from the enabled one silently — `renovate.json` is this repo's own live
example of exactly that failure. Option 3 (a GitHub App scoped to `administration: read`)
is not chosen now, but is recorded below as the specific, pre-agreed upgrade path — not an
open question to re-litigate later.

## Context

#62 requires the audit to report whether repository security settings — secret scanning,
push protection, Dependabot alerts, branch protection, default `GITHUB_TOKEN` permissions
— are actually on. The querying account holds no admin, and two endpoints this needs
(`secret-scanning/alerts`, `branches/launchpad/protection`) return a 404 that is
genuinely ambiguous: it means either the feature is off, or the caller isn't permitted to
ask, and nothing in the response distinguishes the two.

This is deliberately narrower than #25, which owns the deployment identity and where
production secrets live. This decision covers only the read-only credential the audit
workflow itself uses to inspect this repository's own settings; neither decision
constrains the other.

## Consequences

**Good.** The fork's credential surface for this audit stays at exactly zero — no new
credential exists, so none can leak, be phished out of a compromised action, or be reached
by a malicious dependency or a prompt-injected agent-authored workflow change (a real
threat model here: roughly half this repo's commits are agent-authored). Whichever way
this had gone, the audit stops presenting an ambiguous 404 as a fact; choosing this option
keeps that fix free of any new attack surface.

**Bad, stated honestly.** Several of #62's criteria will permanently read `indeterminate`
on this option alone, and the human tendency is to read `indeterminate` as "probably
fine" — it is not evidence either way, and must not be treated as one.

**Contingency — the upgrade path, pre-specified so it doesn't need to be re-derived under
pressure:**

*Trigger.* Any of: a real finding that one of the two ambiguous settings was actually
misconfigured and the audit couldn't see it; a periodic manual check (below) turning up
drift; or the cohort gaining org-owner rights, making the upgrade cheap to do
opportunistically rather than reactively.

*The fix — Option 3.*
1. Someone holding `launchpad-26` org-owner rights creates a GitHub App scoped to
   `administration: read` only.
2. Installs it on `launchpad-26/buzz` specifically, not org-wide.
3. Stores its App ID and private key as an Actions secret or Environment — never a
   tracked file.
4. Points the existing audit workflow at it for exactly the two calls that currently read
   `indeterminate`.
5. Re-runs the audit and confirms both flip to a real pass/fail.
6. This is additive: nothing about the Option 1 baseline needs removing or reworking.

*The safety net until the trigger fires.* The repository owner already holds admin access
directly and can check both settings in the GitHub UI during any broader security pass,
independent of whether the automated upgrade ever happens — so "we don't know" from the
audit does not mean "nobody has ever checked."

## Security implications

Any token-bearing check that is added later (per the contingency above) must run on the
scheduled path only, never on a `pull_request` trigger — repository secrets are not
exposed to fork-PR-triggered runs regardless, but the constraint should be explicit in
the workflow rather than incidental. It must be read-only, scoped to exactly the
endpoints queried, and its scope, owner and expiry recorded where the next person can
find them. The audit's own output is public, so it must continue to report status and
remediation only, never exploitability, whichever privilege level it runs at.

## Provenance

Decided directly in conversation with the repository owner (@serina-mcfall) on
2026-08-15, following the recommendation and the contingency plan both posted as comments
on #65 — the same pattern ADR-0005 recorded, where a decision made outside the issue's own
comment thread is written down here rather than left implicit. `issue` and `decided_in`
both point to #65 because the decision and its filing issue are the same place; there was
no separate PR or review thread where this was argued first.

Not verified independently in this document: whether the cohort could obtain org-owner
rights to execute the Option 3 upgrade path if the contingency trigger fires — that
remains an open fact to check at the time, not assumed here either way.
