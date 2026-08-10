---
status: Accepted
date: 2026-08-11
issue: launchpad-26/buzz#54
decided_in: launchpad-26/buzz#6
supersedes: none
---

# ADR-0001 — Where the handbook source lives and how it is published

## Decision

The knowledge layer described in prd-02 (#4) lives in a **dedicated repository,
`launchpad-26/handbook`**, not inside `launchpad-26/buzz`. The repository is **private**,
and the site is published to **GitHub Pages with access restricted to organisation
members**, deployed by a GitHub Actions workflow on push to the default branch using the
workflow's own `GITHUB_TOKEN`.

These were two of prd-02's open questions — where the MkDocs source lives, and what hosts
the site — and they are recorded as one decision because neither can be taken alone.

## Context

prd-02 asked both questions and left them open. They turned out to be mutually determining:

- Org-restricted Pages is available **because** the repository is private.
- The private source repositories `launchpad-26/launchpad` and `launchpad-26/skills` are
  only citable **because** the site is org-restricted. #10 states this as a standing
  condition: *"This only holds while the site stays org-restricted."*

Deciding either question alone silently decides the other, which is why #6 settled both
together.

A public site citing private repositories fails two ways at once, per #6: a reader outside
the org following a `[cohort]` citation gets a 404, and an agent synthesising a page from a
private repository publishes cohort material to the internet.

## Consequences

**Good.** Private sources stay in scope, so the handbook can answer the questions prd-02
was created for — what operational practices apply to our deployment, and what an agent
should read before working on a part of the system. No deployment secret is needed; the
workflow's own token suffices.

**Bad.** The corpus lives outside `launchpad-26/buzz`, so the conventions in
[`../AGENTS.md`](../AGENTS.md) — issue types, labels, PR rules, DCO — do not automatically
govern it. That gap is real and is not closed by this decision.

The org-restriction condition is load-bearing rather than incidental: if the site is ever
made public, the citations to private repositories become both broken and a disclosure
problem. That is a constraint to re-check before any change to visibility, not a one-time
setup step.

## Provenance

Decided in #6 ("handbook A — repo, MkDocs scaffold and Pages publishing") before ADR #54
was raised. #54 was filed retrospectively to give the decision a home on the board, under
the convention that a PRD's open questions are raised as ADR issues parented to it. This
record ratifies what #6 already committed to; it does not re-open it.

#6 also pre-commits to the right failure behaviour: if a private `launchpad-26` repository
cannot serve a Pages site restricted to org members, the instruction is to stop and reopen
the scope question rather than proceed by making the repository public.
