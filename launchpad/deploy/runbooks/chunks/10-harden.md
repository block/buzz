# Chunk 10 — harden

**What it does.** Applies the dev-VM hardening subset: SSH policy, an ingress firewall, kernel
parameters, automatic security updates, and the Docker daemon — then re-proves the relay still
answers. Roles run in a deliberate order: **access before firewall**, Docker daemon last because it
recreates containers.

**SOP steps covered:** 13 (13.2–13.8). All rationale, and the full production baseline this is a
subset of, live in [`../hardening-spec.md`](../hardening-spec.md).

> **Scope, stated plainly.** This is the **dev-VM subset**, not production parity. It does **not**
> implement `DOCKER-USER` rules, default-deny **egress**, split internal/external container networks
> (§B5), module blacklisting, systemd resource limits (§C6), `auditd` or sudo I/O logging (§C8),
> authenticated time (§C9), the MinIO service account (§B4), or Caddy rate limiting (§B9). A green run
> means "the lockout-risky plays have been rehearsed and this VM is no longer trivially insecure" —
> **not** "production-hardened".
>
> Including this chunk at all **knowingly overrides [`../../AGENTS.md`](../../AGENTS.md) rule 5**,
> which reserves these roles for issues #29–#34 under PRD #5. That was an explicit decision by the
> repo owner, recorded rather than taken silently.

## Preconditions

- Chunk 07 has run and the stack is healthy. Hardening a machine you have never seen working means
  a later failure has two possible causes instead of one (SOP Step 13's opening).
- **A snapshot exists.** Two of these plays can lock you out permanently.
  ```bash
  ./deploy snapshot buzz-working
  ```
- Ansible reaches the host as `dev` with working sudo. The playbook asserts this before touching
  sshd; if it cannot escalate as the unprivileged user it refuses to run at all.

> **On a VPS there is no snapshot.** `hardening-spec.md` §C1 requires break-glass — cloud serial
> console access plus a sealed credential held outside the repo and outside the vault the deploy uses —
> to exist **and be tested** before this runs anywhere near production. Build it at the same time as
> the hardening, not afterwards.

## Run

```bash
./deploy run 10
```

Dry run first, which changes nothing:

```bash
./deploy check 10
```

## Verify

```bash
./deploy verify
```

The hardening block stops skipping once `00-hardening.conf` exists. The two assertions that matter
most, because they are the ones a plausible-looking run gets wrong:

```bash
# Effective merged config, NOT the file — see Traps.
ssh -p 2222 dev@127.0.0.1 'sudo sshd -T | grep -E "^(permitrootlogin|passwordauthentication)"'
# expect: permitrootlogin no
#         passwordauthentication no

# The refusal IS the success condition.
ssh -p 2222 root@127.0.0.1
# expect: Permission denied (publickey)
```

Then snapshot the good state: `./deploy snapshot hardened`.

## Rollback

```bash
./deploy restore buzz-working
```

That is the only undo for the SSH and firewall plays. If you are locked out and have no snapshot, the
VirtualBox console window plus the console password from
`~/vm-images/.buzz-dev-console-password` is the remaining way in.

## Traps

- **Adding a hardening file is not enough — the parity file must be deleted.** sshd keeps the **first**
  value it reads and `/etc/ssh/sshd_config.d/` is read in name order, so `01-dev-parity.conf`
  (`PermitRootLogin yes`) wins over anything added alongside it. The role removes it by glob covering
  both `01-dev-parity.conf` and the older `01-vps-parity.conf` name, because a missed removal means
  hardening reports success and changes nothing (§C2).
- **The hardening file must sort before `50-` and `60-`, not after.** Confirmed on this image on
  2026-08-12: `/etc/ssh/sshd_config.d/` contains `01-dev-parity.conf`, `50-cloud-init.conf` **and**
  `60-cloudimg-settings.conf`, with `60-` setting `PermitRootLogin prohibit-password`. A
  `99-hardening.conf` loses to both. Hence `00-hardening.conf`.
- **Verify with `sshd -T`, never by checking the file exists.** `sshd -T` prints the merged effective
  configuration and is the only output that proves the ordering trap was avoided.
- **`ufw` does not govern Docker's published ports.** Docker writes rules into the `DOCKER` chain,
  evaluated *before* ufw's, so `ufw deny 443` would not close a published container port. On this VM it
  is harmless because VirtualBox forwards only from `127.0.0.1`; on the VPS it is not, and the real fix
  is `compose.caddy.yml`'s `ports: !reset []`, which this deployment already uses. Never read
  `ufw status` as evidence that a container port is closed (§B2).
- **`daemon.json` only supplies defaults for NEW containers.** The relay, Postgres, Redis and MinIO
  were created before it existed, and `live-restore: true` deliberately leaves them running untouched
  across a daemon restart — so without a `--force-recreate` the hardening applies to nothing that is
  actually running. The role does the recreate.
- **Do not verify `no-new-privileges` with `docker inspect .HostConfig.SecurityOpt`.** That field only
  reports flags given at container-creation time; a daemon *default* renders as an empty list even when
  fully active, which looks like failure and sends you round the recreate loop forever. Read
  `/proc/1/status` inside the container instead — it asks the kernel.
- **`net.ipv4.ip_forward=0` is deliberately NOT set.** Docker requires forwarding and turns it back on,
  so setting it only produces a permanent disagreement with reality that makes every future audit look
  like a failure (§C5).
- **Two `daemon.json` keys from popular guides are actively harmful.** `disable-legacy-registry` was
  removed from Docker years ago and now stops the daemon starting; `ip-forward-no-drop` disables Docker
  28+'s default drop of traffic to unpublished container ports. `dockerd --validate` catches the first;
  nothing catches the second. The role rejects both by name.
- **`icc: false` is not container isolation.** It governs only Docker's default bridge. Buzz runs on
  its own `buzz-net`, where containers still reach each other — which is required, since the relay must
  talk to Postgres, Redis and MinIO. Do not remove it while debugging a connectivity problem; it will
  not be the cause.
- **`unattended-upgrade` is singular**, though the package is `unattended-upgrades`. And validating it
  through a pipe (`... | tail -5`) reports the *pipe's* exit status, so a failure reads as success.
