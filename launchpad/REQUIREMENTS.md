# Requirements

What the launchpad-26 cohort's Buzz environment must do and must be, as individually
identified requirements a reader can prioritise, trace to a source and check.

This document records requirements. It builds none of them, and it does not settle
anything an open decision owns — where a choice is open it says so and links the issue.

---

## How to read this

Every requirement below carries exactly one status marker, read as defined in
[VISION.md § How to read this](VISION.md#how-to-read-this). The legend lives there and is
not restated here.

Every requirement also carries a MoSCoW priority — `Must`, `Should`, `Could` or `Won't`.

**Priority and status are different axes, and conflating them is the obvious misreading.**
Priority says how important the requirement is to the cohort. Status says whether it is
satisfied. A `Must` that is `OPEN` is not a contradiction: it is the most important kind of
row in this document, because it is both required and unmet. Nothing here is `IMPLEMENTED`
merely for being a `Must`.

No requirement below is prioritised `Won't`. What the cohort has decided *not* to do is
recorded as scope, in
[VISION.md § Scope boundaries](VISION.md#scope-boundaries), rather than as a requirement
with an emptied-out priority.

---

## How this document is structured

The sections below follow the shape of the Business Requirements Specification described
by ISO/IEC/IEEE 29148:2018 — business purpose, business scope, business overview, business
environment, stakeholders, then the requirements themselves.

**This document does not claim conformance to that standard.** The standard's full text is
paywalled, nobody in the cohort has read it in full, and nothing here has been checked
against its clauses. Only the outline was available, from secondary sources. Saying
"structured following" is the whole of the claim: an unverified conformance assertion in a
public repository is exactly the failure the status markers in this document set exist to
prevent.

The requirement objects carry the metadata shape used by the reference corpus at
[`tucktuck101/ForgePilot` `docs/business-analysis/`](https://github.com/tucktuck101/ForgePilot/tree/main/docs/business-analysis)
— an identifier, a MoSCoW priority, a source, and a summary carrying one dominant
obligation — and take the idea of a stated verification method from that corpus's
[`requirements/quality-and-verification-baseline.md`](https://github.com/tucktuck101/ForgePilot/blob/main/docs/business-analysis/requirements/quality-and-verification-baseline.md),
which supplies an observable method for every non-functional requirement it holds.

Where this document departs from that corpus, it does so deliberately. It states every
obligation with `shall`; the corpus does that for its business requirements but not
throughout, also using `must`, `should` and `may`, and its baseline treats `shall` and
`must` as mandatory and `should` as preferred. It also cites sources directly instead of
modelling stakeholder needs and evidence as separately addressable objects, which
[#91](https://github.com/launchpad-26/buzz/issues/91) records as a deliberate non-goal at
this cohort's size.

That corpus makes no reference to ISO/IEC/IEEE 29148 and is not offered as evidence of
conformance to it. It was read on 2026-08-12, and the metadata claims above were checked
against every requirement object in its requirements collections rather than sampled.

---

## Business purpose

Owned by [VISION.md § Mission](VISION.md#mission) and not restated here: the cohort
operates Buzz rather than developing it, and deploys and runs a shared Buzz platform it can
safely depend on and recreate.

---

## Business scope

Owned by [VISION.md § Scope boundaries](VISION.md#scope-boundaries) and not restated here.
That table is what decides whether a candidate requirement belongs in this document at all.

---

## Business overview

Owned by [ARCHITECTURE.md § The intended end state](ARCHITECTURE.md#the-intended-end-state)
and not restated here: contributors connect to a single cohort relay, agents are initiated
on that relay, and the agent work executes on contributors' own machines.

---

## Business environment

Where these requirements have to hold is owned by
[ENVIRONMENTS.md § The environments](ENVIRONMENTS.md#the-environments), and the state of the
controls that would satisfy the security-shaped ones is owned by
[SECURITY-POSTURE.md § What is true today](SECURITY-POSTURE.md#what-is-true-today). Neither
is restated here.

Environmental facts that bear directly on how these requirements are written, each recorded
elsewhere in this set:

- The repository is public, so no requirement below names a hostname, a credential or a
  person — [SECURITY-POSTURE.md § The public-repository rule](SECURITY-POSTURE.md#the-public-repository-rule).
- The cohort is not yet running the environment these requirements describe, so most of
  them are unmet — [ARCHITECTURE.md § What exists today](ARCHITECTURE.md#what-exists-today).

---

## Stakeholders

Owned by [VISION.md § Who this is for](VISION.md#who-this-is-for) and not restated here.
Each requirement below serves one or more of the audiences in that table.

---

## Why there are no functional requirements here

**The cohort operates Buzz and does not develop it, so what Buzz does is not the cohort's
to require.** That boundary is [`AGENTS.md` §1](AGENTS.md): "We operate Buzz. We do not
develop Buzz."

A functional requirement for the product — what the relay accepts, how a client renders a
channel, which event kinds exist — would be a requirement written on
[`block/buzz`](https://github.com/block/buzz)'s roadmap by people with no standing to write
it. It would also rot silently, because upstream would change the behaviour without ever
reading this file. Genuine product defects go to
[`block/buzz` issues](https://github.com/block/buzz/issues).

What the cohort *does* own is the environment around the product: that it exists, that it
is reachable by the right people, that it can be rebuilt, and that running it does not
endanger the machines and credentials of the people running it. Those are business
requirements and non-functional requirements, which is why this document holds only those
two classes.

Properties of the shipped software that the cohort must design around, rather than require,
are recorded as invariants in
[ENVIRONMENTS.md § What must never differ between them](ENVIRONMENTS.md#what-must-never-differ-between-them).

---

## Business requirements

Each row states one dominant obligation. `Source` is a link a reader can follow to the
milestone, issue or file the obligation comes from.

| ID | Requirement | Priority | Status | Source |
|---|---|---|---|---|
| BR-001 | The cohort **shall** operate a shared internet-facing Buzz relay that multiple cohort members can connect to and communicate through. | `Must` | `DECIDED` | [milestone M0](https://github.com/launchpad-26/buzz/milestone/1); [#2](https://github.com/launchpad-26/buzz/issues/2) |
| BR-002 | The cohort **shall** move its own coordination onto that relay rather than continuing to coordinate on Discord. | `Should` | `PROPOSED` | [#42](https://github.com/launchpad-26/buzz/issues/42), via [VISION.md § What we are building](VISION.md#what-we-are-building) |
| BR-003 | The cohort **shall** be able to answer what is changing and how the system works from within Buzz, "without first needing to know which repository, changelog, issue, PR or documentation file contains the answer". | `Should` | `DECIDED` | [milestone M1](https://github.com/launchpad-26/buzz/milestone/2) |
| BR-004 | Agent work initiated on the relay **shall** be directed at assisting development of `launchpad-26/rhizomorph`. | `Should` | `PROPOSED` | [#42](https://github.com/launchpad-26/buzz/issues/42); conditional on NFR-012 |
| BR-005 | The cohort **shall** deploy and maintain the server through source-controlled automation rather than through manual administration of a running host. | `Must` | `DECIDED` | [milestone M0](https://github.com/launchpad-26/buzz/milestone/1); [#5](https://github.com/launchpad-26/buzz/issues/5) Ruling 1 |
| BR-006 | The cohort **shall** operate Buzz and not develop it, routing genuine product defects to `block/buzz`. | `Must` | `IMPLEMENTED` | [`AGENTS.md` §1](AGENTS.md) |
| BR-007 | The cohort **shall** hold its stable operating knowledge in this repository as documentation, rather than in the recollection of whoever configured something. | `Should` | `PROPOSED` | [#42](https://github.com/launchpad-26/buzz/issues/42); [`AGENTS.md` §2](AGENTS.md) |

The markers above cover only the rows present when this section was written. A requirement
added later does not inherit one — it carries its own status marker, its own priority and
its own link to a source.

`DECIDED` here means the obligation is agreed and recorded on the milestone or issue named,
and is not met yet, exactly as in
[VISION.md § What success looks like](VISION.md#what-success-looks-like).

### How each business requirement is demonstrated

| ID | What would demonstrate it holds |
|---|---|
| BR-001 | Two cohort members, on two different machines, each connect to the deployed relay with their own Nostr identity, see each other's messages in a shared channel, and exchange a direct message in both directions. |
| BR-002 | The coordination that previously happened on Discord happens in relay channels, and the cohort's dependence on Discord ends by a recorded decision rather than by drift. |
| BR-003 | A member and an agent each answer one question about a recent upstream change and one about how a part of the system works, starting from Buzz, without first being told which repository holds the answer. |
| BR-004 | An agent initiated on the relay produces reviewable work against `launchpad-26/rhizomorph`, and no such agent has executed on a contributor's machine before NFR-012 holds. |
| BR-005 | A documented command applied to a fresh supported Ubuntu host produces a working configuration with no manual step outside the controlled bootstrap that [#5](https://github.com/launchpad-26/buzz/issues/5) Ruling 2 defines as the deployment's input, and the security controls arrive from the same version-controlled automation as the rest of the host configuration. Ruling 1 requires that "where practical", so anything that cannot be expressed as automation is declared rather than left as an undocumented requirement of the running host. |
| BR-006 | Cohort changes land under `launchpad/`, `.github/workflows/launchpad-*` and cohort process files; defects in the product appear as issues on `block/buzz` rather than as patches to upstream directories in this fork. |
| BR-007 | A contributor joining later can state what the system is, and the access an agent workflow needs together with its blast radius, from the documents in `launchpad/` alone. |

---

## Non-functional requirements

Standing properties the environment must hold. Each carries one dominant obligation, a
priority, a status and a source; the verification method for each is in the matrix that
follows.

| ID | Property | Requirement | Priority | Status | Source |
|---|---|---|---|---|---|
| NFR-001 | Recoverability | The environment **shall** be rebuildable from a bare supported Ubuntu host to an equivalent functional and security state. | `Must` | `DECIDED` | [milestone M0](https://github.com/launchpad-26/buzz/milestone/1); [#5](https://github.com/launchpad-26/buzz/issues/5) Ruling 14 |
| NFR-002 | Idempotent convergence | Reapplying the deployment automation to an already-converged host **shall** report no changes. | `Must` | `DECIDED` | [#5](https://github.com/launchpad-26/buzz/issues/5) Ruling 11 |
| NFR-003 | Least privilege | No administrative, application or service identity **shall** hold access beyond its role. | `Must` | `DECIDED` | [milestone M0](https://github.com/launchpad-26/buzz/milestone/1); [#5](https://github.com/launchpad-26/buzz/issues/5) Rulings 4 and 9 |
| NFR-004 | Explicit trust | Access between people, automation identities and services **shall** rest on explicit identity and authorisation rather than on network location. | `Must` | `DECIDED` | [milestone M0](https://github.com/launchpad-26/buzz/milestone/1); [#5](https://github.com/launchpad-26/buzz/issues/5) Ruling 4 |
| NFR-005 | Minimal exposure | Only the services intended to be public **shall** be reachable from the internet. | `Must` | `DECIDED` | [milestone M0](https://github.com/launchpad-26/buzz/milestone/1); [#5](https://github.com/launchpad-26/buzz/issues/5) Ruling 5 |
| NFR-006 | Transport protection | Public Buzz traffic **shall** be TLS-protected. | `Must` | `DECIDED` | [#5](https://github.com/launchpad-26/buzz/issues/5); [#2](https://github.com/launchpad-26/buzz/issues/2) |
| NFR-007 | Credential handling | A routine deployment **shall** complete without a root credential being distributed to any cohort member. | `Must` | `DECIDED` | [milestone M0](https://github.com/launchpad-26/buzz/milestone/1); [#5](https://github.com/launchpad-26/buzz/issues/5) Rulings 6 and 7 |
| NFR-008 | Secret containment | Secret material **shall** be kept out of tracked files by a check rather than by attention. | `Must` | `OPEN` | [#67](https://github.com/launchpad-26/buzz/issues/67) for detection in diffs and history, [#68](https://github.com/launchpad-26/buzz/issues/68) for the ignore-pattern and tracked-file assertions, both under [#62](https://github.com/launchpad-26/buzz/issues/62); [`AGENTS.md` §8](AGENTS.md) |
| NFR-009 | Security maintenance | Supported Ubuntu security updates **shall** stay applied on the running host, with reboot-required state visible rather than assumed. | `Must` | `DECIDED` | [#5](https://github.com/launchpad-26/buzz/issues/5) Ruling 10 |
| NFR-010 | Observability of failure | Security-relevant failures **shall** remain observable to authorised operators after hardening. | `Should` | `OPEN` | [#34](https://github.com/launchpad-26/buzz/issues/34); [#5](https://github.com/launchpad-26/buzz/issues/5) Ruling 13 |
| NFR-011 | External verification | A deployment **shall** be declared healthy only after a machine-checkable suite verifies it from an untrusted client's position, not because the configuration run succeeded. | `Must` | `DECIDED` | [#5](https://github.com/launchpad-26/buzz/issues/5) Ruling 12; the minimum set is `OPEN` — [#47](https://github.com/launchpad-26/buzz/issues/47) |
| NFR-012 | Containment at the agent execution boundary | No agent initiated over the relay **shall** execute on a contributor's machine before containment at that boundary exists. | `Must` | `OPEN` | [#43](https://github.com/launchpad-26/buzz/issues/43) |
| NFR-013 | Workload measurability | The cost of a change to the running stack **shall** be measured against a recorded baseline before the change is adopted. | `Should` | `OPEN` | [#41](https://github.com/launchpad-26/buzz/issues/41) |

The markers above cover only the rows present when this section was written. A property
added later does not inherit one — it carries its own status marker, its own priority, its
own source and its own row in the matrix below.

NFR-008 is `OPEN` even though the rule it enforces binds today. The rule's existence is
`IMPLEMENTED` — [`AGENTS.md` §8](AGENTS.md) — and
[SECURITY-POSTURE.md § The public-repository rule](SECURITY-POSTURE.md#the-public-repository-rule)
draws the distinction this requirement turns on: "A rule that binds is not a mechanism that
checks." The requirement is for the check, and nothing checks yet.

NFR-012 states that containment is required and stops there. It does not name a mechanism,
rank the options or imply a direction, because
[#43](https://github.com/launchpad-26/buzz/issues/43) owns that decision and deliberately
left its outcome blank, citing [`AGENTS.md`](AGENTS.md) §5 rule 1: draft everything,
approve nothing. The exposure it bounds is described in
[ARCHITECTURE.md § The execution boundary](ARCHITECTURE.md#the-execution-boundary) and is
not restated here.

### Verification matrix

What someone could do to demonstrate each property holds. "It was tested" is not a
verification method; each row names an observation whose outcome a reader could check.

| ID | Observable verification |
|---|---|
| NFR-001 | A replacement bare supported Ubuntu host, given only replacement infrastructure coordinates and controlled bootstrap credentials, is brought to a state where NFR-011's suite passes, with no step taken that is not in the automation or its documented bootstrap. |
| NFR-002 | The automation is applied twice in succession to the same host and the second run reports zero changed resources. Anything that cannot safely be idempotent is declared as an explicit exception in the automation rather than discovered during a run. |
| NFR-003 | The declared identities are inspected in version control: each administrative, application and service identity's permissions are readable there and map to a stated role, and the Buzz runtime and its supporting services run under identities constrained below root. |
| NFR-004 | For every access path between a person, an automation identity and a service, an identity and an authorisation can be named. No path is permitted on the basis of co-location, source address or `localhost` alone. |
| NFR-005 | An external port scan, run from a client the cohort does not control, reaches only the intentionally public services; the database, cache and object-storage administration interfaces do not answer. |
| NFR-006 | A plaintext connection attempt to the public endpoint fails; the `wss://` endpoint connects from outside the host; the `https://` endpoint returns NIP-11 JSON and `/health` returns 200. |
| NFR-007 | A routine deployment completes end to end through the dedicated CI/CD machine identity while no cohort member holds a root credential, and remote root access on the host matches the declared policy when checked from outside it. |
| NFR-008 | The audit flags deliberately planted secret-shaped fixtures in a pull-request diff and in git history, reporting file and line without echoing the matched value, and fails when a file matching an ignore-list pattern is tracked. A check nobody has watched fail is not known to work. |
| NFR-009 | On the running host, the update policy is active and evidenced, and a check reports pending-reboot state rather than an operator inferring currency from the fact that packages were installed. |
| NFR-010 | An authentication failure and a denied privileged action are each provoked deliberately and then located in the host's records by an authorised operator, with no secret appearing in what is retrieved. |
| NFR-011 | The suite runs against the deployed host from both an authorised user's and an untrusted internet client's position, passes, and is the thing whose result declares the deployment healthy. |
| NFR-012 | Before the first relay-initiated agent executes on a contributor's machine, the containment decision exists as an accepted ADR in [`decisions/`](decisions), and the containment it records can be demonstrated from outside the agent — per [#43](https://github.com/launchpad-26/buzz/issues/43): "Any control that cannot be verified from outside the agent is an assumption, not a control." |
| NFR-013 | The harness drives human-authenticated and agent-authenticated connections against a target it was explicitly given, refuses to run without one, confirms events were accepted before reporting any figure, and reports each change as a delta against a recorded baseline that states the conditions it was measured under. |

Every row above is a method, not a result. None of them has been run: the cohort is not
running the environment they would be run against —
[ARCHITECTURE.md § What exists today](ARCHITECTURE.md#what-exists-today).

---

## Traceability

Requirement → the work that would satisfy it → the milestone that work sits on. Issue
titles are as filed.

| ID | Satisfying work | Milestone |
|---|---|---|
| BR-001 | [#2](https://github.com/launchpad-26/buzz/issues/2) — prd 0 - Deploy the relay to a VPS and verify two people can log in | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| BR-002 | No work item owns the migration itself; the direction is recorded in [#42](https://github.com/launchpad-26/buzz/issues/42) | — |
| BR-003 | [#3](https://github.com/launchpad-26/buzz/issues/3) — prd-01 — upstream intelligence: Buzz keeps the cohort aware of upstream; [#4](https://github.com/launchpad-26/buzz/issues/4) — # prd 02— human + agent knowledge layer: one coherent surface for Buzz | [M1](https://github.com/launchpad-26/buzz/milestone/2) |
| BR-004 | No work item owns the agent workflows; the direction is recorded in [#42](https://github.com/launchpad-26/buzz/issues/42) and is blocked by [#43](https://github.com/launchpad-26/buzz/issues/43) | [M0](https://github.com/launchpad-26/buzz/milestone/1) for [#43](https://github.com/launchpad-26/buzz/issues/43) |
| BR-005 | [#5](https://github.com/launchpad-26/buzz/issues/5) — # prd-03 — reproducible and hardened Buzz deployment: rebuild the cohort server from a bare Ubuntu host | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| BR-006 | No work item — the boundary is in force as a rule, [`AGENTS.md` §1](AGENTS.md) | — |
| BR-007 | [#42](https://github.com/launchpad-26/buzz/issues/42) — prd-06 — capture the cohort's vision, architecture and operating documentation | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| NFR-001 | [#5](https://github.com/launchpad-26/buzz/issues/5) | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| NFR-002 | [#5](https://github.com/launchpad-26/buzz/issues/5) | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| NFR-003 | [#5](https://github.com/launchpad-26/buzz/issues/5) | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| NFR-004 | [#5](https://github.com/launchpad-26/buzz/issues/5) | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| NFR-005 | [#5](https://github.com/launchpad-26/buzz/issues/5) | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| NFR-006 | [#5](https://github.com/launchpad-26/buzz/issues/5); [#2](https://github.com/launchpad-26/buzz/issues/2) | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| NFR-007 | [#5](https://github.com/launchpad-26/buzz/issues/5) | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| NFR-008 | [#62](https://github.com/launchpad-26/buzz/issues/62) — prd-07 — repository security hygiene: an enforced audit sequence for a public, agent-heavy fork | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| NFR-009 | [#5](https://github.com/launchpad-26/buzz/issues/5) | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| NFR-010 | [#34](https://github.com/launchpad-26/buzz/issues/34) — task: keep security-relevant events observable after hardening | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| NFR-011 | [#5](https://github.com/launchpad-26/buzz/issues/5); [#47](https://github.com/launchpad-26/buzz/issues/47) — adr: minimum external security smoke test before a deployment is declared healthy | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| NFR-012 | [#43](https://github.com/launchpad-26/buzz/issues/43) — adr: agent execution security model — containment for relay-initiated agents on contributor machines | [M0](https://github.com/launchpad-26/buzz/milestone/1) |
| NFR-013 | [#41](https://github.com/launchpad-26/buzz/issues/41) — prd-05 — internal performance and agent-workload testing harness | — |

A row carries no milestone where the work is not on one:
[#41](https://github.com/launchpad-26/buzz/issues/41) was filed without a milestone, and a
requirement with no work item has nothing to place on one. **Every requirement that nobody
currently owns says so in its `Satisfying work` cell, which begins "No work item".** Read
that column to find them all — a list here would leave a later addition out, and an
unowned obligation going unnoticed is the problem this document exists to prevent.
Every issue state and milestone in this table was read from the live issues with `gh` on
2026-08-12; if any has since closed or moved, this section is stale — correct it in the
same pull request that changes it. That reading covers only the rows present when this
section was written. A requirement added later brings its own row and its own reading.

A requirement outlives the work that satisfies it. When an issue in this table closes, the
requirement does not close with it: its status marker changes and the row keeps pointing at
the work as the record of how it was met.

---

## Adding to this document

> Additions arrive by pull request against `launchpad`. Every new claim carries a status
> marker and a link to its evidence. Anything not yet true is an issue, not a line here —
> see [`AGENTS.md` §2](AGENTS.md). Append within a section rather than renumbering
> headings, so links from issues and the handbook keep resolving.

A new requirement additionally carries its own identifier taken from the next unused number
in its class, one dominant obligation stated with `shall`, a MoSCoW priority, a status
marker, and a source someone can follow to a milestone, issue or file. A non-functional
requirement also carries a row in the verification matrix naming an observation, not a
result. Retired identifiers are not reused, so a citation from an issue or a pull request
keeps meaning what it meant.
