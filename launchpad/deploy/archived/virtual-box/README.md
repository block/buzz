# virtual-box/ — the mirror layer

Reproduces the *starting state* the VPS provider hands you, and nothing more. Dev-only.

| File | Does |
|---|---|
| `build-vps-clone.sh` | Destroys and rebuilds `vps-clone-noble` from a noble cloud OVA |
| `resize-vps-clone.sh` | Grows disk, RAM or vCPU on the built VM (VDI cannot shrink) |
| `seed.sample/` | **Stale build output, not an input** — see below |

## What belongs here and what does not

Belongs: vCPU, RAM, disk, NAT port forwards, hostname, root SSH access, swap, the fallback user.

Does not: Docker, the Buzz stack, hardening. Those are `../ansible/`, so one definition serves
both this VM and the VPS. Adding them here breaks parity with the VPS's real starting state and
silently invalidates #18's capacity measurements.

## Dev-only, and deliberately insecure

cloud-init here sets `PermitRootLogin yes`, `PasswordAuthentication yes` and a known password on
both `root` and `jeff`. Acceptable because SSH is bound to `127.0.0.1:2222` and unreachable from
the LAN. **Never apply this pattern to the VPS.**

## Known problems, fix before committing

1. **`PWHASH` is hardcoded** (~line 20). #17's DoD forbids committed credentials. Should be read
   at run time or generated per build.
2. **`OVA` points at a previous session's temp scratchpad.** The file is still there today, but
   it is a temp directory subject to cleanup — a rebuild will fail confusingly once it goes.
   Move the OVA somewhere permanent and repoint.
3. **`seed.sample/` is not used by anything.** The script generates cloud-init into
   `$SCRATCH/seed` from an inline heredoc (~line 60). Editing `seed.sample/` changes nothing;
   edit the heredoc. The sample also contains a real SSH public key, so it is gitignored.

## The heredoc trap

The cloud-init block opens as unquoted `<<EOF`, so bash expands `$...` on the **Mac** before
cloud-init sees it. That is intentional for `${PUBKEY}` and `${SWAP_BYTES}`. It means anything
using shell substitution — `$(dpkg --print-architecture)`, `$VERSION_CODENAME`, cloud-init's own
`$KEY_FILE` placeholder — silently evaluates to empty and writes a broken guest config. Escape as
`\$` or hardcode. This is the main reason Docker installation does not live here.

## Port forwards

`ssh` (2222→22) exists. The relay needs 3000; add alongside it in the script rather than by hand
so a rebuild does not lose it:

```bash
VBoxManage modifyvm vps-clone-noble --natpf1 "buzz,tcp,127.0.0.1,3000,,3000"
```

## Snapshot discipline

#18 measures a genuine first `docker pull`, first migration and first relay start, so it needs a
VM that has Docker but has pulled nothing. Snapshot at exactly that point, before #19 dirties the
guest, or you are re-running a full provision between the two tasks:

```bash
VBoxManage snapshot vps-clone-noble take docker-clean --description "Docker installed, nothing pulled"
```
