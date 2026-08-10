# ansible/ — the configure layer

Every transformation applied to a host after the provider hands it over. **The only dual-target
thing in this folder** — the same plays run against the dev VM and the cohort VPS, which is the
entire reason the VM's starting state is kept identical to the VPS's.

Pending ratification of **ADR #24**, which bundles three questions: config-management tool,
Ubuntu baseline, and runtime shape. Proceeding here answers all three de facto (Ansible, noble,
containers via `deploy/compose/`) — every one of which is that ADR's *expected* option, but
`launchpad/AGENTS.md` rule 1 is "draft everything, approve nothing," so the cohort still ratifies.

## Scope right now

Roles **to be written**: `docker`, `compose-bundle`. Neither exists yet — the manual pass is
being done first, by hand on the dev VM, so the roles encode something already observed to work
rather than a guess.

The manual pass is now complete and written up as
[`../runbooks/dev-deployment-SOP.md`](../runbooks/dev-deployment-SOP.md). **That document is the
specification these roles implement** — if a role does something the SOP does not describe, one of
the two is wrong, and the SOP changes first.

Three things it settled that the roles must reproduce:

- **A cohort-owned compose override is unavoidable** for the relay-served front-ends, because
  `deploy/compose/compose.yml` mounts neither `web/dist` nor `admin-web/dist` and must not be
  edited. It is named `compose.cohort.yml`, deliberately **not** `compose.override.yml` — Docker
  auto-loads the latter for bare `docker compose` commands but `run.sh` passes explicit `-f` and
  would ignore it, giving two different stacks depending on the command used.
- **`run.sh` cannot bring up that stack.** Its `-f` list is hardcoded with no extension hook, so the
  role invokes `docker compose --env-file .env -f compose.yml -f compose.cohort.yml up -d --wait`
  and must replicate `require_env`'s no-`CHANGE_ME` check itself, since it loses it.
- **Env-var ordering is load-bearing.** `BUZZ_ADMIN_WEB_DIR` is validated at relay startup against
  `index.html` *inside the container*, so mounts must exist before the variable is set or the relay
  crash-loops on a config error. Also `BUZZ_SERVE_GIT_WEB_GUI=true` is what makes the web bundle
  serve at `/`; without it the bundle only answers `/invite/<code>`.

Decided during that pass: **Docker comes from the Ubuntu 24.04 archive**
(`docker.io`, `docker-compose-v2`, `containerd`), not Docker's official apt repo. Measured on the
VM, the archive ships Compose **2.40.3**, well clear of the 2.24.4 floor the `!reset` tag needs, so
the version argument for a third-party repo does not hold. Keeping to one trusted repo means
Docker security updates ride Ubuntu's stream and #32's update policy covers them without a
third-party GPG key on a host that is about to be hardened.

Roles that deliberately do **not** exist yet — firewall, SSH policy, AppArmor,
unattended-upgrades, service identities. Those are #29–#34 under PRD #5. Writing them now puts
#5's work in front of #2's deadline, which is the mistake #39-gating-#21 already made once.

## Design rules

1. **Call upstream `run.sh`, do not reimplement it.** `deploy/compose/run.sh` already does
   `require_env` validation and `up -d --wait`. Wrapping it keeps `deploy/compose/` upstream-owned
   and conflict-free; `community.docker.docker_compose_v2` would duplicate its logic and drift.
2. **Generate `.env` secrets on the target when absent.** Never template them from control-node
   vars, never commit them. Satisfies #22's "generated on the host and appear in no tracked file"
   without pre-empting ADR #25's secret-storage decision.
3. **Install Docker from Docker's own apt repo**, not `docker.io`/`docker-compose-v2`. Compose
   must be ≥ 2.24.4 for the `!reset` tag in `compose.caddy.yml`; Ubuntu 24.04's archive version
   sits close to that line and varies with updates. The point is a known, current version.
4. **`RELAY_URL` is the variable that matters.** It alone decides which community the relay seeds
   and therefore which `Host` headers it accepts — there is no seeding command. See
   `../runbooks/relay-build-list.md` before writing the `compose-bundle` role.

## Inventory

`inventory/hosts.yml` carries the dev VM only. The VPS is a **private hostname**, which
`AGENTS.md` rule 2 forbids committing — put it in `inventory/hosts.local.yml`, which is
gitignored:

```yaml
all:
  children:
    buzz_relay:
      hosts:
        vps:
          ansible_host: <cohort VPS address>
          ansible_user: root
```

Run against one target at a time and rehearse on the VM first:

```bash
ansible-playbook -i inventory/hosts.yml playbooks/site.yml --limit dev-vm
```

## Why the VM matters more than convenience

Two plays under PRD #5 can lock you out of a host permanently: #29 restricts remote root access,
#30 applies a deny-by-default firewall. Because the VM's starting state matches the VPS's, both
can be run destructively here first. That is worth more than the automation itself.

## Convergence

Ruling 11 requires reruns to converge, and #36 requires rebuilding a destroyed host from the
repository. Every role must be genuinely idempotent — a second run should report zero changes.
Verify that on the VM as a matter of course, not as a separate task.
