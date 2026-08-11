# Target architecture

The intended end state of the launchpad-26 cohort's Buzz environment: what runs where, who
owns which part, and which parts do not exist yet.

This document records a target and an ownership split. It does not build any of it, and it
does not settle anything an open decision owns — where a choice is open it says so and
links the issue.

---

## How to read this

Every claim below carries exactly one status marker, read as defined in
[VISION.md § How to read this](VISION.md#how-to-read-this). The legend lives there and is
not restated here.

---

## The intended end state

`PROPOSED` — no part of the diagram below is deployed. The direction it draws is recorded
in [#42](https://github.com/launchpad-26/buzz/issues/42); the containment question it
raises is `OPEN` — [#43](https://github.com/launchpad-26/buzz/issues/43).

```
                 contributors' machines
                    |              ^
       (1) connect  |              | (3) agent executes here
                    v              |
              +-------------------------+
              |  Buzz relay  (the VPS)  |  <- (2) agent initiated here
              +-------------------------+
                          |
                   work targets
              launchpad-26/rhizomorph
```

Reading the three arrows:

1. Contributors connect their Buzz clients to a single cohort relay, which replaces
   Discord as the coordination surface.
2. An agent is initiated on the relay — the relay is where the work is asked for.
3. The agent executes on a contributor's own machine, not on the relay, and the work it
   does is directed at `launchpad-26/rhizomorph`.

The diagram is a topology, not a deployment. What is actually running on 2026-08-11 is in
[What exists today](#what-exists-today).

---

## Components and who owns them

| Component | Owner | Status |
|---|---|---|
| Buzz relay (`crates/buzz-relay`) | upstream `block/buzz` | `IMPLEMENTED` upstream; not deployed by the cohort |
| Desktop, mobile and web clients | upstream `block/buzz` | `IMPLEMENTED` upstream |
| Agent harness `buzz-acp` → `buzz-agent` → `buzz-dev-mcp` | upstream `block/buzz` | `IMPLEMENTED` upstream |
| Relay deployment and host configuration | cohort | `OPEN` — [#5](https://github.com/launchpad-26/buzz/issues/5), [#22](https://github.com/launchpad-26/buzz/issues/22), [#24](https://github.com/launchpad-26/buzz/issues/24) |
| Relay datastores — PostgreSQL, Redis, and object storage such as MinIO | cohort | `OPEN` — [#5](https://github.com/launchpad-26/buzz/issues/5) |
| Containment at the agent execution boundary | cohort | `OPEN` — [#43](https://github.com/launchpad-26/buzz/issues/43) |
| Upstream intelligence | cohort | `OPEN` — [#3](https://github.com/launchpad-26/buzz/issues/3) |
| Knowledge layer | cohort | `OPEN` — [#4](https://github.com/launchpad-26/buzz/issues/4) |

Each row carries its own marker, and those markers cover only the rows present when this
section was written. A component added later does not inherit any of them — give a new row
its own status marker and its own link to evidence.

The upstream components above are directories in this repository:
[`crates/buzz-relay`](../crates/buzz-relay), [`desktop`](../desktop),
[`mobile`](../mobile), [`web`](../web), [`crates/buzz-acp`](../crates/buzz-acp),
[`crates/buzz-agent`](../crates/buzz-agent) and
[`crates/buzz-dev-mcp`](../crates/buzz-dev-mcp). "Owner: upstream `block/buzz`" means the
cohort operates them and does not change them — see
[`AGENTS.md`](AGENTS.md) §1.

The datastore row is drawn because the relay's state is load-bearing rather than
incidental: the `communities` row each environment depends on is a Postgres table created
by [`migrations/0001_initial_schema.sql`](../migrations/0001_initial_schema.sql), and the
consequence of it being absent is in
[ENVIRONMENTS.md § What must never differ between them](ENVIRONMENTS.md#what-must-never-differ-between-them).
The component names are those in
[#5](https://github.com/launchpad-26/buzz/issues/5)'s Evidence section; the marker is
`OPEN` because the cohort has deployed none of them.

The split of concerns between this repository and `launchpad-26/rhizomorph` is recorded in
[VISION.md § Relationship to rhizomorph](VISION.md#relationship-to-rhizomorph) and is not
restated here.

---

## How an agent is initiated and where it executes

The upstream capability and the cohort's use of it are two different claims with two
different statuses, and are kept apart here on purpose.

| Claim | Status |
|---|---|
| Buzz can initiate an agent and run it as the `buzz-acp` → `buzz-agent` → `buzz-dev-mcp` process tree, carrying the production MCP toolset — shell, file tools, todo — with the `buzz` CLI on the shell's `PATH` | `IMPLEMENTED` upstream — [`benchmarks/harbor-buzz-orchestra/README.md`](../benchmarks/harbor-buzz-orchestra/README.md), [`crates/buzz-acp`](../crates/buzz-acp), [`crates/buzz-agent`](../crates/buzz-agent), [`crates/buzz-dev-mcp`](../crates/buzz-dev-mcp) |
| The cohort initiating agents on its own relay and executing them on contributors' machines | `OPEN` — [#43](https://github.com/launchpad-26/buzz/issues/43) |

Those markers cover only the rows present when this section was written; a claim added
later carries its own marker.

The consequence of the first row is what the next section is about: the toolset an agent
starts with already includes a shell on the machine it executes on. That is upstream's
default, not something the cohort would have to add.

---

## The execution boundary

The point where a relay-initiated agent executes is the control that matters most in this
architecture, and it is undecided.

[#43](https://github.com/launchpad-26/buzz/issues/43) states the exposure plainly: "the
execution targets here are contributors' daily-driver laptops, holding their SSH keys,
credentials and unrelated work". On the toolset described above, the same issue says:
"Shell access on a contributor's machine is the default starting position, not an
escalation."

The containment mechanism at that boundary is `OPEN` —
[#43](https://github.com/launchpad-26/buzz/issues/43). That issue lists candidate options
and deliberately leaves its Decision outcome blank, citing [`AGENTS.md`](AGENTS.md) §5
rule 1: draft everything, approve nothing.

This section names the risk and stops. It does not narrow the options, because
[#42](https://github.com/launchpad-26/buzz/issues/42)'s non-goals reserve the agent
execution security model to that ADR — and a proposal presented as a decision is worse
than an absent document.

---

## What exists today

Nothing in the architecture above is deployed. As of 2026-08-11 the cohort runs Buzz on
`localhost` only, and both of the issues that would change that are open.

| Work that would change this | Status |
|---|---|
| Deploy the relay to the VPS with real DNS and TLS | `OPEN` — [#22](https://github.com/launchpad-26/buzz/issues/22), on [M0 — Buzz MVP](https://github.com/launchpad-26/buzz/milestone/1) |
| Verify two people on two machines can talk on the deployed relay | `OPEN` — [#23](https://github.com/launchpad-26/buzz/issues/23), on [M0 — Buzz MVP](https://github.com/launchpad-26/buzz/milestone/1) |

Both states were read from the live issues with `gh issue view` on 2026-08-11. If either
has since closed, this section is stale — correct it in the same pull request that changes
what is running.

The cohort-owned work beyond deployment sits on
[M1 — Cohort Intelligence & Knowledge](https://github.com/launchpad-26/buzz/milestone/2):
upstream intelligence ([#3](https://github.com/launchpad-26/buzz/issues/3)) and the
knowledge layer ([#4](https://github.com/launchpad-26/buzz/issues/4)). Neither has started.

---

## Open decisions

Every decision this architecture depends on that has not been taken. Titles are the issue
titles as filed.

| Decision | Status |
|---|---|
| adr: VPS specification for the cohort Buzz relay | `OPEN` — [#21](https://github.com/launchpad-26/buzz/issues/21) |
| adr: configuration-management tool, Ubuntu baseline and service runtime shape | `OPEN` — [#24](https://github.com/launchpad-26/buzz/issues/24) |
| adr: CI/CD deployment identity, privilege boundary and secret storage | `OPEN` — [#25](https://github.com/launchpad-26/buzz/issues/25) |
| adr: administrative access model after bootstrap | `OPEN` — [#26](https://github.com/launchpad-26/buzz/issues/26) |
| adr: security patch and reboot policy | `OPEN` — [#27](https://github.com/launchpad-26/buzz/issues/27) |
| adr: what must survive destruction of the host | `OPEN` — [#28](https://github.com/launchpad-26/buzz/issues/28) |
| adr: agent execution security model — containment for relay-initiated agents on contributor machines | `OPEN` — [#43](https://github.com/launchpad-26/buzz/issues/43) |
| adr: host firewall implementation for the deny-by-default policy | `OPEN` — [#44](https://github.com/launchpad-26/buzz/issues/44) |
| adr: AppArmor and service confinement for the Buzz runtime | `OPEN` — [#45](https://github.com/launchpad-26/buzz/issues/45) |
| adr: which security checks gate deployment in CI/CD | `OPEN` — [#46](https://github.com/launchpad-26/buzz/issues/46) |
| adr: minimum external security smoke test before a deployment is declared healthy | `OPEN` — [#47](https://github.com/launchpad-26/buzz/issues/47) |
| adr: observability strategy for the cohort Buzz environment | `OPEN` — [#83](https://github.com/launchpad-26/buzz/issues/83) |

The last row is here and not in
[SECURITY-POSTURE.md § Open security decisions](SECURITY-POSTURE.md#open-security-decisions):
it decides what the cohort runs to watch itself — host logs, infrastructure metrics and
agent traces are three signals with three possible destinations — which is a shape
question for this architecture. It has a security consequence, stated in
[#83](https://github.com/launchpad-26/buzz/issues/83), but the posture's own dependency on
security-event visibility is already carried by
[#34](https://github.com/launchpad-26/buzz/issues/34) in
[SECURITY-POSTURE.md § What is true today](SECURITY-POSTURE.md#what-is-true-today), and
[#83](https://github.com/launchpad-26/buzz/issues/83) defines no control that the posture
asserts.

Those markers cover only the rows present when this section was written, all confirmed
`OPEN` against the live issues on 2026-08-11. A decision added later carries its own
marker, and a decision that is taken becomes `DECIDED` with a link to its accepted ADR in
[`decisions/`](decisions) — see [`AGENTS.md`](AGENTS.md) §2.

---

## Adding to this document

> Additions arrive by pull request against `launchpad`. Every new claim carries a status
> marker and a link to its evidence. Anything not yet true is an issue, not a line here —
> see [`AGENTS.md` §2](AGENTS.md). Append within a section rather than renumbering
> headings, so links from issues and the handbook keep resolving.
