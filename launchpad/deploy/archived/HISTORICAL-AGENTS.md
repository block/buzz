# AGENTS.md — dev deployment environment

Lives at `launchpad/deploy/`. It was drafted in a scratch directory (`~/vps-clone`) and originally
staged for `launchpad/dev/deploy/`; the `dev/` level was dropped when it was committed. If that
split returns, note that **`ansible/` is the one thing here that is not dev-only** — it targets the
production VPS too — so it would belong outside a `dev/` folder.

## What this folder is for

A local VirtualBox VM with CPU, RAM and swap parity to the cohort's Buzz VPS, plus the
configuration management that provisions it. Its purpose is to answer "does this work, and does
it fit?" on a destroyable machine before the cohort's real server is touched.

Serves PRD #2 (#18 capacity, #19 Host/membership rehearsal) and, later, rehearsal of the
lockout-risky hardening plays under PRD #5 (#29, #30).

## The mirror / configure boundary

This is the organising principle. Do not blur it.

| Layer | Owns | Target |
|---|---|---|
| `virtual-box/` | The **starting state a provider hands you** — vCPU, RAM, disk, NAT forwards, hostname, root SSH, swap, fallback user | Dev VM only |
| `ansible/` | **Every transformation applied to that state** — Docker, the compose bundle, `.env`, later all hardening | Dev VM **and** the VPS |

Consequences:

- **Docker is never installed by cloud-init.** It is an Ansible role, so one definition serves
  both targets.
- The VM is only useful if its starting state matches the VPS's. Anything added to cloud-init
  that the provider would not have given you breaks that, and silently invalidates #18's
  measurements.

### The asymmetry that can hurt you

`ansible/` is the only thing here that is **not dev-only**. Its plays are meant to run against the
production VPS.

`virtual-box/` cloud-init is dev-only and deliberately insecure — `PermitRootLogin yes`,
`PasswordAuthentication yes`, and a known password on both `root` and `jeff`. That is acceptable
for a VM whose SSH is bound to `127.0.0.1`. **It must never be applied to the VPS.** Never
generalise a cloud-init pattern from here into an Ansible role without stripping the dev-only
credential handling.

When this folder migrates, `ansible/` most likely belongs at `launchpad/deploy/ansible/` with
`dev/deploy/` keeping only an inventory entry.

## Hard rules

1. **Never edit `deploy/compose/`.** It is upstream-owned by `block/buzz`; every local change
   becomes a merge conflict on each sync. Consume it — call its `run.sh` from Ansible rather than
   reimplementing bring-up with `community.docker.docker_compose_v2`. Cohort-owned overrides
   (for example a `tls internal` Caddyfile) live here, not there.
2. **No secrets, keys or private hostnames committed.** Includes SSH public keys — read the
   operator's at run time. See `.gitignore`, and the outstanding violations below.
3. **Generate secrets on the target.** When `.env` is absent, create it there. This satisfies
   #22's "generated on the host and appear in no tracked file" without pre-empting ADR #25's
   secret-storage decision.
4. **Measurement evidence goes on the issue, not in the repo.** #18 and #20 both state "no
   repository files change — this issue produces evidence, not code." Do not add a
   `measurements/` directory.
5. **Scope discipline.** Ansible roles that exist now: `docker`, `compose-bundle`. Roles that do
   **not** exist yet: firewall, SSH policy, AppArmor, unattended-upgrades, service identities.
   Those are #29–#34 under PRD #5; writing them now puts #5's work ahead of #2's deadline.

## The one operational fact most likely to waste your day

`RELAY_URL` in `deploy/compose/.env` is the single control over which community exists and
therefore which `Host` headers the relay accepts. There is **no** command to seed the
`communities` table — the relay does it at startup from this variable. Get it wrong and every
connection is rejected while the stack looks healthy.

Full mechanism, normalization rules and the port-handling trap: `runbooks/relay-build-list.md`.

## Layout

```
AGENTS.md              this file
CLAUDE.md              pointer to this file
README.md              operator-facing: access, parity table, rebuild
.gitignore
virtual-box/           dev VM only — build/resize scripts, cloud-init
ansible/               dual-target — inventory, roles, playbooks
scripts/               target-agnostic helpers, runnable on VM or VPS
docker/                containerised supporting tools (see its README)
runbooks/              committed procedures — #19 and #38 require these
```

## Outstanding issues in this folder

Known, not yet fixed:

- **`virtual-box/build-vps-clone.sh` hardcodes a password hash** (`PWHASH`, line ~20) and
  `OVA` points at a **previous session's temp scratchpad**, which will be cleaned up and break
  rebuilds. Both must be fixed before this is committed — #17's DoD forbids the former.
- **`virtual-box/seed.sample/` is a build output, not an input.** The build script generates
  cloud-init into `$SCRATCH/seed` from an inline heredoc and never reads this directory. The
  README's claim that you edit these files to change provisioning is wrong — edit the heredoc.
  The sample copy also contains a real SSH public key.
- **`bodies/`, `prd0-breakdown.md`, `prd3-breakdown.md` are issue-drafting artifacts**, not
  deployment code. They should not migrate into `launchpad/dev/deploy/`.
- **#17 and #19 need rewording** before anyone works them — #17's scope shrinks under the
  mirror/configure split, and #19 names a community-seeding command that does not exist.
