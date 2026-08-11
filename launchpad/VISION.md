# Vision — the launchpad-26 cohort's Buzz environment

What this cohort is building on top of [`block/buzz`](https://github.com/block/buzz),
for whom, and what "done" looks like beyond the current milestone.

This document records direction. It does not implement any of it, and it does not settle
anything an open decision owns — where a choice is open it says so and links the issue.

---

## How to read this

Every claim in this document carries exactly one status marker. The marker is the claim's
honesty, not decoration: a proposal presented as a decision is worse than no document at
all.

| Marker | Means | Evidence it carries |
|---|---|---|
| `IMPLEMENTED` | True today | a link to the file or commit |
| `DECIDED` | Agreed, not built | a link to the accepted ADR |
| `PROPOSED` | Not yet agreed | — |
| `OPEN` | Undecided | a link to the ADR issue |

Where a claim's evidence is a milestone or an issue rather than an ADR document, the
Source or Status cell names it instead — the requirement is a link a reader can follow,
not a particular kind of link.

The other documents in this set link to this section rather than restating the legend.

---

## Mission

`PROPOSED`

The launchpad-26 cohort operates Buzz; it does not develop Buzz. Our mission is to deploy
and run a shared Buzz platform that the cohort can safely depend on and recreate, and to
use running it as the way we learn to operate real systems. Beyond that, the intent is for
Buzz to become an operational intelligence and knowledge platform for the cohort, rather
than only the software the cohort happens to be running.

Synthesised from [`AGENTS.md` §1](AGENTS.md),
[milestone M0](https://github.com/launchpad-26/buzz/milestone/1) and
[milestone M1](https://github.com/launchpad-26/buzz/milestone/2). Marked
`PROPOSED` rather than `DECIDED` because nobody has written or confirmed these particular
sentences: [#42](https://github.com/launchpad-26/buzz/issues/42) requires the vision to be
written or confirmed by a human, and approving this document's pull request is what
promotes it.

---

## Why Buzz

`PROPOSED` — a human confirmed these reasons on 2026-08-11, and no ADR issue was raised
for the question, so there is no accepted decision record to link. Approving this
document's pull request is what promotes them, exactly as for the Mission above.

| Reason | Substance |
|---|---|
| Agents cannot live in Discord | Buzz treats agents as first-class members that can be initiated on the relay and executed elsewhere. Discord bots can converse; they cannot be given work this way. |
| Learning by operating it | The cohort learns DevOps, security and AgentOps by running a real internet-facing system. Operating it is the curriculum, not a side effect. |
| We own the infrastructure and data | Self-hosting means the cohort controls the relay, identities, data and membership, with no dependence on a third party's terms, availability or export limits. |
| One surface for humans, agents and knowledge | Answer "what is changing?" and "how does this work?" without first knowing which repository holds the answer. |

The section-level marker above covers only the rows present when this section was
written. A row added later does not inherit it — give any new reason its own status
marker and its own link to evidence.

The wording of "One surface for humans, agents and knowledge" comes from
[milestone M1](https://github.com/launchpad-26/buzz/milestone/2), whose completion
statement is the cohort being able to answer those two questions "without first needing to
know which repository, changelog, issue, PR or documentation file contains the answer."

---

## Who this is for

| Audience | What they need from this environment | Status |
|---|---|---|
| Cohort members | A shared Buzz platform they can depend on and recreate, taking over from Discord as the cohort's coordination surface. | `PROPOSED` — [#42](https://github.com/launchpad-26/buzz/issues/42) |
| Agents contributing to this repository | A written statement of what is being built and why, so work is not scoped against an unwritten target, alongside the rules for how work is filed. | `IMPLEMENTED` — this document and [`AGENTS.md`](AGENTS.md) |
| Developers of `launchpad-26/rhizomorph` | Agent work initiated on the relay and executed elsewhere, directed at assisting that project's development. | `PROPOSED` — [#42](https://github.com/launchpad-26/buzz/issues/42); the execution boundary is `OPEN` — [#43](https://github.com/launchpad-26/buzz/issues/43) |
| Contributors joining later | Enough context to propose an agent workflow, and to state the access it needs and its blast radius, without first asking someone what the system around it is. | `PROPOSED` — [#42](https://github.com/launchpad-26/buzz/issues/42) |

---

## What we are building

The direction below is recorded in
[#42](https://github.com/launchpad-26/buzz/issues/42), and a human confirmed it on
2026-08-11. Nothing here is deployed, and #42 records the *requirement* for that
confirmation rather than the confirmation itself, so each row stays `PROPOSED` until
approving this document's pull request promotes it — the same promotion event the Mission
section names.

| Direction | Status |
|---|---|
| Migrate the cohort off Discord onto Buzz | `PROPOSED` — [#42](https://github.com/launchpad-26/buzz/issues/42) |
| Initiate agents on the relay that execute on cohort members' own machines | `PROPOSED` — [#42](https://github.com/launchpad-26/buzz/issues/42); how execution is contained is `OPEN` — [#43](https://github.com/launchpad-26/buzz/issues/43) |
| Direct that agent work at assisting development of `launchpad-26/rhizomorph` | `PROPOSED` — [#42](https://github.com/launchpad-26/buzz/issues/42) |

This changes what "secure" and "correctly sized" mean for the relay: it stops being only a
service to protect and becomes an initiation point for code execution on other people's
machines. That consequence is owned by
[#43](https://github.com/launchpad-26/buzz/issues/43) and is not settled here.

---

## Constraints we build within

Every row below is sourced, and every status was checked against the live issue state on
2026-08-11.

| Constraint | Status | Source |
|---|---|---|
| The repository is public; every tracked file is world-readable | `IMPLEMENTED` | [`launchpad/AGENTS.md` §8](AGENTS.md) |
| No secrets, keys, tokens, private hostnames or member rosters in tracked files | `IMPLEMENTED` as a binding rule, not as a checked property of the repository — [SECURITY-POSTURE.md § The public-repository rule](SECURITY-POSTURE.md#the-public-repository-rule) | [`launchpad/AGENTS.md` §8](AGENTS.md) and [#42](https://github.com/launchpad-26/buzz/issues/42) |
| The cohort operates Buzz and does not develop it; product bugs go upstream | `IMPLEMENTED` | [`launchpad/AGENTS.md` §1](AGENTS.md) |
| Upstream files are never moved or renamed; cohort files live under `launchpad/` | `IMPLEMENTED` | [`launchpad/AGENTS.md` §3](AGENTS.md) |
| Stable knowledge is a document; active work is an issue | `IMPLEMENTED` | [`launchpad/AGENTS.md` §2](AGENTS.md) |
| Agents draft everything and approve nothing | `IMPLEMENTED` | [`launchpad/AGENTS.md` §5 rule 1](AGENTS.md) |
| One VPS of cohort scale; whether the stack fits the proposed sizing is unmeasured | `OPEN` | [#18](https://github.com/launchpad-26/buzz/issues/18), [#21](https://github.com/launchpad-26/buzz/issues/21) |
| Routine deployment without distributing root credentials to cohort members | `OPEN` | [#5](https://github.com/launchpad-26/buzz/issues/5), [#25](https://github.com/launchpad-26/buzz/issues/25) |

The host sizing is deliberately not stated as a settled number.
[#18](https://github.com/launchpad-26/buzz/issues/18) is a task to *measure* whether the
stack fits the proposed sizing, and [#21](https://github.com/launchpad-26/buzz/issues/21)
has not decided the specification. Writing an unmeasured hypothesis here as though it were
a constraint is exactly the failure this document is meant to avoid.

The full rules for filing, reviewing and merging work are in
[`AGENTS.md`](AGENTS.md); this section lists only the constraints that shape *what* gets
built, not *how* work is filed.

---

## What success looks like

Whole-programme measures, not the acceptance criteria of any single PRD. Those live on
their own issues.

| Measure | Status | Source |
|---|---|---|
| Multiple cohort members can connect to a usable internet-facing Buzz server and communicate through it | `DECIDED` | [milestone M0](https://github.com/launchpad-26/buzz/milestone/1) |
| The server is hardened around least privilege, minimal exposure and explicit trust | `DECIDED` | [milestone M0](https://github.com/launchpad-26/buzz/milestone/1) |
| Running it does not require routine sharing of root credentials | `DECIDED` | [milestone M0](https://github.com/launchpad-26/buzz/milestone/1) |
| The server can be deployed and maintained through automation | `DECIDED` | [milestone M0](https://github.com/launchpad-26/buzz/milestone/1) |
| The server can be rebuilt from a bare supported Ubuntu host if the existing one is lost | `DECIDED` | [milestone M0](https://github.com/launchpad-26/buzz/milestone/1) |
| The outcome is a shared Buzz platform the cohort can safely depend on and recreate — not merely "Buzz is running" | `DECIDED` | [milestone M0](https://github.com/launchpad-26/buzz/milestone/1) |
| The cohort can answer "What is changing?" without first knowing which repository, changelog, issue, PR or documentation file holds the answer | `DECIDED` | [milestone M1](https://github.com/launchpad-26/buzz/milestone/2) |
| The cohort can answer "How does this work?" on the same terms | `DECIDED` | [milestone M1](https://github.com/launchpad-26/buzz/milestone/2) |
| Agents initiated on the relay do useful work assisting development of `launchpad-26/rhizomorph` | `PROPOSED` | beyond both current milestones; direction recorded in [#42](https://github.com/launchpad-26/buzz/issues/42) |

`DECIDED` here means the measure is agreed and recorded on its milestone, and is not met
yet. None of these is `IMPLEMENTED`.

---

## Scope boundaries

| Concern | In this cohort's scope? | Status |
|---|---|---|
| Deploying Buzz and configuring the host it runs on | Yes | `DECIDED` — [milestone M0](https://github.com/launchpad-26/buzz/milestone/1) |
| Operating the relay and the community running on it | Yes | `DECIDED` — [milestone M0](https://github.com/launchpad-26/buzz/milestone/1) |
| Documenting what the cohort is doing with Buzz and why | Yes | `IMPLEMENTED` for this document; the wider set is [#42](https://github.com/launchpad-26/buzz/issues/42) |
| Automating deployment, CI/CD and agent workflows around Buzz | Yes | `DECIDED` — [milestone M0](https://github.com/launchpad-26/buzz/milestone/1) and [milestone M1](https://github.com/launchpad-26/buzz/milestone/2) |
| Developing Buzz itself — Rust crates, desktop, mobile | No. Genuine product bugs go to [`block/buzz` issues](https://github.com/block/buzz/issues) | `IMPLEMENTED` — [`AGENTS.md` §1](AGENTS.md) |
| Publishing the knowledge layer — the MkDocs surface, page contract, provenance gates and synthesised content | No. Owned by [#4](https://github.com/launchpad-26/buzz/issues/4) on [milestone M1](https://github.com/launchpad-26/buzz/milestone/2), per [#42](https://github.com/launchpad-26/buzz/issues/42)'s non-goals | `OPEN` — [#4](https://github.com/launchpad-26/buzz/issues/4) |

This document produces source material a later synthesis can draw from. If its content
ends up published, it is published by [#4](https://github.com/launchpad-26/buzz/issues/4)'s
machinery, not by anything here.

---

## Relationship to rhizomorph

`launchpad-26/rhizomorph` is a public sibling project in the same organisation, described
as "A live, replayable localhost dashboard for a git-worktree agent swarm — watches
worktrees, branches, tmux panes, and workmux state, read-only."

[ADR-0002](decisions/ADR-0002-handbook-source-repository-scope.md) records its role for
the handbook as "Agent observability tooling", carrying the `[supporting]` origin prefix.

[#43](https://github.com/launchpad-26/buzz/issues/43) records the asymmetry that matters
most between the two projects: "Sibling project `launchpad-26/rhizomorph` is deliberately
read-only; agents deployed to assist its development are not, and that delta is exactly
what needs containing".

| Concern | Owner | Status |
|---|---|---|
| Coordination surface, agent initiation, relay operation | `launchpad-26/buzz` (this repository) | `OPEN` — nothing deployed |
| Observing a git-worktree agent swarm on a developer's machine | `launchpad-26/rhizomorph` | `IMPLEMENTED` (that project) |
| Containment at the execution boundary between them | undecided | `OPEN` — [#43](https://github.com/launchpad-26/buzz/issues/43) |

---

## Adding to this document

> Additions arrive by pull request against `launchpad`. Every new claim carries a status
> marker and a link to its evidence. Anything not yet true is an issue, not a line here —
> see [`AGENTS.md` §2](AGENTS.md). Append within a section rather than renumbering
> headings, so links from issues and the handbook keep resolving.
