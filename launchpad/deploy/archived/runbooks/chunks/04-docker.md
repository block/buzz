# Chunk 04 — Docker

**What it does**

Installs `docker.io`, `docker-compose-v2` and `containerd` on the target from the Ubuntu 24.04
archive, and enables the `containerd` and `docker` services. Asserts Compose is at least 2.24.4 and
fails the play if it is not, records the installed versions as facts, and reports the memory the
Docker daemon costs. Adds the `dev` account to the root-equivalent `docker` group.

**SOP steps covered:** 5, 5.1, 5.2 (5.3's snapshot is host-side — see Rollback) — rationale lives
there, not here.

**Preconditions**

- The `buzz-dev` VM exists and is running, cloud-init has finished, and the earlier chunks' checks
  passed. Confirm with `ansible dev-vm -m ansible.builtin.ping` returning `SUCCESS`.
- `ansible-core` 2.21 on the control node, run from `launchpad/deploy/ansible/` so `ansible.cfg` is
  picked up (it supplies the inventory, `roles_path` and the dev VM's SSH options).
- Nothing else needs to have run. This chunk installs no Buzz files and pulls no images.

**Run**

```bash
cd /Users/jeff/group-build-project/buzz/launchpad/deploy/ansible
ansible-playbook playbooks/04-docker.yml --limit dev-vm
```

**Verify**

```bash
cd /Users/jeff/group-build-project/buzz/launchpad/deploy/ansible

# 1. Convergence: the second run must report no changes at all.
ansible-playbook playbooks/04-docker.yml --limit dev-vm | tail -n 4

# 2. Versions and service state, read from the target (SOP Step 5.1).
ansible dev-vm -m ansible.builtin.shell \
  -a 'docker --version; docker compose version; systemctl is-active containerd docker'

# 3. Group membership (root-equivalent — see Traps).
ansible dev-vm -m ansible.builtin.command -a 'id -nG dev'
```

Expected output — (1) ends with `changed=0` and `failed=0`:

```
dev-vm : ok=16   changed=0    unreachable=0    failed=0    skipped=0    rescued=0    ignored=0
```

(2) prints, roughly:

```
Docker version 29.1.3, build 29.1.3-0ubuntu3~24.04.2
Docker Compose version 2.40.3+ds1-0ubuntu1~24.04.1
active
active
```

(3) includes `docker` in the group list, for example
`dev adm dialout cdrom floppy sudo audio dip video plugdev users docker`.

The first run's own output also prints the versions and
`memory used: 296 MB -> 449 MB (delta 153 MB)` or thereabouts.

**Rollback**

Nothing here is destructive and re-running is safe, so the first move is to re-run. If the target
itself is wrong, restore a snapshot — `VBoxManage snapshot buzz-dev list` shows what exists; there is
no pre-Docker snapshot in the SOP, so the fallback is a rebuild with
`virtual-box/build-vps-clone.sh` and the earlier chunks again. Do not try to unwind with
`apt-get purge docker.io`: it leaves `/var/lib/docker` and the `docker` group behind, which is a
different state from "never installed".

Once the Verify block passes, take SOP Step 5.3's restore point — this is the state you want back
when measuring what the Buzz stack really consumes, and recreating it by hand means repeating
Steps 1-5. On your own computer:

```bash
VBoxManage controlvm buzz-dev acpipowerbutton
until VBoxManage showvminfo buzz-dev --machinereadable | grep -q '^VMState="poweroff"'; do sleep 3; done
VBoxManage snapshot buzz-dev take docker-clean --description "Docker installed, no images pulled"
VBoxManage startvm buzz-dev --type headless
```

Powering off first keeps the snapshot small; snapshotting a running VM also captures its RAM.

**Traps**

- A Compose below 2.24.4 does not reject `ports: !reset []`, it ignores it — the relay comes back
  published on `0.0.0.0:3000` in plaintext past `ufw` while the stack reports healthy
  (hardening-spec.md §B2). The play's assert is the only gate; never bypass it.
- Do not "fix" a version failure by adding Docker's own apt repo — that is a second GPG key and a
  second `Unattended-Upgrade::Allowed-Origins` entry on a host about to be hardened, and Docker
  silently stops being patched if you forget the latter (hardening-spec.md lines 34-43).
- Only the Compose *plugin* is installed: `docker compose version` works, `docker-compose` is
  command-not-found. That is correct, not a partial install (SOP Step 5.1).
- The `docker` group is root-equivalent — members can read every secret out of `docker inspect` and
  mount the host filesystem (hardening-spec.md §B7). Membership buys typing convenience, not reduced
  privilege.
- The group change needs a fresh login session to take effect, so it does nothing for Ansible's
  current connection; later chunks still use `become` and must keep doing so, because on the VPS
  nobody is in that group (hardening-spec.md §B7).
- The ~125 MB daemon overhead is a first-install number. On a rerun the delta reads ~0 because
  Docker was already resident when the "before" sample was taken (SOP Step 5.2).
- On a VM that booted seconds ago, cloud-init still holds the dpkg lock; the role waits 300s. If it
  still fails, run `ansible dev-vm -b -m command -a 'cloud-init status --wait'` first.
- `--check` against a never-provisioned host fails at the version reads — `docker --version` is a
  real command against a host where Docker does not exist yet. Check mode is meaningful only after a
  successful first run.
- This chunk writes no `/etc/docker/daemon.json`. `no-new-privileges`, `icc: false` and the
  `buzz-net` split are hardening-spec.md §C10 and §B5, applied by the hardening chunk with a
  `dockerd --validate` gate — deliberately not here.
- Version numbers belong on the issue, not in the repo (AGENTS.md rule 4: #18 and #20 produce
  evidence, not repository files).
