---
status: Proposed
date: 2026-08-15
issue: launchpad-26/buzz#44
decided_in: launchpad-26/buzz#44
supersedes: none
---

# ADR-0014 — Host firewall implementation for the deny-by-default policy

## Decision

**`ufw` with a deny-by-default inbound policy, plus explicit rules in the `DOCKER-USER`
chain** so container-published traffic is filtered on the path Docker actually uses.
Not-publishing — Caddy's `ports: !reset []`, and binding any unavoidable local-only port
to `127.0.0.1` — is retained as a **complement** to this policy, not a substitute for it.

`ufw` stays readable to a cohort that has never configured a firewall before, satisfying
the "an unreadable ruleset is a risk of its own" driver, and it carries no persistent
daemon of its own — it is a thin front end over `iptables`, appropriate for the 1 vCPU /
1.9 GiB VPS #24 already measured. `DOCKER-USER` is Docker's own documented hook for
rules that must survive Docker's own chain manipulation, which is exactly the property
this decision needs: Docker DNATs and forwards published container ports past the
host's `INPUT` chain, where a naively configured `ufw` policy filters — so a rule placed
anywhere else risks being silently bypassed or reordered by Docker itself.

## Context

Ruling 5 (#5) makes network access deny-by-default and exposes only what the public
Buzz service needs; Ruling 11 puts firewall rules inside the converged desired state.
Reading `deploy/compose/`: the base `compose.yml` publishes only the relay
(`"${BUZZ_HTTP_PORT:-3000}:3000"`) and gives PostgreSQL, Redis, and MinIO no published
ports at all — they are reachable only over the `buzz-net` bridge. `compose.caddy.yml`
removes even the relay's own port (`ports: !reset []`) and publishes `80`/`443` through
Caddy instead. But `compose.dev.yml` publishes `5432`, `6379`, `9000`, `9001`, Adminer on
`8082`, and Prometheus on `9090`, and `run.sh` pulls that whole file in from one
environment variable (`BUZZ_COMPOSE_DEV=true`).

The sharper problem is that a published Docker port is DNAT'd and traverses the
forwarding path, not the host's `INPUT` path, where a default `ufw` policy does its
filtering. A naively configured firewall — `default deny incoming` plus `allow 80,443`
— reports success while `BUZZ_COMPOSE_DEV=true ./run.sh start` quietly puts Postgres and
Adminer on the public internet. #30 implements this decision; its own footer left the
firewall choice deliberately unraised as an ADR and invited one if the cohort disagreed.
This ADR takes that invitation, so the reasoning is argued once here rather than buried
in a task's definition of done.

MinIO's credentials are the relay's own S3 credentials
(`MINIO_ROOT_USER: ${BUZZ_S3_ACCESS_KEY}`), so an exposed console on `9001` is not a
peripheral leak — it is a path to the relay's own object-storage credentials.

## Consequences

**Good.** Naming the implementation lets #30 be written as concrete, reviewable rules
instead of "a firewall," and gives #35 a specific thing to assert from off-host. It also
forces the Docker interaction to be answered once, in the open, rather than
rediscovered by whoever first notices an open port.

**Bad, stated honestly.** Every option that actually constrains Docker traffic is less
familiar than the naive one, so the cohort takes on a ruleset that is harder to read and
easier to get subtly wrong than a bare `ufw allow` list. Filtering in `DOCKER-USER` also
means Docker and the firewall share responsibility for the same packets, and a future
Docker upgrade can change that interaction.

**Contingency — Docker's own chain handling changes underneath this control.**

*Trigger:* any Docker Engine upgrade that changes how Docker manages its
`iptables`/`nftables` chains — including Docker's own documented shift toward an
`nftables`-native firewalling backend on newer releases — or any observed case where
`DOCKER-USER` rules stop intercepting container-forwarded traffic as expected.

*The fix:* re-verify, not assume, that `DOCKER-USER` rules still intercept forwarded
traffic after the upgrade. Docker and the firewall share responsibility for the same
packets by design here, so a chain-layout change on Docker's side means the automation's
`DOCKER-USER` rules need updating to match Docker's new hook points, not a
re-litigation of the `ufw`-vs-alternative choice.

*The safety net in the meantime:* #35's external port scan is exactly the check that
catches this regression from off-host — this is why the risk this decision accepts is
that #35 becomes load-bearing rather than decorative. #35 should run after every Docker
version bump on the host, not only at initial deployment, since ADR-0013 already ties
Docker's patch stream to Ubuntu's `-security` releases, which land without anyone
deciding to touch the firewall.

## Security implications

The failure mode that matters here is the silent one: a firewall that reports a
deny-by-default policy while container-destined traffic never passes through the chain
it filters. #5's success criteria state the test directly — an external port scan must
reach only the intentionally public services, with PostgreSQL, Redis, and
object-storage administration unreachable. Because the bundle ships a dev override that
publishes exactly those services, the policy must hold against a misconfigured Compose
invocation, not merely against the intended one. `ufw` and `DOCKER-USER` rules are both
expressible as idempotent Ansible tasks (`community.general.ufw` for the host policy, a
templated ruleset or `ansible.builtin.iptables` block for `DOCKER-USER`), satisfying
Ruling 11's convergence requirement and Ruling 1's requirement that the control live in
version-controlled automation rather than a provider's web console.

## Provenance

Decided directly in conversation with the repository owner (@serina-mcfall) on
2026-08-15, following the recommendation posted as a comment on #44 (2026-08-15) and the
contingency plan posted as a follow-up comment on #44 (2026-08-15) — the same pattern
used for [ADR-0008](./ADR-0008-security-audit-privilege.md),
[ADR-0009](./ADR-0009-upstream-intel-phase-1-scope.md),
[ADR-0011](./ADR-0011-external-security-smoke-test-floor.md),
[ADR-0012](./ADR-0012-inference-provider-boundary.md), and
[ADR-0013](./ADR-0013-config-management-ubuntu-baseline-runtime-shape.md). `issue` and
`decided_in` both point to #44 because the decision and its filing issue are the same
place.

Not verified independently in this document: the running behaviour of any firewall on
the cohort VPS or the destroyable VM, the installed Docker version's actual chain
layout, and whether any firewall is configured on the host today — nothing was executed
against a host in this session.
