---
status: Proposed
date: 2026-08-15
issue: launchpad-26/buzz#47
decided_in: launchpad-26/buzz#47
supersedes: none
---

# ADR-0011 — Minimum external security smoke test before a deployment is declared healthy

## Decision

**Floor B** — reachability and TLS (Floor A), plus the negative assertions: an off-host
scan confirming PostgreSQL, Redis, object-storage administration, and the relay's own
health (`8080`) and metrics (`9102`) ports are **not** reachable from the internet, and no
other unexpected port answers.

Floor A alone is rejected: it passes cleanly on a host that also has PostgreSQL open to
the internet, certifying the one thing that was never in doubt while missing the failure
#5's success criteria are written around. Floor D is rejected for the opposite reason:
setting the minimum equal to the full external-verification suite's scope means nothing
can be declared healthy until the last check exists, leaving the deploy step with no bar
to satisfy in the meantime.

## Context

The bundle's own validation is an inside-the-host check —
`curl -fsS "http://127.0.0.1:.../_liveness"` from `deploy/compose/README.md` — loopback,
plaintext, on the machine being tested. It proves the relay process is up and proves
nothing about what the internet can reach, which is exactly the gap Ruling 12 (#5) exists
to close. Reading the bundle: `compose.caddy.yml` removes the relay's own port
(`ports: !reset []`) and publishes only `80`/`443` via Caddy; the relay's health and
metrics ports are configured but never published; `compose.dev.yml` would publish six
additional ports if it were ever enabled on the VPS.

Floor C (this decision plus image-version-pin verification, plus remote privileged access
matching #26's policy) is the natural next step, but is not decided here — it depends on
#26, which is still open. Floor B is explicitly callable now, including by hand against the
destroyable VM, before any CI pipeline or #26 exist.

## Consequences

**Good.** A named floor turns "deployed" into a claim with evidence behind it, and gives
#35 (the verification suite) a concrete subset to build first, and #22 something to
satisfy before the relay is handed to the cohort. Ruling 12's remaining items are recorded
as deliberately deferred rather than silently dropped.

**Bad, stated honestly.** Any floor is also a ceiling in practice — checks outside it tend
not to get written. The negative assertions are the expensive part: proving a port is
closed requires scanning rather than reading configuration, and provider-side filtering
could make a misconfigured host look correctly closed by accident rather than by design.

**Contingency — the accepted gap, and when to close it.** Floor B does not confirm the
deployed relay is running the *pinned* image version — only that the network surface is
correct. A deployment could pass every Floor B check while still running a stale or wrong
build, and Floor B alone would call that "healthy."

*Trigger for upgrading to Floor C:* either #26 lands (making Floor C's remote-access-policy
half decidable), or a real incident where a deployment passed Floor B while running the
wrong version — whichever comes first.

*The fix:* add Floor C's two checks on top of Floor B — confirm the deployed image matches
the pinned `BUZZ_IMAGE`, and confirm remote privileged access matches #26's eventual
policy. Nothing about Floor B needs reworking; Floor C is additive.

*The safety net in the meantime:* the image-pin problem is largely covered already, just
not from off-host — this fork's deploy guard (fixed under #141) rejects floating tags
before a deployment can even start, and the ADR-0005 boundary check enforces the same
image-namespace constraint in CI. Floor C's marginal value is *external, off-host*
confirmation of what those internal checks already assert, not a first line of defense
against the gap.

## Security implications

The purpose of this floor is to make the negative claims testable, not merely assumed.
Positive checks (the relay answers, TLS works) fail loudly and are hard to get wrong; the
claims that actually carry the security posture are the ones about what is *not*
reachable, and those fail silently by simply never being checked. Scanning must be scoped
to the cohort's own host, and should be confirmed against the VPS provider's
acceptable-use policy before it runs from CI — not yet checked, tracked as a live
follow-up rather than assumed clear. The smoke test's own output describes the host's
exposure and must not be published anywhere the public repository would carry it, per
`launchpad/AGENTS.md` section 8.

## Provenance

Decided directly in conversation with the repository owner (@serina-mcfall) on
2026-08-15, following the recommendation and contingency plan both posted as comments on
#47 — the same pattern used for [ADR-0008](./ADR-0008-security-audit-privilege.md) and
[ADR-0009](./ADR-0009-upstream-intel-phase-1-scope.md). `issue` and `decided_in` both
point to #47 because the decision and its filing issue are the same place.

Not verified independently in this document: actual port reachability on a deployed host
(nothing has been scanned — no host is deployed yet), and the VPS provider's
acceptable-use policy on port scanning from CI.
