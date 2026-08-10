---
status: Accepted
date: 2026-08-11
issue: launchpad-26/buzz#55
decided_in: launchpad-26/buzz#10
supersedes: none
---

# ADR-0002 — Which repositories the handbook draws on

## Decision

The handbook synthesises from **exactly five** source repositories, each with a stated role
and the origin prefix its claims carry:

| Repository | Role | Prefix |
|---|---|---|
| `block/buzz` | Upstream product and technical documentation | `[upstream]` |
| `launchpad-26/buzz` | The fork — local changes and their rationale | `[launchpad]` |
| `launchpad-26/launchpad` | Cohort working practices | `[cohort]` |
| `launchpad-26/skills` | Agent skills used alongside Buzz | `[supporting]` |
| `launchpad-26/rhizomorph` | Agent observability tooling | `[supporting]` |

Anything outside these five is out of scope until this record is superseded.

## Context

prd-02 (#4) asked which supporting DevOps/AgentOps repositories were in scope and left it
open. Left unanswered, "supporting repository" has no boundary, and the `[supporting]`
origin prefix means nothing in particular — it would end up labelling a dozen unrelated
things, including personal and challenge repositories that happen to share the org.

Scope defined as "whatever we can read" is not scope. Naming five makes the boundary
checkable: a reviewer can verify a citation's origin prefix against its repository by eye,
which is what makes the prefix-versus-reference check in #8 meaningful.

## Consequences

**Good.** Five rows is small enough to hold in your head and to check mechanically. Each
has a written role, so "which repository answers this question?" has an answer rather than
a judgement call.

**Bad.** Repositories the cohort later cares about are excluded until this record changes —
and the right place to change it is here, not quietly inside a page. Two of the five are
private, which binds this decision to ADR-0001: the site must stay org-restricted for those
citations to be legitimate.

## Provenance

Decided in #10 ("handbook E — first content set across the eleven categories"), which names
the five in a table. ADR #55 was raised afterwards so the decision has a home on the board.
This record ratifies #10 rather than re-opening it.
