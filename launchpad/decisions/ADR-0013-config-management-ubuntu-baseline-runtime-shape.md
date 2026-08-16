---
status: Proposed
date: 2026-08-15
issue: launchpad-26/buzz#24
decided_in: launchpad-26/buzz#24
supersedes: none
---

# ADR-0013 — Configuration-management tool, Ubuntu baseline and service runtime shape

## Decision

**Ansible** as the configuration-management tool, **Ubuntu 24.04 LTS (noble)** as the
supported starting state, and **containers via the existing `deploy/compose/` bundle**
as the runtime shape for Buzz and its dependencies.

Ansible is the PRD's own named expectation, is agentless (no persistent daemon on a
1 vCPU / 1.9 GiB host), has genuinely idempotent modules (satisfying Ruling 11's
convergence requirement, which shell scripts plus systemd cannot without effectively
reimplementing a configuration-management system), and is readable by a teaching cohort
that has not used it before. Host-managed services are rejected: replacing
`deploy/compose/` with them discards an upstream-maintained artifact and creates the
divergence-at-every-sync maintenance trap `launchpad/AGENTS.md` section 3 already exists
to avoid.

## Context

#5 requires the cohort server to be reproducible from version-controlled automation, and
Ruling 3 names the core problem as reproducibly transforming a supplied bare Ubuntu host
into the intended application and security state. Three of that PRD's open questions —
configuration-management tool, Ubuntu LTS baseline, and container-vs-host-managed
runtime shape — are bundled into one ADR because they are not independent: the runtime
shape constrains what the automation manages, and the supported baseline constrains
both.

This was also at risk of being decided de facto rather than by choice. `#22` deploys the
relay using the repository's existing `deploy/compose/` bundle, which is containerised;
had it shipped before this ADR was settled, the container question would have been
answered by precedent rather than by a recorded decision. As of this ADR, #22 has not
shipped.

**A prior attempt exists and informs this decision without being resumed.**
`launchpad/deploy/archived/ansible/README.md` records that Ansible, noble, and
containers via `deploy/compose/` were already built toward as "ADR #24's expected
option," pending this ratification. That attempt measured real numbers on a VM matching
the VPS's spec: peak memory 563 MB against 1.9 GiB with swap never touched, all-healthy
in 18 seconds, and Docker installed from Ubuntu 24.04's own archive package
(`docker.io`/`docker-compose-v2`/`containerd`) measured at Compose **2.40.3** — well
clear of the 2.24.4 floor `compose.caddy.yml`'s `!reset` tag needs. Keeping Docker on
Ubuntu's own trusted repository also means its security patches ride Ubuntu's
`-security` stream rather than needing a separate third-party GPG key and its own
unattended-upgrades allow-list entry on a host that is about to be hardened.

The entire `launchpad/deploy/` tree is nonetheless marked as a **failed deployment
method** and is not to be used to build or deploy Buzz. Reading
`launchpad/deploy/VPS-DEPLOYMENT-AUDIT.md`, the actual failure was narrow and orthogonal
to this ADR's three questions: the deployment defaulted to Block's own upstream image
(`ghcr.io/block/buzz:main`) instead of `launchpad-26/buzz`'s own, mixing
Launchpad-specific work with upstream image-selection behaviour. That problem is being
fixed separately (#144, tracked under ADR-0005's deployment boundary), not by this
decision. The three answers this ADR ratifies are the same ones the archived attempt
assumed, now backed by measurement rather than assumption — no code from the archive is
reused or resurrected; a fresh implementation still builds its roles against the
already-fixed image-selection path.

## Consequences

**Good.** Settling this unblocks nearly every implementation task under #5, all of which
are currently worded tool-agnostically and cannot be written concretely until it is
answered. It also gives #44 (host firewall) and #45 (AppArmor) — both blocked on this
ADR — a concrete confinement target: container runtime permissions, capabilities, and
networks, rather than operating-system users alone.

**Bad, stated honestly.** This is on the critical path, and the tasks under #5 cannot
start in earnest until it lands. Choosing Ansible also adds a tool the cohort may not
already know, against a milestone deadline. Choosing to keep `deploy/compose/` means the
security posture of the container runtime becomes part of the hardening surface, which
Ruling 9 must then address through container and service permissions rather than
operating-system users and AppArmor applied directly to Buzz's own processes.

**Contingency — Ansible unfamiliarity against the milestone deadline.**

*Trigger:* #5's Ansible-authoring tasks are visibly behind schedule as the milestone
approaches, or the cohort reports being genuinely blocked by unfamiliarity with Ansible.

*The fix:* the tool choice does not change. Shell scripts plus systemd were already
rejected earlier in this same decision for failing Ruling 11's convergence requirement,
and switching under deadline pressure would forfeit real idempotency for a false
schedule win. The fix is investing in reference material and pairing, not reopening the
tool decision.

*The safety net in the meantime:* `launchpad/deploy/archived/ansible/` is unusable for
execution — it is the archived, failed attempt — but is retained as a documented
reference for role structure and shape. `docker` and `compose-bundle` roles were already
sketched there, and the measured facts (Docker-from-archive ships Compose 2.40.3, the
three-file compose invocation, env-var ordering constraints between mounts and
`BUZZ_ADMIN_WEB_DIR`) can be consulted without resurrecting its code.

## Security implications

The runtime shape determines what confinement even means. Host-managed services would
be confined with operating-system users and AppArmor directly; containerised services
are confined with runtime permissions, capabilities, and networks instead. Rulings 4, 5,
and 9 have different implementations under each, so this decision is what lets the
confinement tasks under #5 state what they are actually confining. The choice of tool
also determines how secrets reach the host: the archived attempt's pattern of
generating `.env` secrets on the target when absent, rather than templating them from
control-node variables or committing them, satisfies #22's requirement that secrets be
generated on the host and appear in no tracked file, without pre-empting #25's separate
secret-storage decision. A private VPS hostname must never be committed to the
inventory — it belongs in a gitignored `inventory/hosts.local.yml`, per
`launchpad/AGENTS.md`'s public-repository rule.

## Provenance

Decided directly in conversation with the repository owner (@serina-mcfall) on
2026-08-15, following the recommendation posted as a comment on #24 (2026-08-15) and the
contingency plan posted as a follow-up comment on #24 (2026-08-15) — the same pattern
used for [ADR-0008](./ADR-0008-security-audit-privilege.md),
[ADR-0009](./ADR-0009-upstream-intel-phase-1-scope.md),
[ADR-0011](./ADR-0011-external-security-smoke-test-floor.md), and
[ADR-0012](./ADR-0012-inference-provider-boundary.md). `issue` and `decided_in` both
point to #24 because the decision and its filing issue are the same place.

Not verified independently in this document: whether the cohort has existing Ansible
experience (the original issue's own caveat, still open); the archived measurements were
read from `launchpad/deploy/archived/temp-handoff.md` and
`launchpad/deploy/archived/ansible/README.md`, not re-measured in this session.
