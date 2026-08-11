# Chunk 00 -- PREFLIGHT

**What it does**

Runs every host-side check that must pass before the VM is built, and refuses to let you start if
one fails. It is read-only: it inspects the host and changes nothing, so it is safe to re-run at any
point. Each failure prints a remedy and the SOP step it came from.

**SOP steps covered:** 0.2, 0.3, 0.4, 1, 3.2 (plus a warning-only look ahead at Step 10) -- rationale
lives there, not here

**Preconditions**

- None. This is the first chunk; no other chunk has to have run.
- VirtualBox installed (SOP Step 0.4), `noble.ova` downloaded (SOP Step 1) and an ed25519 SSH key
  present (SOP Step 3.2). Preflight **checks** these, it does not create them -- the remedies tell
  you the command to run.
- Ansible installed on this machine (the control node). Everything that runs on the target is
  Ansible's from chunk 02 onward.
- macOS on Intel. The chunked path is Mac-only because `build-vps-clone.sh` builds the cloud-init
  seed with `hdiutil`.

**Run**

```bash
cd ~/code/buzz/launchpad/deploy/virtual-box
./preflight.sh
```

Optional overrides, matching `build-vps-clone.sh`: `OVA=/path/to/noble.ova`, `VM=buzz-dev`.

**Verify**

```bash
cd ~/code/buzz/launchpad/deploy/virtual-box
./preflight.sh; echo "exit=$?"
```

Expected tail:

```
--- PREFLIGHT SUMMARY ---

  RESULT  CHECK                             DETAIL
  ------  --------------------------------  ------
  PASS    host platform                     Darwin (24.6.0)
  PASS    processor architecture            x86_64
  PASS    free disk space                   61.4 GiB free on /System/Volumes/Data (need 15 GiB)
  PASS    VBoxManage                        version 7.1.4r165100 at /usr/local/bin/VBoxManage
  PASS    SSH public key                    /Users/you/.ssh/id_ed25519.pub present (99 bytes, ssh-ed25519)
  PASS    noble.ova                         /Users/you/vm-images/noble.ova (570 MB)
  PASS    ansible                           ansible [core 2.21.0]
  PASS    host port 2222 free               nothing listening on 127.0.0.1:2222
  PASS    host port 8080 free               nothing listening on 127.0.0.1:8080
  PASS    host port 8443 free               nothing listening on 127.0.0.1:8443
  WARN    buzz-vm.test resolves             does not resolve yet
  WARN    admin.buzz-vm.test resolves       does not resolve yet

  10 passed, 2 warning(s), 0 failed

PREFLIGHT PASSED. Next: chunk 01 (build the VM).
exit=0
```

Exact versions and sizes will differ. What must hold: `0 failed` and `exit=0`. The two `WARN` rows
become `PASS` after chunk 03 and are informational until then.

**Rollback**

Nothing to roll back -- no file is written and no host state is changed. On failure, apply the
remedies printed above the summary and run it again. Do not proceed to chunk 01 with a failure
outstanding.

**Traps**

- Never pipe a producer into `head` in these scripts: `set -o pipefail` turns the resulting SIGPIPE into exit 141 before anything is logged, which is exactly how `build-vps-clone.sh` died (see its comments at lines 47-51 and 76).
- A running `buzz-dev` holds host port 2222, so preflight fails -- correctly: chunk 01 opens with `unregistervm --delete`, which errors on a running VM. `VBoxManage controlvm buzz-dev poweroff` first (SOP Step 2.7).
- `lsof` without `sudo` only sees your own processes, so a port held by another user's daemon reads as free (SOP Step 2.7).
- macOS `/bin/bash` is 3.2 from 2007; the script is written to that dialect, so do not add bash 4+ syntax when editing it (SOP Step 0.1 makes the same point about `bash --version`).
- Ports 80 and 443 are deliberately not checked -- nothing is ever bound to them on the host, which is why the forwards are 8080 and 8443 (hardening-spec.md sec-A.3, sec-B2).
- The hosts-file checks are warnings, not failures, because chunk 03 is what adds those entries (SOP Step 10).
- `~/.ssh/id_ed25519.pub` is not overridable here because `build-vps-clone.sh` hardcodes that path; an override would pass preflight and then fail the build (SOP Step 3.2).
