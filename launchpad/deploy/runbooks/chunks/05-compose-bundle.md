# Chunk 05 — compose bundle

**What it does**

Resolves which `ghcr.io/block/buzz` image matches the deployment files in this checkout, then copies
those seven files to `/opt/buzz/compose/` on the target as root and writes an `UPSTREAM_COMMIT` note
recording the source repo, the upstream sync point, the resolved image and how it got there. Nothing
is built or started; the target receives about 32 KB of YAML and never needs a compiler.

**SOP steps covered:** 6 (6.1, 6.2, 6.3, 6.4) — rationale lives there, not here. Digest pinning is
`hardening-spec.md` §B12.

**Preconditions**

- Chunks 01–04 have run: the VM exists, `ssh -p 2222 dev@127.0.0.1 true` succeeds, and the
  `docker-clean` snapshot (SOP Step 5.3) exists as the rollback point.
- Docker is not actually needed to copy files — it is needed by chunk 07's bring-up.
- The control node is a fork checkout with `deploy/compose/` **unmodified**, and
  `launchpad/deploy/scripts/resolve-image-tag.sh` can run: `git`, `curl`, `python3`, and network
  access to `ghcr.io`.
- No `.env` exists yet, and this chunk does not create one — chunk 06 does.

**Run**

```bash
cd /Users/jeff/group-build-project/buzz

# 1. Resolve the pin. Progress notes go to stderr (watch them); the values are on stdout.
OUT=$(launchpad/deploy/scripts/resolve-image-tag.sh "$PWD" HEAD)

TAG=$(printf '%s\n'   "$OUT" | sed -n 's/^BUZZ_IMAGE=//p')
DIGEST=$(printf '%s\n' "$OUT" | sed -n 's/^# digest:[[:space:]]*//p')
SYNC=$(printf '%s\n'  "$OUT" | sed -n 's/^# sync point:[[:space:]]*//p')

# Immutable digest form (hardening-spec §B12). ${TAG%:*} drops ":sha-<7>".
IMAGE="${TAG%:*}@${DIGEST}"
printf 'image: %s\ntag:   %s\nsync:  %s\n' "$IMAGE" "$TAG" "$SYNC"

# 2. Deploy the bundle (same shell — the three variables must still be set).
cd launchpad/deploy/ansible
ansible-playbook -i inventory/hosts.yml playbooks/05-bundle.yml \
  -e "buzz_image=$IMAGE" \
  -e "buzz_image_tag=$TAG" \
  -e "buzz_sync_commit=$SYNC"
```

Keep `$IMAGE` — chunk 06 needs the same value to write `BUZZ_IMAGE` into `.env`.

**Verify**

```bash
# Eight files (seven upstream + UPSTREAM_COMMIT), all root:root, run.sh executable, ~32K total.
ssh -p 2222 dev@127.0.0.1 'ls -la /opt/buzz/compose/ && du -sh /opt/buzz && cat /opt/buzz/compose/UPSTREAM_COMMIT'

# Expected, allowing for column widths:
#   -rw-r--r-- 1 root root  ... .env.example
#   -rw-r--r-- 1 root root  ... Caddyfile
#   -rw-r--r-- 1 root root  ... README.md
#   -rw-r--r-- 1 root root  ... UPSTREAM_COMMIT
#   -rw-r--r-- 1 root root  ... compose.caddy.yml
#   -rw-r--r-- 1 root root  ... compose.dev.yml
#   -rw-r--r-- 1 root root  ... compose.yml
#   -rwxr-xr-x 1 root root  ... run.sh
#   36K   /opt/buzz
#   source: https://github.com/launchpad-26/buzz
#   upstream sync point: <commit>
#   image: ghcr.io/block/buzz@sha256:<64 hex>
#   image tag: ghcr.io/block/buzz:sha-<7>
#   delivered: ansible role compose_bundle, ...

# hardening-spec Part D check 12 — the pin is a digest, not a mutable tag.
ssh -p 2222 dev@127.0.0.1 'grep -c "^image: .*@sha256:" /opt/buzz/compose/UPSTREAM_COMMIT'
# Expected: 1

# Convergence gate (ansible/README.md "Convergence", Ruling 11) — rerun the exact
# same command from Run step 2 and read the recap.
ansible-playbook -i inventory/hosts.yml playbooks/05-bundle.yml \
  -e "buzz_image=$IMAGE" -e "buzz_image_tag=$TAG" -e "buzz_sync_commit=$SYNC" | tail -3
# Expected: ok=<n>  changed=0  unreachable=0  failed=0
```

**Rollback**

Before chunk 06 nothing here is stateful — no containers, no volumes, no secrets — so deleting the
directory is the whole rollback:

```bash
cd /Users/jeff/group-build-project/buzz/launchpad/deploy/ansible
ansible -i inventory/hosts.yml buzz_relay -b \
  -m ansible.builtin.file -a 'path=/opt/buzz state=absent'
```

**If chunk 06 or later has already run, do not use that command** — it also deletes `.env`, and the
relay's private key inside it is unrecoverable and unrotatable (`hardening-spec.md` §B8). Copy
`.env` aside first, or restore the snapshot instead.

If the target is in a worse state than that, restore the snapshot from SOP Step 5.3 (host-side
shell, not Ansible):

```bash
VBoxManage controlvm buzz-dev acpipowerbutton
until VBoxManage showvminfo buzz-dev --machinereadable | grep -q '^VMState="poweroff"'; do sleep 3; done
VBoxManage snapshot buzz-dev restore docker-clean
VBoxManage startvm buzz-dev --type headless
```

**Traps**

- `resolve-image-tag.sh` exits 1 when `deploy/compose/` differs from the sync point — that is the
  honest answer; revert the local edit (AGENTS.md rule 1) rather than hand-passing a pin.
- Because the script only accepts a candidate whose bundle is identical to the sync point's, SOP
  Step 6.1's "extract the files from *that* commit with `git archive`" case cannot arise here —
  a successful resolve means the checkout's bundle already is the image's bundle.
- `$DIGEST` is scraped from the script's `# digest:` comment; the clean fix is a real
  `BUZZ_IMAGE_DIGEST=` line in that heredoc, which `hardening-spec.md` Part F item 4 already plans.
  If the comment format changes, `$IMAGE` ends in a bare `@` and the role's shape assert fails
  rather than deploying garbage.
- `${TAG%:*}` is safe only because `ghcr.io` publishes on the default port; a registry host written
  `host:5000/repo:tag` would lose the port (§B12 pins `ghcr.io` only).
- Ownership is set explicitly to `root:root` on every file: SOP Step 6.2's `scp` carried the
  operator's host UID into the VM, which is why that step needs a `chown` and this role does not.
- `run.sh` must keep its `x` bit or chunk 07's bring-up cannot execute it (SOP Step 6.4).
- Never clone the fork into the VM (SOP Step 6 preamble): ~460 MB of source and ~30 Rust crates on
  1 vCPU / 2 GB will thrash or OOM.
- `UPSTREAM_COMMIT` deliberately carries no timestamp — the script's `# Resolved <date>` line would
  make every rerun report a changed task and break the convergence gate above.
- Re-running with a *different* `-e buzz_image` only rewrites `UPSTREAM_COMMIT`; the image the
  stack actually uses comes from `.env` (chunk 06), and `compose.yml`'s
  `${BUZZ_IMAGE:-ghcr.io/block/buzz:main}` default silently wins if that is missed (SOP 6.1,
  "`main` moves").
- Dev only warns about a tag pin; the production inventory must set
  `buzz_image_require_digest: true` so §B12 is enforced rather than remembered.
