# Dev deployment SOP — from nothing to a working Buzz environment

**Audience:** you have a **Mac or a Windows PC**, a terminal, and no particular experience with
virtual machines, Bash, or networking. Every command is written out. Nothing is hidden in a script.

**What you will have at the end:** a virtual machine on your own computer running the Buzz relay,
Postgres, Redis and MinIO, reachable from your browser; the admin dashboard; the web bundle served
by the relay; the desktop app connected to it; the machine **hardened** — the dev-VM subset of the
production baseline, with the scope spelled out in Step 13; and an **AI agent you can @mention in a
channel and get a reply from**, backed by your own OpenRouter key.

The last two are what make this more than a demo. Steps 1–12 give you a working Buzz. Step 13
applies the security configuration, and Steps 14–16 put an agent in it. Nothing is considered
finished until the checklist in Step 17 passes.

**On Mac versus Windows.** Almost every command here runs *inside the virtual machine*, which is
Ubuntu whichever computer you start from — so the bulk of this document is identical either way.
Only the steps acting on your own machine differ, and they are marked **On macOS** / **On Windows**
where they do. Step 0.1 gets both platforms onto the same shell so even most of those match.

(Linux hosts are not covered. Everything would work with minor substitutions, but nobody has run it,
so it is not written down here.)

**Why this document exists twice over.** It is the way to build the environment by hand, *and* it
is the specification the automation must follow. If an Ansible role does something this document
does not describe, one of the two is wrong. Keep them in step.

---

## AMENDMENT 2026-08-12 — this document now specifies the TLS path

**Read this before Step 7, Step 9, Step 10 or Step 12.** The automation in
[`../runbooks/chunks/`](./chunks/) implements the amended values below, and they have been executed
and verified. The step-by-step prose further down still shows the older plaintext-on-3000 values in
places; **where the two disagree, this amendment wins.**

**What changed and why.** Steps 8–12 originally published the relay directly on port 3000 over
plaintext HTTP and told you to type `ws://`. [`hardening-spec.md`](./hardening-spec.md) §B2 identifies
that as a defect rather than a simplification — the restart command omits `compose.caddy.yml`, whose
`ports: !reset []` is *the only thing* that stops the relay publishing 3000, and `ufw` cannot close a
Docker-published port. §A.3 adds that running Caddy in dev is the single change that buys the most
production parity: identical Compose file sets, `wss://` through a reverse proxy, and the admin vhost
split all exercised before they matter.

**The amended dev values** (see [`../ansible/inventory/group_vars/dev.yml`](../ansible/inventory/group_vars/dev.yml)):

| Setting | Was | Now |
|---|---|---|
| `RELAY_URL` | `ws://buzz-vm.test:3000` | `wss://buzz-vm.test:8443` |
| Community host | `buzz-vm.test:3000` | `buzz-vm.test:8443` |
| `BUZZ_MEDIA_BASE_URL` | `http://buzz-vm.test:3000/media` | `https://buzz-vm.test:8443/media` |
| `BUZZ_MEDIA_SERVER_DOMAIN` | `buzz-vm.test:3000` | `buzz-vm.test:8443` |
| `BUZZ_CORS_ORIGINS` | `http://buzz-vm.test:3000` | `https://buzz-vm.test:8443` |
| `BUZZ_ADMIN_HOST` | `admin.buzz-vm.test:3000` | `admin.buzz-vm.test:8443` |
| Compose files | `compose.yml` + `compose.cohort.yml` | **plus `compose.caddy.yml`** |
| VM port forwards | `2222`, `3000` | `2222`, **`8080→80`, `8443→443`** |
| Desktop app address (Step 12.3) | `ws://buzz-vm.test:3000` | `wss://buzz-vm.test:8443` |

**Why 8443 and not 443.** Caddy binds 80/443 *inside* the guest, exactly as on the VPS, so container
configuration is byte-identical to production. But macOS refuses to let a non-root process bind a host
port below 1024, and VirtualBox host-only networking — which would have given the guest its own IP and
a real 443 — is unavailable because `/dev/vboxnetctl` does not exist (the network kext is not loaded).
So the *host* side of each forward is high. The relay preserves any non-default port in its community
host and strips only a trailing `:443`/`:80`, which is why dev's community carries `:8443` and
production's is a bare domain. `buzz_relay_url` was already one of the nine dev/prod variables in
§A.2, so this costs no parity the model had not already conceded.

**Four traps found by executing this path.** None appears anywhere else in this document:

1. **The WebSocket check must force HTTP/1.1.** Over TLS, `curl` negotiates HTTP/2 via ALPN, and
   HTTP/2 removed the `Connection: Upgrade` mechanism — so the relay sees a plain GET and answers
   `200` with the NIP-11 document. Verified: the identical request returns `101` with `--http1.1` and
   `200` without it. Step 8.2's test is correct as written *only* because it runs over plaintext; add
   `--http1.1` the moment it goes through Caddy.
2. **Capture only the status code from that check.** A successful upgrade leaves the socket open and
   `curl` then emits raw WebSocket frames, which are not valid UTF-8. Anything that deserializes the
   output (Ansible, `jq`) fails on it despite the relay behaving perfectly.
3. **Caddy declares no healthcheck**, so `docker compose ps --format json` reports `Health: ""` for it
   while `up --wait` prints `Healthy`. A check demanding `healthy` for every service fails on a good
   stack.
4. **Step 17 item 27's secret scan fails on a clean checkout.** `git grep -nE 'sk-or-v1-|OPENROUTER_API_KEY=sk'`
   matches this document's own prose, `crates/buzz-agent/README.md`, and
   `desktop/src-tauri/src/commands/agent_models_tests.rs` — which uses `sk-or-v1-secret-key-12345` as
   a fixture in a test *for secret redaction*. Scope the scan and match real key shapes instead.

**Two further corrections, from `hardening-spec.md` Part G:**

- **Step 7.3 lists `BUZZ_AUTO_MIGRATE=true` under "leave these alone — they are already correct."**
  Correct for a throwaway VM; **wrong for production**, where upstream's own `compose.yml` defaults it
  to `false`. With it true, `run.sh upgrade` can migrate a production schema as a side effect of
  pulling a newer image — no backup gate, no dry run, no rollback (§B6).
- **Step 9.4's `admin data: [] <- 200` is not a clean pass, it is the expected result of a
  Host-header-only check.** The admin API has no token, no Nostr auth and no membership check; its only
  credential is a matching `Host`. Correct on a loopback VM, an unauthenticated disclosure of
  moderation reports and feedback attachments on a public host. Production leaves `BUZZ_ADMIN_HOST`
  unset so the router is never mounted (§B1).
- **Step 7.1's "save both halves somewhere safe" undersells the relay key.** It cannot be rotated:
  losing or changing it breaks signature verification for every message the relay ever signed, and the
  answer to a suspected compromise is a new community, not a new key (§B8).

**Still owed.** The prose in Steps 7.3, 8.1–8.3, 9.2–9.4, 10 and 12.3 has not been rewritten
line-by-line; it still shows `:3000` and `ws://` in examples. Treat those as illustrations of the
mechanism, take the values from the table above, and finish the reconciliation.

---

## Conventions used here

- Commands that run **on your own computer** are shown plainly.
- Commands that run **inside the virtual machine** are prefixed with a comment saying so.
- Where a step genuinely differs, it is split into **On macOS** and **On Windows**.
- After most steps there is an **Expect to see** block with real output from a successful run. If
  yours differs materially, stop there rather than continuing.
- `<angle brackets>` mean *substitute your own value*.

Words you will meet:

| Term | Meaning here |
|---|---|
| **host** | Your own computer, Mac or PC. Confusingly, also means "the name part of a web address" — context will make it clear |
| **guest** | The virtual machine running Ubuntu |
| **VM** | Virtual machine |
| **relay** | The Buzz server program. Everything else exists to support it |
| **container** | A packaged program that runs in isolation. Docker runs containers |
| **SSH** | A way to get a terminal on another machine |
| **port forward** | A rule that makes a port on your own computer reach a port inside the VM |
| **shell** | The program that reads your typed commands. zsh on macOS, Bash on Windows — commands here work in both |

---

## Step 0 — Before you start

### 0.1 Open the right terminal

Do this first. It is what lets the rest of the document use one set of commands for both platforms.

**On macOS** — open **Terminal** (Applications → Utilities → Terminal). Nothing to install.

**On Windows** — install **Git for Windows** from <https://git-scm.com/download/win>, accepting the
defaults. This gives you **Git Bash**, which you will find in the Start menu. Open that.

Git Bash is a real Bash shell with `ssh`, `scp`, `curl`, `tar` and `openssl` included, which is why
this document can give you the same commands as a Mac user. Throughout, "terminal" means Terminal on
macOS and **Git Bash** on Windows.

> **Windows: do not use PowerShell or Command Prompt for this document.** The commands here are Bash
> and will fail or, worse, half-work in PowerShell. In particular PowerShell handles piped binary
> data differently and will silently corrupt file transfers.

> **Windows: do not use WSL2 for this document either.** WSL2 turns on Hyper-V, and Hyper-V and
> VirtualBox contend for the same hardware virtualisation features. VirtualBox 7 will still run
> alongside it but noticeably slower, and it is a known source of confusing failures. Git Bash needs
> no Hyper-V, so it avoids the problem entirely.

Check which shell you actually have:

```bash
ps -p $$ -o comm=
```

**Expect to see** `-zsh` or `zsh` on macOS, and `bash` on Windows. Either is fine — every command in
this document is written to work in both.

> **Do not use `bash --version` to check this.** It reports the version of a `bash` program somewhere
> on your PATH, not the shell you are actually typing into. On macOS those are different things:
> Terminal runs **zsh**, and the `/bin/bash` that ships with macOS is version 3.2 from 2007. A check
> that expects "bash version 5" fails on a perfectly normal Mac while telling you nothing about the
> shell you are in.
>
> The two shells differ in small ways that matter — most notably `read`, which is why Step 12.2 spells
> that command out the long way instead of using the usual shorthand.

### 0.2 Check your processor. This is a hard stop.

```bash
uname -m
```

**Expect to see** `x86_64`.

If you see **`arm64`**, you cannot complete this document on this machine. VirtualBox cannot run
x86_64 guests on ARM hardware, and the architecture has to match the production server. This affects:

- **Apple Silicon Macs** (M1/M2/M3/M4) — the majority of Macs sold since 2020
- **Windows on ARM** PCs, such as Snapdragon X models

Your options are to use an Intel Mac or an AMD/Intel PC, or to run this on a cloud VM instead. Note
that the whole point of this environment is matching the production server's specification, so
switching to an ARM guest defeats it.

### 0.3 Check you have enough free disk

You need **at least 15 GB free**, comfortably more. The VM's disk file grows as the guest writes to
it, and running your own machine out of space mid-install produces confusing failures that look like
faults in the VM.

**On macOS:**

```bash
df -h /System/Volumes/Data
```

**On Windows:**

```bash
df -h /c
```

Read the `Avail` column.

### 0.4 Install VirtualBox

Download and install from <https://www.virtualbox.org/wiki/Downloads> — the **x86/amd64** host
package for your platform. Then confirm your terminal can find its command-line tool:

```bash
VBoxManage --version
```

**Expect to see** a version like `7.1.4r165100`.

**On Windows**, if that reports `command not found`, VirtualBox installed correctly but Git Bash
cannot see it. Add it for this session:

```bash
export PATH="$PATH:/c/Program Files/Oracle/VirtualBox"
VBoxManage --version
```

To avoid retyping that in every new terminal:

```bash
echo 'export PATH="$PATH:/c/Program Files/Oracle/VirtualBox"' >> ~/.bashrc
```

### 0.5 Get the Buzz repository

Pick a folder you can find again. `~` means your home folder — `/Users/you` on macOS,
`C:\Users\you` on Windows.

```bash
mkdir -p ~/code
cd ~/code
git clone https://github.com/launchpad-26/buzz.git
cd buzz
```

**Expect to see** cloning progress, then a `buzz` folder. This is a large repository; allow a few
minutes.

From here on, "the repository" means `~/code/buzz`.

### 0.6 Turn on the project's build tools

You need Node and pnpm to build the two browser interfaces in Step 9 and the desktop app in Step 12.

**On macOS** — the repository pins its own versions using a tool called Hermit. Activate it in every
new terminal you use for this work:

```bash
cd ~/code/buzz
. ./bin/activate-hermit
```

> **Hermit alone will not give you pnpm on this machine, and Step 0.2 guarantees that.** The
> repository pins pnpm 11.x, and pnpm stopped publishing an Intel-macOS build at 11.0.5 — there is no
> `darwin-amd64` download for it to fetch. Since Step 0.2 sent every Apple Silicon reader away, every
> macOS reader here is on Intel, so every macOS reader hits this. Hermit fails with a "no source"
> error rather than anything that mentions architecture.
>
> Install an older pnpm and let it upgrade itself:
>
> ```bash
> hermit install pnpm-10.34.5
> ```
>
> pnpm reads the version the repository asks for and updates itself from there. If that still fails,
> use the same route this document gives Windows readers below — install Node from
> <https://nodejs.org> and run `corepack enable pnpm`, skipping Hermit for Node and pnpm entirely.
>
> This matters for **Step 12**, the desktop app, which is what you need in order to talk to the agent
> in Step 16. It is not optional and it cannot be worked around later.

**On Windows** — Hermit targets macOS and Linux, so install Node yourself instead. Download the
**LTS** installer from <https://nodejs.org>, accept the defaults, then close and reopen Git Bash and
enable pnpm:

```bash
node --version
corepack enable pnpm
```

Either way, confirm both tools answer:

```bash
node --version && pnpm --version
```

**Expect to see** two version numbers, for example `v24.15.0` and `11.4.0`. Exact versions may
differ; both commands succeeding is what matters.

> **Not verified on Windows.** The Hermit-free path above is the documented way to get Node on
> Windows, but nobody has yet built Buzz's interfaces that way. If Step 9 fails, this is the first
> thing to suspect, and the fix belongs back in this document.

---

## Step 1 — Download the Ubuntu image

We use Ubuntu's **cloud image** in `.ova` form rather than the normal installer ISO. The cloud
image needs no interactive install: it configures itself on first boot from a file we will write in
Step 3. That is what makes this whole process automatable later.

```bash
mkdir -p ~/vm-images
cd ~/vm-images
curl -L -o noble.ova \
  https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.ova
```

This is roughly 570 MB.

**Expect to see** a progress meter, then:

```bash
ls -lh ~/vm-images/noble.ova
```

showing a file of a few hundred MB.

> **Why `.ova` and not `.img`?** Ubuntu's `.img` cloud image is in a format called QCOW2 that
> VirtualBox cannot read. The `.ova` imports natively.

---

## Step 2 — Create the virtual machine

We are going to make a VM whose CPU, memory and swap match the production VPS: **1 CPU, 2048 MB
RAM, 496 MB swap**. Matching matters — a laptop with 32 GB of RAM hides every memory problem the
real server will have.

**Disk is deliberately not matched**, and that is the one dimension in which this VM is not a
production stand-in. Step 2.5 explains why, and the checklist in Step 17 says so explicitly, so nobody
draws storage conclusions from it.

### 2.1 Import the image

We deliberately do **not** tell VirtualBox where to put the VM. Letting it use its own default
folder means this command is identical on both platforms and you never have to type a path that
contains spaces.

```bash
VBoxManage import ~/vm-images/noble.ova --vsys 0 --vmname buzz-dev
```

**Expect to see** a progress meter ending with `Successfully imported the appliance.`

### 2.2 Find the disk that came with it

The imported disk is in a format that cannot be resized, so we must convert it. First ask VirtualBox
where it put it:

```bash
VBoxManage showvminfo buzz-dev --machinereadable | grep -i '\.vmdk'
```

**Expect to see** one line. On macOS it looks like:

```
"SCSI-0-0"="/Users/you/VirtualBox VMs/buzz-dev/buzz-dev-disk001.vmdk"
```

On Windows like:

```
"SCSI-0-0"="C:\Users\you\VirtualBox VMs\buzz-dev\buzz-dev-disk001.vmdk"
```

Read the left half as three values: **controller** `SCSI`, **port** `0`, **device** `0`. Yours may
say something other than `SCSI`. The right half is the **disk path**.

**Write down all four.** The next three steps need them, and the paths are different on every
machine, so this document cannot fill them in for you.

To save typing, put the disk path into a variable now — paste **exactly** what your own output
showed, keeping the quotes:

```bash
VMDK="/Users/you/VirtualBox VMs/buzz-dev/buzz-dev-disk001.vmdk"
VDI="/Users/you/VirtualBox VMs/buzz-dev/buzz-dev.vdi"
```

The second line is the same folder with a new filename — that is the disk we are about to create.

> **Windows: always keep the double quotes.** The path contains a space, and without quotes the
> command silently uses only the first word. If you pasted a path with backslashes it will work as
> shown; you do not need to convert them.

Check the variable is right before continuing:

```bash
ls -l "$VMDK"
```

**Expect to see** one file listed. If you see `No such file or directory`, the path is mistyped —
fix it before going on rather than debugging three commands later.

### 2.3 Convert the disk so it can be resized

```bash
VBoxManage clonemedium disk "$VMDK" "$VDI" --format VDI
```

**Expect to see** a progress meter and `Clone medium created in format 'VDI'`.

> **Why bother?** `VBoxManage modifymedium --resize` only works on VDI and VHD disks. Skip this and
> resizing later fails silently, leaving you wondering why the disk never grew.

### 2.4 Swap the new disk in and discard the old one

Substitute the controller, port and device you wrote down in 2.2.

```bash
VBoxManage storageattach buzz-dev --storagectl SCSI --port 0 --device 0 --medium none

VBoxManage closemedium disk "$VMDK" --delete

VBoxManage storageattach buzz-dev --storagectl SCSI --port 0 --device 0 \
  --type hdd --medium "$VDI"
```

**Expect to see** no output from any of the three. Silence is success.

### 2.5 Make the disk bigger

The cloud image ships a small disk. 20 GB is a sensible dev size:

```bash
VBoxManage modifymedium disk "$VDI" --resize 20480
```

**Expect to see** a progress meter to 100%.

> **If you opened a new terminal since 2.2**, `$VMDK` and `$VDI` are gone — variables last only for
> the session that set them. Set them again from your notes.

> **Deliberate difference from production.** The real VPS has a 49.5 GB disk. This VM is smaller on
> purpose, to save laptop space. That means this VM can prove software is *too big* to fit, but can
> never prove it *will* fit. Do not draw disk-capacity conclusions from it.

### 2.6 Set CPU, memory and graphics

```bash
VBoxManage modifyvm buzz-dev \
  --cpus 1 \
  --memory 2048 \
  --nic1 nat \
  --audio-driver none \
  --graphicscontroller vmsvga \
  --vram 16
```

**Expect to see** no output.

### 2.7 Add the two port forwards

The VM sits behind a private network. To reach it from your own computer, you add rules mapping a
port on `127.0.0.1` (your own machine) to a port inside the VM.

```bash
VBoxManage modifyvm buzz-dev --natpf1 "ssh,tcp,127.0.0.1,2222,,22"
VBoxManage modifyvm buzz-dev --natpf1 "buzz,tcp,127.0.0.1,3000,,3000"
```

**Expect to see** no output. Confirm:

```bash
VBoxManage showvminfo buzz-dev --machinereadable | grep -i natpf
```

**Expect to see** two lines mentioning `2222` and `3000`.

> **Why `127.0.0.1` and not blank?** Binding to `127.0.0.1` means only your own computer can reach these
> ports. Leaving it blank exposes your dev relay to everyone on your café's wifi.

---

## Step 3 — Write the VM's self-configuration files

The cloud image looks, on first boot, for a small disk labelled `cidata` containing two files. It
uses them to create users, install your SSH key, and set up swap. This replaces sitting through an
installer.

### 3.1 Make a working folder

```bash
mkdir -p ~/vm-images/seed
cd ~/vm-images/seed
```

### 3.2 Check you have an SSH key

```bash
cat ~/.ssh/id_ed25519.pub
```

**Expect to see** one line starting `ssh-ed25519 AAAA...`.

If it errors, create one:

```bash
ssh-keygen -t ed25519 -C "buzz-dev"
```

Press Enter at each prompt to accept defaults.

### 3.3 Create a password for console access

You will normally log in with the SSH key. The password is only a fallback for when the VM's
network is broken and you need the VirtualBox console window.

```bash
openssl passwd -6
```

Type a password twice. **Expect to see** a long string starting `$6$`. Copy the whole thing.

> **Never paste a password hash from another document into this file.** Generate your own. A shared
> hash is a shared password, and this account has root access.

### 3.4 Write the first file

```bash
cat > ~/vm-images/seed/meta-data <<'EOF'
instance-id: buzz-dev-001
local-hostname: buzz-dev
EOF
```

### 3.5 Write the second file

Substitute your real SSH public key and your real password hash. Both go where marked.

```bash
cat > ~/vm-images/seed/user-data <<'EOF'
#cloud-config
hostname: buzz-dev
manage_etc_hosts: true

# The production VPS is accessed as `ssh root@<ip>`, so root login is enabled
# here to match. This is a throwaway dev VM reachable only from your own computer.
disable_root: false

users:
  - default
  - name: root
    ssh_authorized_keys:
      - PASTE_YOUR_SSH_PUBLIC_KEY_HERE
  - name: dev
    gecos: Dev
    sudo: ALL=(ALL) NOPASSWD:ALL
    shell: /bin/bash
    lock_passwd: false
    passwd: "PASTE_YOUR_PASSWORD_HASH_HERE"
    ssh_authorized_keys:
      - PASTE_YOUR_SSH_PUBLIC_KEY_HERE

ssh_pwauth: true

chpasswd:
  expire: false
  users:
    - name: root
      password: "PASTE_YOUR_PASSWORD_HASH_HERE"
      type: hash
    - name: dev
      password: "PASTE_YOUR_PASSWORD_HASH_HERE"
      type: hash

write_files:
  # sshd uses the FIRST value it finds for a setting, and the cloud image's own
  # 60-cloudimg-settings.conf sets `PermitRootLogin prohibit-password`. This
  # file must sort BEFORE that one to win, hence the 01- prefix.
  - path: /etc/ssh/sshd_config.d/01-dev-parity.conf
    permissions: '0644'
    content: |
      PermitRootLogin yes
      PasswordAuthentication yes

runcmd:
  - [ systemctl, restart, ssh ]

# The cloud image ships no swap; the production VPS has a 496 MB swap partition.
# Match the size so memory-pressure behaviour is comparable.
swap:
  filename: /swapfile
  size: 520093696

package_update: false
package_upgrade: false
EOF
```

Now open the file and replace the four `PASTE_...` placeholders.

**On macOS:**

```bash
open -e ~/vm-images/seed/user-data
```

**On Windows:**

```bash
notepad ~/vm-images/seed/user-data
```

Save and close. Verify nothing was missed:

```bash
grep -c PASTE ~/vm-images/seed/user-data
```

**Expect to see** `0`. If you see any other number, you have placeholders left.

### 3.6 Turn the two files into a disk

The VM reads its configuration from a small CD image. Building one needs a tool, and this is the
only step where Mac and Windows use genuinely different software.

> **The volume name must be exactly `cidata`.** This is the single most common failure in this whole
> document. Get it wrong and the VM boots with no users, no SSH key, and no way in. If you later
> cannot log in, suspect this first.

**On macOS** — the tool is built in:

```bash
cd ~/vm-images
rm -f seed.iso
hdiutil makehybrid -iso -joliet -default-volume-name cidata -o seed.iso seed
```

**Expect to see** a couple of lines ending in a size summary. Then confirm the label:

```bash
hdiutil imageinfo ~/vm-images/seed.iso | grep -i -A1 "Volume Name"
```

**Expect to see** `cidata`.

**On Windows** — Windows ships no such tool, so install one. First install Chocolatey, a package
manager, following the single command at <https://chocolatey.org/install> — that one command must run
in **PowerShell as Administrator** (right-click Start → Terminal (Admin)). It is the only time this
document asks you to leave Git Bash.

Then, still in the Administrator PowerShell:

```
choco install cdrtools -y
```

Close it, return to **Git Bash**, and build the image:

```bash
cd ~/vm-images
rm -f seed.iso
mkisofs -output seed.iso -volid cidata -joliet -rock seed
```

**Expect to see** progress percentages and a total size. Then confirm the label:

```bash
isoinfo -d -i seed.iso | grep -i "Volume id"
```

**Expect to see** `Volume id: cidata`.

If `mkisofs` reports `command not found` after installing, close and reopen Git Bash so it picks up
the new program.

> **Not verified on Windows.** The macOS path above has been run successfully. The Chocolatey and
> `mkisofs` route is the standard way to produce a cloud-init image on Windows, but nobody has run it
> for this project. If it fails, or if the flags differ in the version you get, correct this document
> rather than working around it locally.

### 3.7 Plug the disk into the VM

```bash
VBoxManage storagectl buzz-dev --name IDE --add ide --controller PIIX4
VBoxManage storageattach buzz-dev --storagectl IDE --port 0 --device 0 \
  --type dvddrive --medium ~/vm-images/seed.iso
```

**Expect to see** no output. If the first command complains the controller already exists, that is
harmless — carry on.

---

## Step 4 — First boot and SSH

### 4.1 Start the VM

```bash
VBoxManage startvm buzz-dev --type headless
```

**Expect to see** `VM "buzz-dev" has been successfully started.`

`headless` means no window appears. The VM is running in the background.

### 4.2 Wait, then log in

First boot takes 60–90 seconds while the VM configures itself. Then:

```bash
ssh -p 2222 root@127.0.0.1
```

**Expect to see** a warning about the authenticity of the host — type `yes` — then an Ubuntu
welcome banner and a prompt like `root@buzz-dev:~#`.

If it says `Connection refused`, the VM is still booting. Wait 30 seconds and retry. If it still
fails after three minutes, go back to Step 3.6 and check the `cidata` label.

If `root` is refused but the VM is clearly up, try the fallback account instead — it has the same key
and passwordless `sudo`:

```bash
ssh -p 2222 dev@127.0.0.1
```

If that works, carry on as `dev` and put `sudo` in front of the commands in Steps 5 to 12 that expect
root.

### 4.3 Confirm the VM matches production

```bash
# inside the VM
free -h && swapon --show && df -h / && uname -m
```

**Expect to see** roughly:

```
               total        used        free      shared  buff/cache   available
Mem:           1.9Gi       296Mi       1.6Gi       1.0Mi       197Mi       1.6Gi
Swap:          495Mi          0B       495Mi
NAME      TYPE SIZE USED PRIO
/swapfile file 496M   0B   -2
Filesystem      Size  Used Avail Use% Mounted on
/dev/sda1        20G  2.2G   17G  12% /
x86_64
```

The important numbers are **1.9Gi memory** and **496M swap**. If swap is missing, cloud-init did
not apply your `user-data`.

Type `exit` to return to your own computer.

---

## Step 5 — Install Docker inside the VM

Buzz runs as a set of containers, so the VM needs Docker.

Connect first. This line runs **on your own computer**:

```bash
ssh -p 2222 root@127.0.0.1
```

Then, at the prompt inside the VM:

```bash
# inside the VM
apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y \
  docker.io docker-compose-v2 containerd
```

> **Do not paste those as one block.** The `ssh` line runs on your own computer and the rest run inside
> the VM. Pasted together, the remaining lines are fed to `ssh` as input rather than typed at the VM's
> prompt — they run without a terminal, print garbled output, and the session closes when the input
> ends. Every block in this document headed `# inside the VM` assumes you already have a prompt there.

**Expect to see** a few hundred lines of package output, ending without errors.

### 5.1 Check the versions

```bash
# inside the VM
docker --version && docker compose version && systemctl is-active docker
```

**Expect to see** roughly:

```
Docker version 29.1.3, build 29.1.3-0ubuntu3~24.04.2
Docker Compose version 2.40.3+ds1-0ubuntu1~24.04.1
active
```

**Docker Compose must be 2.24.4 or newer.** Buzz's TLS configuration uses a syntax older versions
cannot read. Ubuntu 24.04 ships 2.40.x, comfortably above the line.

> **Why Ubuntu's packages and not Docker's own repository?** Docker's official instructions add a
> third-party package source to the machine. Ubuntu's version is new enough, and staying with a
> single trusted source means security updates arrive through Ubuntu's normal channel with nothing
> extra to configure. On the production server that is one less thing to harden.

### 5.2 Note what Docker itself costs

```bash
# inside the VM
free -m | sed -n '2p'
```

**Expect to see** used memory rise to roughly **449 MB** from 296 MB. Docker's two background
processes account for about 125 MB of that. Worth knowing: on a 1.9 GB server, the tooling is not
free.

### 5.3 Save a restore point now, before anything else is installed

Do this before continuing. Right now the VM has Docker and nothing else — which is the exact state
you want to come back to if you need to measure how much the Buzz stack really consumes, or if a
later step goes wrong. Recreating this by hand means repeating Steps 1 to 5.

On your own computer:

```bash
VBoxManage controlvm buzz-dev acpipowerbutton
until VBoxManage showvminfo buzz-dev --machinereadable | grep -q '^VMState="poweroff"'; do sleep 3; done
VBoxManage snapshot buzz-dev take docker-clean \
  --description "Docker installed, no images pulled"
VBoxManage startvm buzz-dev --type headless
```

**Expect to see** `Snapshot taken.` between the shutdown and the restart.

The shutdown and restart keep the snapshot small — snapshotting a running VM also captures its
memory, which adds a couple of gigabytes to your laptop's disk for no benefit here.

---

## Step 6 — Copy the Buzz deployment files into the VM

Buzz's repository contains a small folder, `deploy/compose/`, that describes how to run the
production stack. That is all the VM needs — about 32 KB. The relay itself arrives later as a
prebuilt container image.

> **Do not clone the whole repository into the VM, and do not build Buzz from source there.** The
> source is roughly 460 MB and includes about 30 Rust packages. Compiling that on a single CPU with
> 2 GB of RAM will either take hours or run out of memory. The prebuilt image exists for this
> reason.

### 6.1 Work out which image version matches these files

Run this **on your own computer**, in the repository:

```bash
cd ~/code/buzz
git remote add upstream https://github.com/block/buzz.git
git fetch upstream main --filter=blob:none
git merge-base HEAD upstream/main
```

**Expect to see** a long commit identifier, for example `96ae14176ee...`.

That is the point where this fork last matched the upstream project. The container image built from
that same commit is the one that matches these deployment files. Take its **first seven
characters** — here `96ae141` — and the image name becomes:

```
ghcr.io/block/buzz:sha-96ae141
```

Check that image was actually published:

First put the seven characters into a variable so the rest of this document can reuse them.
Substitute your own:

```bash
BUZZ_SHA=96ae141
```

> **This variable lasts only for your current terminal session, and it does not travel over `ssh`.**
> Steps 7.1 and 14.2 run *inside the VM*, so the value cannot reach them — write the seven characters
> down and set `IMG` from your note there.

```bash
TOKEN=$(curl -s 'https://ghcr.io/token?scope=repository:block/buzz:pull&service=ghcr.io' \
  | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')

curl -s -o /dev/null -w "%{http_code}\n" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Accept: application/vnd.oci.image.index.v1+json" \
  "https://ghcr.io/v2/block/buzz/manifests/sha-$BUZZ_SHA"
```

> **`sed`, not `python3`.** An earlier version of this step used Python to read the token. Git Bash on
> Windows ships no Python, and on macOS `python3` is part of the Xcode command line tools, which this
> document does not install until Step 12 — so it would either fail outright or pop up a GUI
> installer. `sed` is present everywhere.

**Expect to see** `200`.

If you see `404`, that commit has no published image. Step back through earlier commits until one does:

```bash
git log --first-parent -10 --format=%h $(git merge-base HEAD upstream/main)
```

Try each with the check above until you get a `200`, and set `BUZZ_SHA` to the one that worked.

> **Then take the deployment files from *that* commit, not from your checkout.** Otherwise you pair an
> older relay binary with newer configuration files — the exact mismatch this step exists to prevent.
> Once `BUZZ_SHA` is settled, replace Step 6.2's copy with files extracted from that commit:
>
> ```bash
> rm -rf /tmp/buzz-compose && mkdir -p /tmp/buzz-compose
> git archive "$BUZZ_SHA" deploy/compose | tar -x -C /tmp/buzz-compose
> ```
>
> and copy `/tmp/buzz-compose/deploy/compose` in Step 6.2 instead of `deploy/compose`. If the merge-base
> image existed on the first try, your checkout already matches and no extraction is needed.

> **Why not just use the `main` tag?** `main` moves. Two people following this document a week
> apart would get different relay versions with the same configuration files, and their results
> would not be comparable. Worse, a newer relay may expect configuration these files do not
> provide.

### 6.2 Copy the folder in

Still on your own computer, from the repository root (`~/code/buzz`):

```bash
ssh -p 2222 root@127.0.0.1 'mkdir -p /opt/buzz'
scp -P 2222 -r deploy/compose root@127.0.0.1:/opt/buzz/
ssh -p 2222 root@127.0.0.1 'chown -R root:root /opt/buzz'
```

**Expect to see** a list of filenames with transfer progress from the `scp` line, and no output from
the other two.

> **`-P` is a capital letter here.** `scp` uses capital `-P` for the port while `ssh` uses lowercase
> `-p`. Lowercase `-p` on `scp` means "preserve timestamps" and your transfer will fail trying to
> reach the wrong port.

> **The `chown` matters.** The copy carries your own computer's user ID across, so without it the
> files end up owned by an unrelated user inside the VM.

### 6.3 Record where the files came from

Future you will want to know. Substitute the commit you found in 6.1:

```bash
ssh -p 2222 root@127.0.0.1 'cat > /opt/buzz/compose/UPSTREAM_COMMIT' <<'EOF'
source: https://github.com/launchpad-26/buzz
upstream sync point: 96ae14176
image: ghcr.io/block/buzz:sha-96ae141
delivered: copied by hand per dev-deployment-SOP.md
EOF
```

### 6.4 Confirm

```bash
ssh -p 2222 root@127.0.0.1 'ls -la /opt/buzz/compose/ && du -sh /opt/buzz'
```

**Expect to see** eight files — the seven that came from the repository (`.env.example`, `Caddyfile`,
`compose.caddy.yml`, `compose.dev.yml`, `compose.yml`, `README.md`, `run.sh`) plus the
`UPSTREAM_COMMIT` note you just wrote — and a total size around `32K`. `run.sh` must show an `x` in
its permissions.

---

## Step 7 — Create the configuration and secrets

This is the step where mistakes are most expensive, so it is broken down finely.

Log into the VM. This line runs **on your own computer**:

```bash
ssh -p 2222 root@127.0.0.1
```

Then, at the prompt inside the VM, start from the supplied example:

```bash
# inside the VM
cd /opt/buzz/compose
cp .env.example .env
chmod 600 .env
```

`chmod 600` means only root can read the file. It is about to contain every secret in the system.

### 7.1 Generate the two identities — read this before running anything

Buzz needs **two separate Nostr keypairs**, and confusing them causes problems that surface much
later:

| Keypair | Which half goes in `.env` | Purpose |
|---|---|---|
| **Owner** | the **public** key → `RELAY_OWNER_PUBKEY` | The administrator. Whoever holds the *secret* half controls the relay |
| **Relay** | the **secret** key → `BUZZ_RELAY_PRIVATE_KEY` | The relay's own signing identity |

> **The key generator will give you misleading advice.** It prints
> `Set BUZZ_PRIVATE_KEY to the secret key to use this identity.` **Ignore that line.** There is no
> `BUZZ_PRIVATE_KEY` in this deployment. Use the table above.

The key generator lives inside the Buzz image, so name your image once and reuse it. Substitute the
image you worked out in Step 6.1:

```bash
# inside the VM
BUZZ_SHA=96ae141                      # the seven characters from Step 6.1 — substitute yours
IMG="ghcr.io/block/buzz:sha-$BUZZ_SHA"
```

This lasts only for your current session in the VM. If you log out and back in, set it again.

Generate the first keypair — the owner:

```bash
# inside the VM
docker run --rm --entrypoint /usr/local/bin/buzz-admin "$IMG" generate-key
```

**Expect to see:**

```
Public key:  38980a43aba04331ba61b5e7b64b90e250cd411d042050eaf102a408acc6c379
Secret key:  <64 hex characters>

Set BUZZ_PRIVATE_KEY to the secret key to use this identity.
```

Save both halves somewhere safe outside the VM — a password manager. The secret half is the
administrator's login. Then note the **public** key for the next step.

Generate the second keypair — the relay's own:

```bash
# inside the VM
docker run --rm --entrypoint /usr/local/bin/buzz-admin "$IMG" generate-key
```

Note the **secret** key from this second run.

> **Do not reuse one keypair for both.** They are different roles.

### 7.2 Decide the hostname — the single most important value

The relay decides which community it serves from the address it is configured with, and **rejects
every connection whose address does not match exactly**. There is no command to add a community; it
is created from this one setting on startup.

For this dev VM we use `buzz-vm.test`.

> **Why `.test` and not `.local`?** `.local` is used by Apple's Bonjour service discovery, and
> macOS will try to resolve it over the network, giving intermittent failures that look like relay
> faults. `.test` is reserved by standard for exactly this purpose.

The matching rule you must understand: the relay strips a trailing `:443` or `:80` from the address
and **nothing else**. Our relay is on port 3000, so the port stays. The address must therefore be
written `ws://buzz-vm.test:3000`, and the community becomes `buzz-vm.test:3000`.

Get this wrong and everything appears healthy while every client is refused.

### 7.3 Fill in the file

Open the file:

```bash
# inside the VM
nano .env
```

`nano` is a basic text editor. Arrow keys to move, type to edit, `Ctrl-O` then Enter to save,
`Ctrl-X` to quit.

Change these lines. Generate each random value by running the command shown in a second terminal
inside the VM, then paste the result.

| Line to change | Set it to |
|---|---|
| `BUZZ_IMAGE=` | **the image name you worked out in Step 6.1** — e.g. `ghcr.io/block/buzz:sha-96ae141` |
| `BUZZ_DOMAIN=` | `buzz-vm.test` |
| `RELAY_URL=` | `ws://buzz-vm.test:3000` |
| `BUZZ_MEDIA_BASE_URL=` | `http://buzz-vm.test:3000/media` |
| `BUZZ_MEDIA_SERVER_DOMAIN=` | `buzz-vm.test:3000` |
| `BUZZ_CORS_ORIGINS=` | `http://buzz-vm.test:3000` |
| `RELAY_OWNER_PUBKEY=` | the **public** key from the *first* keypair |
| `BUZZ_RELAY_PRIVATE_KEY=` | the **secret** key from the *second* keypair |
| `BUZZ_GIT_HOOK_HMAC_SECRET=` | output of `openssl rand -hex 32` |
| `POSTGRES_PASSWORD=` | output of `openssl rand -hex 24` |
| `REDIS_PASSWORD=` | output of `openssl rand -hex 24` |
| `BUZZ_S3_ACCESS_KEY=` | output of `openssl rand -hex 16` |
| `BUZZ_S3_SECRET_KEY=` | output of `openssl rand -hex 32` |

Leave everything else as supplied. In particular leave these alone — they are already correct:

- `BUZZ_REQUIRE_RELAY_MEMBERSHIP=true` — only approved people may connect
- `BUZZ_AUTO_MIGRATE=true` — the relay creates its database tables on first start
- `BUZZ_REQUIRE_AUTH_TOKEN=true`

### 7.4 Check nothing was missed

```bash
# inside the VM
grep -En '^[[:space:]]*[A-Za-z_][A-Za-z0-9_]*=.*CHANGE_ME' .env
```

**Expect to see** no output. Anything printed is a value you still have to fill in, and the relay
will refuse to start.

You may still see `CHANGE_ME` in a comment on line 2. That is fine — comments are ignored.

### 7.5 Two things that stop the relay dead

Both of these are deliberate refusals, not bugs, and both are caused by mistakes in this step:

- **Missing `BUZZ_RELAY_PRIVATE_KEY`** while membership is required. The relay will not start with
  a temporary key, because messages signed with one become unverifiable after a restart.
- **An unusable `RELAY_URL`** from which no address can be worked out.

---

## Step 8 — Start the relay and verify it properly

```bash
# inside the VM
cd /opt/buzz/compose
./run.sh start
```

This downloads five container images (about 1.1 GB) then starts them in dependency order. Allow a
few minutes for the download; startup itself takes under a minute.

**Expect to see** a list of containers ending with:

```
 Container buzz-prod-relay-1  Healthy
```

Every container should reach `Healthy`, except `buzz-prod-minio-init-1`, which does one job and
correctly shows `Exited`.

### 8.1 Confirm the community was created

This is the evidence that Step 7.2 was done correctly:

```bash
# inside the VM
docker compose logs relay | grep -i "community"
```

**Expect to see:**

```
INFO Deployment community ensured host=buzz-vm.test:3000 community=d97ea868-...
```

The `host` value must be exactly `buzz-vm.test:3000`, **including the port**. If it says something
else, `RELAY_URL` is wrong; fix it and run `./run.sh restart`.

And directly in the database:

```bash
# inside the VM
docker compose exec -T postgres psql -U buzz -d buzz -c "select host from communities;"
```

**Expect to see** one row, `buzz-vm.test:3000`.

### 8.2 The test that actually proves it works

> **Read this even if you skip everything else.** The obvious checks are misleading. `/health`,
> `/_liveness` and the relay information document at `/` **all return success even when the address
> is completely wrong.** A relay that refuses every single client will pass all three. They tell you
> the program is running, not that it is usable.
>
> The only check that proves the address is right is attempting an actual WebSocket connection.

```bash
# inside the VM
curl -s -i -N --max-time 6 \
  -H "Host: buzz-vm.test:3000" \
  -H "Connection: Upgrade" -H "Upgrade: websocket" \
  -H "Sec-WebSocket-Version: 13" \
  -H "Sec-WebSocket-Key: AAAAAAAAAAAAAAAAAAAAAA==" \
  http://127.0.0.1:3000/ | head -1
```

**Expect to see:**

```
HTTP/1.1 101 Switching Protocols
```

`101` means the connection was accepted. Now prove the rejection works, by using a deliberately
wrong address:

```bash
# inside the VM
curl -s --max-time 6 \
  -H "Host: wrong.example.com" \
  -H "Connection: Upgrade" -H "Upgrade: websocket" \
  -H "Sec-WebSocket-Version: 13" \
  -H "Sec-WebSocket-Key: AAAAAAAAAAAAAAAAAAAAAA==" \
  http://127.0.0.1:3000/
```

**Expect to see:**

```
relay: no community is configured for this host
```

Seeing **both** of those — `101` for the right address, that message for the wrong one — is what
tells you the relay is correctly configured.

### 8.3 Check it fits

```bash
# inside the VM
free -m | sed -n '2,3p'
docker stats --no-stream --format "{{.Name}}  {{.MemUsage}}"
```

**Expect to see** roughly 570 MB used of 1968 MB, **swap at 0**, and per-container figures near:

| Container | Memory |
|---|---|
| relay | 9 MB |
| postgres | 51 MB |
| redis | 3 MB |
| minio | 92 MB |

If swap is being used at idle, something is wrong — investigate before continuing.

---

## Step 9 — Turn on the browser front-ends

Two web interfaces exist, and **the relay image already contains both of them** — you do not have to
build or copy anything. They just need switching on.

| Interface | What it is | Address |
|---|---|---|
| **Web bundle** | Invite landing pages, and optionally a repository browser | `buzz-vm.test:3000` |
| **Admin dashboard** | **Read-only** view of moderation reports and product feedback | `admin.buzz-vm.test:3000` |

> **The admin dashboard is not a control panel.** It cannot add users, change settings, or manage
> the roster. It only displays reports and feedback. Adding people is done from the command line in
> Step 11.

> **An earlier version of this document had you build both bundles with `pnpm` and copy them into the
> VM.** That was unnecessary: the relay image builds them during its own build and ships them at
> `/srv/buzz/web` and `/srv/buzz/admin-web`, with the two directory settings already pointing at them.
> The old steps copied files over the top of identical files, and did it from your fork's latest code
> rather than the commit the relay is pinned to — so they could actually introduce a mismatch. If you
> want to test your **own** changes to the front-end code, see the appendix at the end of this
> document.

### 9.1 Create our own Compose file

We need a file of our own for two things later — the agent in Step 14, and one setting the relay needs
in order to reach itself by name. The supplied `compose.yml` **must not be edited**: it belongs to the
upstream project and every local change becomes a merge conflict on the next sync.

```bash
# inside the VM
cat > /opt/buzz/compose/compose.cohort.yml <<'EOF'
# Our own additions. Deliberately NOT named compose.override.yml: Docker loads
# that name automatically for some commands but not others, which would give
# two different stacks depending on which command you ran. This name is never
# loaded automatically, so it is always explicit.
services:
  relay:
    # Makes the name `buzz-vm.test` resolve to the relay from inside the Docker
    # network. Needed by the agent in Step 14 — harmless until then.
    networks:
      buzz-net:
        aliases:
          - buzz-vm.test
EOF
```

> **Why the network alias, when nothing needs it yet.** The `buzz-vm.test` name you add to your own
> computer's hosts file in Step 10 exists **only on your own computer**. Containers do not see it.
> Docker's internal name service knows only service names like `relay`, and `.test` is a reserved
> suffix that never resolves on the public internet.
>
> That matters in Step 14, because the agent runs *inside* a container and has to reach the relay. It
> cannot use `relay:3000`, because the relay refuses any address that is not the one its community was
> created with — the same refusal you deliberately triggered in Step 8.2. The alias is what lets a
> container use the real name and get the real address. Writing it now means the file is created once.

### 9.2 Switch the two front-ends on

Only two settings are needed. The relay image already sets the directory paths, so they are not
repeated here.

```bash
# inside the VM
cat >> /opt/buzz/compose/.env <<'EOF'

# Relay-served front-ends. The bundles themselves ship inside the relay image;
# these two settings are what make them reachable.
BUZZ_ADMIN_HOST=admin.buzz-vm.test:3000
BUZZ_SERVE_GIT_WEB_GUI=true
EOF
```

`BUZZ_SERVE_GIT_WEB_GUI=true` is what makes the web bundle appear at the root address — without it
the bundle only answers invite links. `BUZZ_ADMIN_HOST` is what makes the admin dashboard exist at
all; the admin bundle is inert until it is set.

### 9.3 Restart, using both files

```bash
# inside the VM
cd /opt/buzz/compose
docker compose --env-file .env -f compose.yml -f compose.cohort.yml up -d --wait
```

> **Why not `./run.sh restart` here?** `run.sh` has a fixed list of configuration files built in
> and no way to add ours, so it would start the relay without our network alias — and later, without
> the agent. From this point on, use the longer command above whenever you restart. Note that
> `run.sh` also checks for unfilled secrets and this command does not, so re-run the check from
> Step 7.4 yourself if you have edited `.env`.

**Expect to see** every container reporting `Healthy`.

### 9.4 Verify all four surfaces

```bash
# inside the VM
echo "web root:"    ; curl -s -o /dev/null -w "%{http_code} %{content_type}\n" -H "Host: buzz-vm.test:3000"       -H "Accept: text/html" http://127.0.0.1:3000/
echo "admin page:"  ; curl -s -o /dev/null -w "%{http_code} %{content_type}\n" -H "Host: admin.buzz-vm.test:3000" -H "Accept: text/html" http://127.0.0.1:3000/reports
echo "admin data:"  ; curl -s -w "  <- %{http_code}\n" -H "Host: admin.buzz-vm.test:3000" http://127.0.0.1:3000/api/admin/v1/reports
echo "admin denied:"; curl -s -o /dev/null -w "%{http_code}\n" -H "Host: buzz-vm.test:3000" http://127.0.0.1:3000/api/admin/v1/reports
```

**Expect to see:**

```
web root:
200 text/html; charset=utf-8
admin page:
200 text/html; charset=utf-8
admin data:
[]  <- 200
admin denied:
403
```

That last `403` matters: the admin data is refused on the ordinary address and only served on the
admin one. `[]` is an empty list — correct on a fresh system with no reports.

---

## Step 10 — Make the addresses work in your browser

Your computer does not yet know what `buzz-vm.test` means. Two names must point at your own machine,
where the port forwards are waiting. Both platforms keep this in a protected system file, so both
need administrator rights to change it.

The two lines you are adding, either way:

```
127.0.0.1 buzz-vm.test
127.0.0.1 admin.buzz-vm.test
```

**On macOS** — in your terminal. It will ask for your login password:

```bash
sudo sh -c 'printf "\n127.0.0.1 buzz-vm.test\n127.0.0.1 admin.buzz-vm.test\n" >> /etc/hosts'
```

Confirm:

```bash
grep buzz-vm /etc/hosts
```

**On Windows** — Git Bash has no `sudo`, so use Notepad with administrator rights:

1. Press Start, type `notepad`
2. **Right-click** Notepad and choose **Run as administrator** — this is the part people miss, and
   without it the save silently fails
3. File → Open, and paste this into the filename box:
   `C:\Windows\System32\drivers\etc\hosts`
4. Add the two lines above at the end of the file
5. File → Save

Confirm, back in Git Bash:

```bash
grep buzz-vm /c/Windows/System32/drivers/etc/hosts
```

**Expect to see** both lines, whichever platform you are on.

> **If Notepad refuses to save**, you did not open it as administrator. Close it and repeat from
> step 2. Editing the file without those rights appears to work and then quietly discards the change.

Now open in your browser:

- <http://buzz-vm.test:3000> — the web bundle
- <http://admin.buzz-vm.test:3000/reports> — the admin dashboard

**Expect to see** a page render at each, rather than a connection error.

> **If the browser shows JSON instead of a page** at the first address, `BUZZ_SERVE_GIT_WEB_GUI` is
> not set — revisit 9.2. The same address deliberately serves different things to different
> callers: a page to browsers, relay information to Nostr clients, and a live connection to the
> apps.

---

## Step 11 — Approve people so they can connect

The relay is running closed: only approved keys may connect. The owner from Step 7.1 was approved
automatically. Anyone else must be added.

> **Roster commands only work after the relay has started successfully at least once**, because the
> tool looks up the community created at startup. If you see
> `RELAY_URL host '...' is not mapped to a community`, the relay has not started properly — fix
> that first.

Each person needs their public key. They can find it in their Buzz client. Then:

```bash
# inside the VM
cd /opt/buzz/compose
./run.sh add-member <their-public-key> --role member
```

**Expect to see** confirmation that the member was added.

List everyone currently approved:

```bash
# inside the VM
./run.sh list-members
```

> **Add people one at a time, with a pause between.** Adding several at once — or in a loop without
> a `sleep 1` — can corrupt the membership record because entries share a timestamp. Never use
> parallel commands for this.

---

## Step 12 — Connect the desktop app

The desktop app runs on your own computer, not in the VM, and is built from the repository.

The desktop app is built with Tauri, which compiles Rust as well as web code. That means each
platform needs its own build prerequisites, installed once:

- **macOS** — Xcode command line tools: `xcode-select --install`
- **Windows** — Microsoft Visual Studio C++ Build Tools, and the WebView2 runtime (already present on
  Windows 11). Tauri's own prerequisites page covers both:
  <https://tauri.app/start/prerequisites/>

  Also install **CMake** (<https://cmake.org/download/> — choose the "add to PATH" option) and
  **NASM** (<https://nasm.us>). A cryptography dependency compiles C code that needs both. macOS
  readers get CMake from Hermit and do not need to do anything; on Windows nothing supplies them, and
  without them the Rust build fails deep inside a C compile with a message that looks nothing like a
  missing tool.

Both also need Rust, which Tauri's page covers via `rustup`.

Then:

```bash
cd ~/code/buzz
```

**On macOS only:**

```bash
. ./bin/activate-hermit
```

### 12.1 Create the placeholder helper files

The desktop app bundles six helper programs, and Tauri checks that each one **exists while
compiling**. They are build outputs, so a fresh clone does not have them and the build fails with a
complaint about a missing `binaries/buzz-acp-…` file — which reads like a corrupt checkout rather than
a missing build step.

Empty placeholder files are enough to satisfy the check. Nothing on this document's path runs those
helpers on your own computer; the agent runs in the VM.

**On macOS:**

```bash
cd ~/code/buzz
just _ensure-sidecar-stubs
```

**On Windows** — Hermit is skipped there, so `just` is not installed. Create the files directly:

```bash
cd ~/code/buzz
TARGET=$(rustc -vV | sed -n 's|host: ||p')
mkdir -p desktop/src-tauri/binaries
for bin in buzz-acp buzz-agent buzz-dev-mcp git-credential-nostr buzz; do
  touch "desktop/src-tauri/binaries/${bin}-${TARGET}.exe"
done
```

> **The `.exe` on the end is required on Windows** and is easy to miss, because the macOS files do not
> have it. Tauri looks for `<name>-<target>.exe` on Windows and will not accept the extensionless name.
> Note that the repository's own `just _ensure-sidecar-stubs` recipe omits it too, so do not copy that
> recipe as a reference here — it works on macOS and Linux only.

Confirm either way:

```bash
ls desktop/src-tauri/binaries/
```

**Expect to see** five or six files whose names end in your platform's target, such as
`buzz-acp-x86_64-apple-darwin`.

### 12.2 Launch it, signed in as the owner

Your keys were printed as 64 hex characters, but the app's sign-in form only accepts the other common
format for the same key — the one beginning `nsec1` — and nothing in this document converts between
them. Pasting hex leaves the "Continue" button disabled with an unhelpful message.

So hand the app the identity directly instead. Read the key in without it appearing on your screen or
in your shell history:

```bash
cd ~/code/buzz
printf 'Owner secret key: '; stty -echo; read -r BUZZ_PRIVATE_KEY; stty echo; echo
export BUZZ_PRIVATE_KEY BUZZ_SHARE_IDENTITY=1
```

Paste the **secret** key from the *first* keypair in Step 7.1 when prompted. Nothing appears as you
type or paste — that is `stty -echo` doing its job. Press Enter.

> **That command is written the long way on purpose.** The obvious shorthand, `read -rsp "…" VAR`,
> works in Bash but **fails in zsh** — which is what macOS Terminal actually runs. In zsh, `-p` means
> "read from a coprocess", so you get `read: -p: no coprocess`, the variable is never set, and the app
> then launches on a throwaway identity that the agent will ignore in Step 16 with no error anywhere.
> The `printf` + `stty` form above behaves identically in both shells.

Confirm the variable actually holds something before going further:

```bash
[ ${#BUZZ_PRIVATE_KEY} -eq 64 ] && echo "key loaded, 64 characters" || echo "WRONG LENGTH: ${#BUZZ_PRIVATE_KEY}"
```

**Expect to see** `key loaded, 64 characters`. Anything else means the paste did not land — redo the
`printf`/`read` line rather than continuing.

Then:

```bash
pnpm install
pnpm -C desktop tauri dev
```

The first run compiles Rust components and is slow — expect several minutes, possibly much longer on
a first-ever Rust build. Subsequent runs are far faster.

**Expect to see** a desktop window open, already signed in as the owner.

> **Do not use `just desktop-standalone` for this.** It looks like the right recipe and it does build
> the helpers properly — but it runs `unset BUZZ_PRIVATE_KEY BUZZ_SHARE_IDENTITY` just before starting
> the app, deliberately, so the identity you supplied is discarded and you land on the sign-in screen
> with no way to get past it. That is why 12.1 creates the placeholder files separately and this step
> uses `tauri dev` directly.

> **`BUZZ_PRIVATE_KEY` is what does the work here.** The app reads it at startup and that identity
> takes precedence over anything saved from a previous run. It accepts the 64-hex form, which is why
> this sidesteps the sign-in form's `nsec1`-only restriction. `BUZZ_SHARE_IDENTITY=1` is set alongside
> it because it governs related identity-sharing behaviour, but it is **not** the mechanism — do not
> assume setting it alone will do anything.
>
> **If `BUZZ_PRIVATE_KEY` is malformed, the app does not stop.** It prints
> `buzz-desktop: invalid BUZZ_PRIVATE_KEY: …` and quietly falls back to whatever identity was saved
> before, or a throwaway one. That is why 12.4 checks the key it actually loaded rather than trusting
> that the launch looked fine.
>
> **Do not click "Create a new identity key"** if the app offers it. That makes a *different*
> identity, and the agent will then ignore you in Step 16 with no error anywhere.
>
> `export` matters. Without it the variables apply only to the line that set them, not to the
> `pnpm` command that follows.

> **This is not a fully isolated dev launch.** `just desktop-standalone` also sets a development app
> identifier, a separate keyring, and per-worktree ports; running `tauri dev` directly skips all of
> that, so this uses the standard app identifier and can share stored data with another Buzz install on
> the same machine. For a throwaway dev VM that is acceptable. If you already run Buzz on this
> computer, be aware the two can interact.

### 12.3 Point it at your relay — type the address in full

The first screen offers three choices, and **the one that sounds right is the wrong one.**

Choose **Join a community**. (Or, if you go via **I already have a community**, then pick
**I'm a member or admin**.)

> **Do not choose "Create a community", and do not choose "I own the community".** Both open a
> sign-in for Block's *hosted* service — an account you do not have and do not need — and neither
> offers a field to type your VM's address into. This is a genuine trap: you have just been told at
> length that you are the owner of this community, so "I own the community" is the obvious click, and
> it is a dead end. Back out and choose the member path instead.

In the single address field, enter **exactly**:

```
ws://buzz-vm.test:3000
```

Leave any invite-code field empty — you do not need one.

> **You must type `ws://` at the front.** If you leave the scheme off, the app assumes `wss://`,
> which means an encrypted connection. This dev relay does not use encryption, so the connection
> will fail with an error that does not explain why. Production will use `wss://`; this VM does not.

**Expect to see** the app connect and show the community. It may first ask for a display name and walk
you through a short welcome sequence — that is normal and this document does not describe those screens
in detail; work through them and you will arrive at the channel list.

If it does not connect, check in order: the relay is healthy (Step 8), you typed `ws://`, you typed
the port `:3000`, and your key has been approved (Step 11).

### 12.4 Confirm you are the owner

Everything in Step 16 depends on the app running as the owner, so check it properly rather than
assuming.

**Look in the terminal you launched from**, among the startup output, for this line:

```
buzz-desktop: configured identity pubkey 38980a43aba04331ba61b5e7b64b90e250cd411d042050eaf102a408acc6c379
```

**Expect to see** that line present, and its 64 hex characters **identical** to the **public** key of
the *first* keypair from Step 7.1.

That is the whole check, and it works because this line reports the key the app actually loaded, in the
same format Step 7.1 gave you.

| What you see | Meaning |
|---|---|
| The line, matching your owner public key | Correct — continue |
| The line, but a **different** key | You pasted the wrong secret. Relaunch with the right one |
| `buzz-desktop: invalid BUZZ_PRIVATE_KEY: …` | The key was mistyped or truncated. The app has fallen back to another identity — relaunch |
| No such line at all | The variables did not reach the app. Check you used `export` in 12.2, and relaunch |

> **Do not use the app's own profile display for this comparison.** It shows identities in the other
> common format, beginning `npub1` — the same key written differently — and nothing in this document
> converts between the two, so comparing by eye there is not a check you can actually perform. The
> terminal line is in hex, which is why it is the one to use.
>
> **Reaching the community without being asked to sign in is not sufficient proof.** If the key was
> malformed the app silently reuses a previously saved identity, which also skips the sign-in screen.

This matters more than it looks. The agent you add in Step 14 is configured, by default, to take
instructions from **its owner and nobody else**. If you are signed in as any other identity, the
agent will connect, look perfectly healthy, and silently ignore every message you send it. That
failure is indistinguishable from a broken agent, and it costs an hour to find.

If they do not match, close the app and relaunch it with the correct secret key.

### 12.5 Create a channel

Step 16 needs somewhere to talk to the agent, and nothing so far has made a channel.

In the app, create a channel called `agent-test`. Post a message in it and confirm it appears.

**Expect to see** the message you just sent, in the channel, without an error.

> **If posting fails**, the relay is refusing you. Re-check Step 8.1 (the community host matches) and
> Step 11 (your key is approved). A connected app does not prove an approved key — the connection
> succeeds first and authentication happens after it.

---

## Step 13 — Harden the virtual machine

Everything up to here got Buzz working. This step applies the host-level security configuration, and it
is deliberately placed **after** the stack works rather than before.

> **Scope, stated plainly: this is a subset, not production parity.** It covers the controls that make
> sense on a loopback-only dev VM — SSH policy, an ingress firewall, kernel parameters, automatic
> security updates, and the Docker daemon. It deliberately does **not** implement the full production
> baseline in [`hardening-spec.md`](./hardening-spec.md), which additionally requires TLS via Caddy with
> the relay's port unpublished, `DOCKER-USER` rules, default-deny **egress**, split internal/external
> container networks, module blacklisting, systemd resource limits, `auditd` and sudo I/O logging,
> authenticated time, image digest pinning, and a verification suite.
>
> Those matter on a public host and are tracked as their own work. Do not read the end of this step as
> "this machine is now production-hardened" — read it as "the lockout-risky plays have been rehearsed
> and the dev VM is no longer trivially insecure."

The reason is debugging. Hardening breaks things — that is what it is for. If you apply it to a
machine you have never seen working, and something then fails, you cannot tell whether hardening
broke it or it was never right. With a working stack behind you, every failure in this step has
exactly one cause.

> **Production runs these in the opposite order** — a real server is hardened before it is exposed
> and before it has real data. That is not a contradiction: the roles are the same and the finished
> state is the same, only the rehearsal order differs. This VM exists precisely so that the plays
> that can lock you out get rehearsed somewhere disposable first.

### 13.1 Take a snapshot. Do not skip this.

Two of the changes below can lock you out of the machine permanently. This snapshot is the undo.

On your own computer:

```bash
VBoxManage controlvm buzz-dev acpipowerbutton
until VBoxManage showvminfo buzz-dev --machinereadable | grep -q '^VMState="poweroff"'; do sleep 3; done
VBoxManage snapshot buzz-dev take buzz-working \
  --description "Buzz running, desktop connected, before hardening"
VBoxManage startvm buzz-dev --type headless
```

**Expect to see** `Snapshot taken.` between the shutdown and the restart.

If any later step locks you out:

```bash
VBoxManage controlvm buzz-dev poweroff
VBoxManage snapshot buzz-dev restore buzz-working
VBoxManage startvm buzz-dev --type headless
```

### 13.2 Prove you can get in as a non-root user — before you disable root

You have been logging in as `root` this whole time. Step 13.3 turns that off. If the `dev` account
does not work when it does, you have no way in at all.

So test it **first**:

```bash
ssh -p 2222 dev@127.0.0.1 'sudo whoami'
```

**Expect to see** `root`.

If that does not print `root`, **stop here** and fix it before going further. Do not continue on the
assumption that it will work later. The `dev` user and its passwordless `sudo` were created by the
cloud-init file you wrote in Step 3.5; if they are missing, that file did not apply.

From this point on, log in as `dev`, not `root`:

```bash
ssh -p 2222 dev@127.0.0.1
```

Commands that previously ran as root now need `sudo` in front of them.

### 13.3 Turn off root login and password authentication

The VM was built deliberately insecure — `PermitRootLogin yes` and `PasswordAuthentication yes`,
with a known password on both accounts — because it is reachable only from your own computer. The
production server must never be configured that way, so this is where that gets undone.

> **The trap in this step, and it bites twice.** SSH uses the **first** value it finds for a setting,
> and files in `/etc/ssh/sshd_config.d/` are read in name order. Step 3.5 wrote
> `01-dev-parity.conf` with a `01-` prefix specifically so it would sort first and win.
>
> So two things follow, and missing either one leaves you believing you hardened a machine you did
> not:
>
> 1. **`01-dev-parity.conf` must be deleted.** Adding another file alongside it changes nothing.
> 2. **The new file must sort before everything else too**, not after. The cloud image ships its own
>    `60-cloudimg-settings.conf` containing `PermitRootLogin prohibit-password`, and cloud-init may
>    write a `50-cloud-init.conf` containing `PasswordAuthentication yes`. A file named
>    `99-hardening.conf` loses to **both** of those, because `50` and `60` sort before `99`. Root
>    login would stay possible by key and passwords would stay enabled, while the file you wrote sat
>    there looking correct.
>
> That is why the file below is named `00-hardening.conf`. First in name order, therefore first
> read, therefore the value that wins.

First see what is actually in there, so you know what you are competing with:

```bash
# inside the VM
ls -1 /etc/ssh/sshd_config.d/
```

**Expect to see** `01-dev-parity.conf` and `60-cloudimg-settings.conf`, and possibly
`50-cloud-init.conf`. Now remove the dev-parity file and write yours as `00-`:

```bash
# inside the VM
sudo rm /etc/ssh/sshd_config.d/01-dev-parity.conf

sudo tee /etc/ssh/sshd_config.d/00-hardening.conf >/dev/null <<'EOF'
PermitRootLogin no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
PermitEmptyPasswords no
MaxAuthTries 3
LoginGraceTime 30
X11Forwarding no
EOF
```

Check the file is valid **before** restarting. A syntax error here plus a restart is a lockout:

```bash
# inside the VM
sudo sshd -t && echo "config OK"
```

**Expect to see** `config OK`. If you see an error instead, fix it before continuing — do not
restart SSH.

Now apply it, and verify what SSH actually believes rather than what the file says:

```bash
# inside the VM
sudo systemctl restart ssh
sudo sshd -T | grep -E '^(permitrootlogin|passwordauthentication|kbdinteractiveauthentication)'
```

**Expect to see:**

```
permitrootlogin no
passwordauthentication no
kbdinteractiveauthentication no
```

> **Verify with `sshd -T`, never by looking at the file.** `sshd -T` prints the settings actually in
> force after all files are merged. That is the only output that proves the trap above was avoided.

Finally, confirm from your own computer — in a **new** terminal, keeping your existing session open
in case something is wrong:

```bash
ssh -p 2222 root@127.0.0.1
```

**Expect to see** `Permission denied (publickey)`. That refusal is the success condition.

### 13.4 Turn on the firewall

Default deny inbound, then allow only SSH back in.

```bash
# inside the VM
sudo apt-get update
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y ufw

sudo ufw default deny incoming
sudo ufw default allow outgoing
sudo ufw allow 22/tcp
sudo ufw --force enable
sudo ufw status verbose
```

**Expect to see** `Status: active`, a default of `deny (incoming)`, and one `22/tcp ALLOW IN` rule.

Then confirm you did not just cut yourself off — from a **new** terminal on your own computer:

```bash
ssh -p 2222 dev@127.0.0.1 'echo still-here'
```

**Expect to see** `still-here`.

> **`ufw` does not control Docker's ports, and this surprises everyone.** Docker writes its own
> firewall rules that are consulted *before* `ufw`'s. Port 3000 is still reachable right now even
> though `ufw` says everything is denied — you can prove it by loading
> <http://buzz-vm.test:3000> in your browser, which still works.
>
> On this VM that is harmless: VirtualBox only forwards those ports from `127.0.0.1`, so nothing off
> your own computer can reach them either way. **On the production server it is not harmless**, and
> the fix there is to stop publishing the relay's port at all and put it behind the Caddy reverse
> proxy that ships in `deploy/compose/compose.caddy.yml`. That is production work, not dev work, and
> it is written up in [`hardening-spec.md`](./hardening-spec.md) §B2. It is noted here so nobody
> reads `ufw status` on the real server and believes the port is closed.

### 13.5 Turn on automatic security updates

```bash
# inside the VM
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y unattended-upgrades

sudo tee /etc/apt/apt.conf.d/52buzz-hardening >/dev/null <<'EOF'
Unattended-Upgrade::Allowed-Origins {
        "${distro_id}:${distro_codename}-security";
};
Unattended-Upgrade::Automatic-Reboot "false";
EOF

sudo systemctl enable --now apt-daily.timer apt-daily-upgrade.timer
sudo systemctl is-enabled apt-daily-upgrade.timer

command -v unattended-upgrade
set -o pipefail
sudo unattended-upgrade --dry-run --debug 2>&1 | tail -5
echo "exit status: $?"
```

**Expect to see** `enabled` from the timer check, then output mentioning the packages it would
consider, then `exit status: 0`.

> **`set -o pipefail` is doing real work here.** Without it, `… | tail -5` reports `tail`'s exit
> status, not the upgrade command's. The check would print a cheerful `0` even when the command it was
> supposed to be testing had failed. Installing the package is also not the same as switching it on,
> which is why the timers are enabled explicitly rather than assumed.
>
> **The binary is `unattended-upgrade`, singular**, even though the package is
> `unattended-upgrades`, plural. The plural form is not reliably present, so using it gives you
> `command not found` — and with `pipefail` on, a failing exit status against an expectation of `0`,
> on the one step whose whole point is that its success signal can be trusted. The `command -v` line
> above makes a missing binary read as a missing binary.

> **Only the `-security` origin, and no automatic reboot.** Ubuntu's `universe` component has no
> guaranteed security updates, so auto-patching from it gives you the reboots without the safety.
> And a server that reboots itself at 3am because a kernel arrived is a server that took your
> service down without asking. Reboots are a decision, not a side effect.

### 13.6 Apply the kernel settings

These close a set of information leaks and disable kernel features this machine has no use for.

```bash
# inside the VM
sudo tee /etc/sysctl.d/99-buzz-hardening.conf >/dev/null <<'EOF'
# Hide kernel addresses and logs from unprivileged users
kernel.kptr_restrict = 2
kernel.dmesg_restrict = 1
kernel.yama.ptrace_scope = 1
kernel.sysrq = 0
kernel.kexec_load_disabled = 1
kernel.perf_event_paranoid = 3
kernel.unprivileged_bpf_disabled = 1
net.core.bpf_jit_harden = 2

# No core dumps — they can contain secrets read from the environment
fs.suid_dumpable = 0

# Filesystem link protections
fs.protected_symlinks = 1
fs.protected_hardlinks = 1
fs.protected_fifos = 2
fs.protected_regular = 2

# Network
net.ipv4.conf.all.rp_filter = 1
net.ipv4.tcp_syncookies = 1
net.ipv4.conf.all.send_redirects = 0
net.ipv4.conf.all.accept_redirects = 0
net.ipv4.conf.all.accept_source_route = 0
net.ipv4.conf.all.log_martians = 1
net.ipv4.icmp_echo_ignore_broadcasts = 1
EOF

sudo sysctl --system >/dev/null && sudo sysctl kernel.kptr_restrict fs.suid_dumpable
```

**Expect to see:**

```
kernel.kptr_restrict = 2
fs.suid_dumpable = 0
```

> **One setting is deliberately missing.** Most Linux hardening guides tell you to set
> `net.ipv4.ip_forward = 0`. **Do not add it here.** Docker requires IP forwarding and simply turns
> it back on, so all you achieve is a setting that permanently disagrees with reality and makes every
> future audit look like a failure.

### 13.7 Harden the Docker daemon

```bash
# inside the VM
sudo tee /etc/docker/daemon.json >/dev/null <<'EOF'
{
  "no-new-privileges": true,
  "icc": false,
  "live-restore": true,
  "log-driver": "journald"
}
EOF
```

Check it before restarting, because a bad key here stops Docker starting and takes Buzz with it:

```bash
# inside the VM
sudo dockerd --validate && echo "daemon config OK"
```

**Expect to see** `configuration OK` from the validator, then `daemon config OK`.

Only then:

```bash
# inside the VM
sudo systemctl restart docker
sleep 10
sudo docker ps --format '{{.Names}}  {{.Status}}'
```

**Expect to see** your containers listed and running again.

**Now recreate them, or the settings you just wrote do not apply to them.** `daemon.json` supplies
*defaults for new containers*. Your relay, database, Redis and MinIO containers were created before it
existed, and restarting the daemon does not rebuild them — with `live-restore: true` it deliberately
leaves them alone. They keep their old logging driver and no `no-new-privileges`:

```bash
# inside the VM
cd /opt/buzz/compose
sudo docker compose --env-file .env -f compose.yml -f compose.cohort.yml up -d --force-recreate --wait
```

Then confirm the settings actually took:

```bash
# inside the VM
sudo docker inspect buzz-prod-relay-1 --format '{{.HostConfig.LogConfig.Type}}'
sudo docker exec buzz-prod-relay-1 grep NoNewPrivs /proc/1/status
```

**Expect to see** `journald` from the first command and `NoNewPrivs:	1` from the second. If the first
says `json-file`, the recreate did not happen — re-run the command above.

> **Two commands, because one of them cannot be checked the obvious way.** You might expect
> `docker inspect … {{.HostConfig.SecurityOpt}}` to show `no-new-privileges`. It will not: that field
> only reports flags given when the container was created, and this setting arrived as a *daemon
> default*. It renders as an empty list even when the setting is active, which looks like a failure and
> would send you round the recreate loop forever. Reading `/proc/1/status` inside the container asks
> the kernel directly, which is the thing that actually matters.

> **`icc: false` is not container isolation, despite how it reads.** It only governs Docker's *default*
> bridge network. Buzz runs on its own `buzz-net` network, where containers still reach each other —
> which is required, since the relay must talk to Postgres, Redis and MinIO. So it neither breaks this
> stack nor hardens it. It is here because it is the right default for anything that later runs on the
> default bridge. Do not remove it while debugging a connectivity problem; it will not be the cause.

> **Do not copy a `daemon.json` out of an internet hardening guide.** Two settings that appear in
> many of them are actively harmful: `disable-legacy-registry` was removed from Docker years ago and
> now **stops the daemon starting**, and `ip-forward-no-drop` switches *off* the protection that
> makes Docker 28+ drop traffic to unpublished container ports. `dockerd --validate` catches the
> first. Nothing catches the second.

### 13.8 Confirm Buzz still works

Hardening that breaks the thing you are hardening is not finished. Re-run the checks that mattered:

```bash
# inside the VM
cd /opt/buzz/compose
sudo docker compose ps

curl -s -i -N --max-time 6 \
  -H "Host: buzz-vm.test:3000" \
  -H "Connection: Upgrade" -H "Upgrade: websocket" \
  -H "Sec-WebSocket-Version: 13" \
  -H "Sec-WebSocket-Key: AAAAAAAAAAAAAAAAAAAAAA==" \
  http://127.0.0.1:3000/ | head -1

free -m | sed -n '2,3p'
```

**Expect to see** every container healthy, `HTTP/1.1 101 Switching Protocols`, and **swap still at
0**.

Then reload the desktop app and confirm it is still connected.

Take a second snapshot now that the machine is both working and hardened. **This runs on your own
computer, not in the VM** — `VBoxManage` does not exist inside the guest. Type `exit` to leave your
SSH session first, or open a second terminal:

```bash
VBoxManage controlvm buzz-dev acpipowerbutton
until VBoxManage showvminfo buzz-dev --machinereadable | grep -q '^VMState="poweroff"'; do sleep 3; done
VBoxManage snapshot buzz-dev take buzz-hardened \
  --description "Buzz running, hardening applied, verified"
VBoxManage startvm buzz-dev --type headless
```

**Expect to see** `Snapshot taken.` between the shutdown and the restart.

> **Shut down first, as in Steps 5.3 and 13.1.** Snapshotting a running VM also captures its memory,
> which adds a couple of gigabytes to your laptop's disk for no benefit.

---

## Step 14 — Add an AI agent

The agent is a member of your community with its own identity, like a person. You @mention it in a
channel and it replies. Behind that, four programs are involved:

```
  Buzz relay ──WS──→ buzz-acp ──stdio──→ buzz-agent ──HTTPS──→ openrouter.ai
   (@mention)        the harness          the model loop
                                               │
                                               │ stdio
                                               ▼
                                          buzz-dev-mcp
                                        (shell + file tools)
                                               │
                                               ▼
                                    `buzz messages send` → back to the relay
```

Read the bottom of that diagram carefully, because it explains a failure you are otherwise very
likely to hit: **the agent replies by running a shell command.** It has no built-in "post a message"
ability. Take its tools away and it can think, call the model, and still be completely unable to
speak.

All four programs are the same file. The `ghcr.io/block/buzz-sprig` image contains one binary with
`buzz-acp`, `buzz-agent`, `buzz-dev-mcp` and `buzz` all pointing at it. So the agent is **one more
container**.

Step 13.8 powered the VM off and on, so your SSH session is gone. Reconnect before continuing — as
`dev`, because 13.3 turned root login off:

```bash
ssh -p 2222 dev@127.0.0.1
```

### 14.1 Get an OpenRouter key

OpenRouter is a service that forwards requests to many AI models with one key and one bill.

1. Sign up at <https://openrouter.ai>
2. Add a small amount of credit — a few dollars is plenty for testing
3. Create a key under **Keys**. It starts `sk-or-v1-`
4. **Set a spending limit on the key while you are there.** This key is a payment instrument. It is
   about to live on a virtual machine and be used by a program that decides for itself how often to
   call it

Keep it somewhere safe. You will paste it in 14.4.

### 14.2 Generate the agent's identity

This is a **third** keypair, separate from the owner and the relay keys in Step 7.1. Do not reuse
either of those — they are different roles, and every agent needs its own identity.

```bash
# inside the VM
BUZZ_SHA=96ae141                      # the seven characters from Step 6.1 — substitute yours
IMG="ghcr.io/block/buzz:sha-$BUZZ_SHA"
sudo docker run --rm --entrypoint /usr/local/bin/buzz-admin "$IMG" generate-key
```

**Expect to see** a public and a secret key, as in Step 7.1. Save both.

Ignore the `Set BUZZ_PRIVATE_KEY to the secret key` line for the relay's purposes — but note that
for the *agent*, unlike the relay, that advice is actually correct. `BUZZ_PRIVATE_KEY` is exactly
where this secret goes in 14.4.

### 14.3 Approve the agent

The relay only lets approved keys connect, and the agent is no exception.

```bash
# inside the VM
cd /opt/buzz/compose
sudo ./run.sh add-member <agent public key> --role member
sudo ./run.sh list-members
```

**Expect to see** the agent's public key in the list.

> **One at a time, with a pause.** The same warning as Step 11 applies: adding members in a loop or
> in parallel can corrupt the membership record, because entries share a timestamp.

### 14.4 Write the agent's configuration

The agent's settings go in their own file, not in the relay's `.env`. Your OpenRouter key is a
spending credential, and it has no business being in the environment of the database, the object
store, or the relay.

```bash
# inside the VM
sudo tee /opt/buzz/compose/.env.agent >/dev/null <<'EOF'
# --- who the agent is ---
BUZZ_PRIVATE_KEY=PASTE_AGENT_SECRET_KEY_HERE
BUZZ_RELAY_URL=ws://buzz-vm.test:3000

# --- which agent program to run ---
# The default is `goose`, which is not in this image, so this must be set.
# The arguments are set empty for clarity; the harness already resolves
# buzz-agent to no arguments on its own.
BUZZ_ACP_AGENT_COMMAND=buzz-agent
BUZZ_ACP_AGENT_ARGS=

# --- the tools it needs in order to reply at all ---
BUZZ_ACP_MCP_COMMAND=buzz-dev-mcp

# --- who it takes instructions from ---
BUZZ_ACP_AGENT_OWNER=PASTE_OWNER_PUBLIC_KEY_HERE

# --- which model, and the key that pays for it ---
BUZZ_AGENT_PROVIDER=openrouter
OPENROUTER_API_KEY=PASTE_OPENROUTER_KEY_HERE
OPENROUTER_MODEL=anthropic/claude-sonnet-4.5

# --- behaviour ---
BUZZ_AGENT_REQUIRE_REPLY=1
BUZZ_ACP_AGENTS=1

# Without this the harness hides its own log lines, which are the ones the
# troubleshooting in Steps 15 and 16 rely on. The agent subprocess prints its
# own errors to the same place regardless of this setting.
RUST_LOG=buzz_acp=info
EOF

sudo chmod 600 /opt/buzz/compose/.env.agent
sudo nano /opt/buzz/compose/.env.agent
```

Replace the three `PASTE_` placeholders:

| Placeholder | What goes there |
|---|---|
| `PASTE_AGENT_SECRET_KEY_HERE` | the **secret** key from 14.2 |
| `PASTE_OWNER_PUBLIC_KEY_HERE` | the **public** key of the *owner*, from Step 7.1 |
| `PASTE_OPENROUTER_KEY_HERE` | your `sk-or-v1-...` key from 14.1 |

`OPENROUTER_MODEL` is not a placeholder — leave it as supplied unless you have a reason not to.

> **If you change the model, it must support tool calling.** The agent's only way to reply is to run a
> command through a tool, so a model that cannot make tool calls will read your message, produce a
> perfectly sensible answer, and have no way to deliver it. OpenRouter serves plenty of such models,
> and the connection test in Step 15 will return `200` for them — nothing in this document catches it
> except the mention in Step 16 going unanswered.
>
> The supplied `anthropic/claude-sonnet-4.5` supports tool calling. If you pick another, check its
> entry on OpenRouter's model list for tool-calling support first.

Check nothing was missed:

```bash
# inside the VM
sudo grep -c PASTE /opt/buzz/compose/.env.agent
```

**Expect to see** `0`.

> **The two settings that cause silent failure.** Both of these are defaults, both look harmless,
> and both produce an agent that starts cleanly and then does nothing at all:
>
> - **`BUZZ_ACP_MCP_COMMAND` is empty by default.** `buzz-agent` has no built-in tools whatsoever —
>   its only way to post to Buzz is by running `buzz messages send` through a tool server. With no
>   tool server it accepts your message, calls the model, and has no way to answer. It looks like a
>   broken model.
> - **`BUZZ_ACP_AGENT_OWNER` is unset by default**, and the agent's default policy is to take
>   instructions from its owner only. An agent with no owner set ignores **everything**, from
>   everyone, forever. It looks like a broken relay.
>
> This is also why Step 12.4 told you to confirm the desktop app as the owner. The public key you
> just pasted must be the identity you are using in the app.

### 14.5 Give the agent a name

Right now the agent has a public key and nothing else — no name, no profile. That is a problem for
Step 16, because of how mentions work in Buzz.

**A mention is a tag on the message, not the text you typed.** When you type `@something` in the
desktop app and pick a suggestion from the dropdown, the app attaches that person's public key to the
message as a hidden tag. The agent's harness watches for messages carrying *its own key* in that tag.
Typed text that never resolved to a suggestion carries no tag at all — so the message is delivered to
the channel, is perfectly readable by humans, and is invisible to the agent.

With no name, the agent's only autocomplete entry is a truncated public key, which is not something
you can reasonably type. So give it a name first.

The next few commands need secret keys. **Keep them out of your shell history** — this is a machine
you have just spent Step 13 hardening, and a key in `~/.bash_history` or visible in `ps` undoes part
of that. Write them to a protected file once and reference the file:

```bash
# inside the VM
sudo tee /opt/buzz/compose/.keys.tmp >/dev/null <<'EOF'
AGENT_SECRET=<agent secret key from 14.2>
OWNER_SECRET=<owner secret key from 7.1>
EOF
sudo chmod 600 /opt/buzz/compose/.keys.tmp
sudo nano /opt/buzz/compose/.keys.tmp
```

Fill in the two real values, then confirm nothing is left:

```bash
# inside the VM
sudo grep -c '<' /opt/buzz/compose/.keys.tmp
```

**Expect to see** `0`.

Now read the two values into shell variables. Do this again if you reconnect — shell variables do not
survive a new SSH session:

```bash
# inside the VM
AGENT_SECRET=$(sudo sed -n 's/^AGENT_SECRET=//p' /opt/buzz/compose/.keys.tmp)
OWNER_SECRET=$(sudo sed -n 's/^OWNER_SECRET=//p' /opt/buzz/compose/.keys.tmp)
test -n "$AGENT_SECRET" && test -n "$OWNER_SECRET" && echo "both keys loaded"
```

**Expect to see** `both keys loaded`.

> **Delete this file at the end of Step 14.6** — it exists only for the commands in 14.5 and 14.6, and 14.6 ends by removing it:
> `sudo shred -u /opt/buzz/compose/.keys.tmp`

The command below runs from the sprig image, as the **agent's own identity** — a profile can only be
set by the person it belongs to:

```bash
# inside the VM
BUZZ_PRIVATE_KEY="$AGENT_SECRET" \
sudo --preserve-env=BUZZ_PRIVATE_KEY docker run --rm \
  --network buzz-prod_buzz-net \
  -e BUZZ_RELAY_URL=ws://buzz-vm.test:3000 \
  -e BUZZ_PRIVATE_KEY \
  --entrypoint /usr/local/bin/buzz \
  ghcr.io/block/buzz-sprig:main \
  users set-profile --name buzzbot
```

> **`-e BUZZ_PRIVATE_KEY` with no `=value` is deliberate.** Written that way, Docker copies the value
> from the environment rather than taking it as an argument. Writing `-e BUZZ_PRIVATE_KEY="$AGENT_SECRET"`
> would put the secret into the command's arguments, where anyone able to list running processes can
> read it for as long as the container runs — which would defeat the point of the key file. The
> `--preserve-env` is what carries the variable through `sudo`, which otherwise strips it.


**Expect to see** a JSON object containing `"accepted": true`.

Three things in that command are easy to get wrong:

| Part | Why it is that way |
|---|---|
| `--network buzz-prod_buzz-net` | The relay is only reachable from inside its own Docker network. `buzz-prod` is the project name set at the top of `compose.yml`; Docker prefixes network names with it |
| `ws://buzz-vm.test:3000` | Works because of the alias you added in Step 9.1. Without it this command cannot find the relay |
| `<agent secret key>` | The **secret** half from 14.2 — the agent's own. Not the owner's, not the relay's |

> **If this fails with a name-resolution error**, the alias from Step 9.1 is missing or the stack has
> not been restarted since you added it. Re-run the restart from Step 9.3 and try again.

### 14.6 Add the agent to the channel

Being approved on the relay (14.3) lets the agent connect. It does **not** let it see any channels.
The agent finds its channels by asking the relay which ones list it as a member, and subscribes only
to those. Creating `agent-test` in Step 12.5 made *you* a member, not the agent — so as things stand
the agent would start up, report zero channels, and never receive anything.

First find the channel's ID. Channels are identified internally by a UUID, not by their name:

```bash
# inside the VM
BUZZ_PRIVATE_KEY="$OWNER_SECRET" \
sudo --preserve-env=BUZZ_PRIVATE_KEY docker run --rm \
  --network buzz-prod_buzz-net \
  -e BUZZ_RELAY_URL=ws://buzz-vm.test:3000 \
  -e BUZZ_PRIVATE_KEY \
  --entrypoint /usr/local/bin/buzz \
  ghcr.io/block/buzz-sprig:main \
  channels list
```

**Expect to see** a JSON array with an entry for `agent-test`. Copy its **`channel_id`** — a long
value with dashes, like `d97ea868-3f1a-4c2e-9b7d-1e5a6c8f0b23`.

> **The field is `channel_id`, not `id`.** There is no `id` field in this output, so looking for one
> gets you nothing — and a `jq .id` returns `null`, which then becomes an empty `--channel` in the
> next command.

> **Note this command uses the *owner's* secret key**, not the agent's. Adding somebody to a channel
> is something only an existing member can do.

Now add the agent to it:

```bash
# inside the VM
BUZZ_PRIVATE_KEY="$OWNER_SECRET" \
sudo --preserve-env=BUZZ_PRIVATE_KEY docker run --rm \
  --network buzz-prod_buzz-net \
  -e BUZZ_RELAY_URL=ws://buzz-vm.test:3000 \
  -e BUZZ_PRIVATE_KEY \
  --entrypoint /usr/local/bin/buzz \
  ghcr.io/block/buzz-sprig:main \
  channels add-member --channel <channel uuid> --pubkey <agent public key> --role member
```

**Expect to see** a JSON object containing `"accepted": true`.

Confirm it took effect:

```bash
# inside the VM
BUZZ_PRIVATE_KEY="$OWNER_SECRET" \
sudo --preserve-env=BUZZ_PRIVATE_KEY docker run --rm \
  --network buzz-prod_buzz-net \
  -e BUZZ_RELAY_URL=ws://buzz-vm.test:3000 \
  -e BUZZ_PRIVATE_KEY \
  --entrypoint /usr/local/bin/buzz \
  ghcr.io/block/buzz-sprig:main \
  channels members --channel <channel uuid>
```

**Expect to see** both your own public key and the agent's public key listed. If the agent is not
there, do not continue — Step 16 cannot work.

Now delete the key file. It has done its job, and it currently holds the owner and agent secrets
together in one place on a machine you spent Step 13 hardening:

```bash
# inside the VM
sudo shred -u /opt/buzz/compose/.keys.tmp
test ! -e /opt/buzz/compose/.keys.tmp && echo "key file removed"
unset AGENT_SECRET OWNER_SECRET
```

**Expect to see** `key file removed`.

> **`shred -u` is not a guarantee here, and it is worth knowing why.** It removes the file and
> overwrites what it can reach, but this filesystem is journaled, sits on a dynamically-growing virtual
> disk, and is covered by VM snapshots — any of which may still hold a copy. On a throwaway dev VM that
> is acceptable; treat the real protection as "these keys only ever existed on a machine you are going
> to delete", and use fresh keys for anything that matters.
>
> The `unset` clears the two shell variables, which would otherwise stay readable in this session.
>
> **You will not need either secret again in this document** — the checklist in Step 17 is written to
> avoid them. Keep them in your password manager, as Step 7.1 and 14.2 told you to; if you ever need
> them here again, load them the same way 14.5 did.

> **Use `--role member`, not `--role bot`, even though `bot` exists and sounds correct.** This one
> word decides whether Step 16 can work at all.
>
> Buzz does have a `bot` role, and the relay treats it identically to `member` for posting. But the
> desktop app treats a `bot`-role member as an *agent*, and it only offers agents in the `@`
> suggestion list if they are registered in its agent directory. That directory is built from a kind
> of profile event that the agent harness never publishes. So a `bot`-role agent is silently dropped
> from the suggestion list — and with no suggestion to select, you cannot produce the mention tag
> that Step 16 depends on.
>
> `--role member` keeps the agent an ordinary channel member, which is what the suggestion list
> offers. If you have already added it as `bot`, remove and re-add it:
>
> ```bash
> # ... channels remove-member --channel <uuid> --pubkey <agent public key>
> # then add-member again with --role member
> ```

### 14.7 Add the agent container

As in Step 9.1, we add to our own file rather than editing the supplied one:

```bash
# inside the VM
sudo tee -a /opt/buzz/compose/compose.cohort.yml >/dev/null <<'EOF'

  agent:
    image: ghcr.io/block/buzz-sprig:main
    env_file:
      - .env.agent
    restart: unless-stopped
    depends_on:
      relay:
        condition: service_healthy
    networks:
      - buzz-net
    security_opt:
      - "no-new-privileges:true"
    cap_drop:
      - ALL
    mem_limit: 512m
    pids_limit: 256
EOF
```

> **The tag is `main`, not `latest`.** This image publishes no `latest` tag at all — asking for one
> gives a `manifest unknown` error and the container never starts.
>
> `main` is weaker than the careful pinning in Step 6.1, and knowingly so: it moves, so two people
> following this document a week apart can get different agent builds against the same relay.
>
> The image *does* publish immutable `sha-<7 characters>` tags, and they are ordinary Git commit SHAs
> from this same repository — so it is worth trying yours first, using the same check as Step 6.1 with
> `buzz-sprig` in place of `buzz`. It may not exist: this image only builds on commits that touched the
> agent code, so many relay commits have no matching sprig tag. If yours returns 404, use `main`.
>
> **Pin it as soon as you have a working pair.** Once the agent replies in Step 16, note the sprig
> digest you actually ran and put that in `compose.cohort.yml` in place of `main`:
>
> ```bash
> sudo docker inspect --format '{{index .RepoDigests 0}}' ghcr.io/block/buzz-sprig:main
> ```

Check the tag really is there before you rely on it:

```bash
# inside the VM
sudo docker pull ghcr.io/block/buzz-sprig:main
```

**Expect to see** the download complete and end with a `Status:` line. If you see `manifest unknown`,
the tag has changed — list what is available with:

```bash
# inside the VM
TOKEN=$(curl -s 'https://ghcr.io/token?scope=repository:block/buzz-sprig:pull&service=ghcr.io' \
  | sed -n 's/.*"token":"\([^"]*\)".*/\1/p')
curl -s -H "Authorization: Bearer $TOKEN" \
  https://ghcr.io/v2/block/buzz-sprig/tags/list
```

Now check the file still makes sense. `agent:` must line up with `relay:` under the same `services:`
heading, and Docker must be able to read the two files together:

```bash
# inside the VM
cat /opt/buzz/compose/compose.cohort.yml

cd /opt/buzz/compose
sudo docker compose --env-file .env -f compose.yml -f compose.cohort.yml config >/dev/null \
  && echo "compose files merge cleanly"
```

**Expect to see** the file contents, then `compose files merge cleanly`. An error here is a YAML
indentation problem in what you just appended — `agent:` needs exactly two spaces in front of it.

> **Why the limits.** The agent runs commands chosen by an AI model that is reading your channel
> messages. That is what makes it useful and it is also what makes it the riskiest thing on this
> machine — anyone who can get text in front of it is, indirectly, choosing what it runs. `cap_drop`,
> `no-new-privileges` and `mem_limit` are what keep a bad turn contained. On a machine with 1.9 GB of
> RAM, `mem_limit` in particular is what stops a runaway agent taking the relay down with it.
>
> **Never give this container the Docker socket**, no matter what a future guide suggests. That would
> hand the model root on the host.

---

## Step 15 — Start the agent and check it connected

> **Every command in this step needs both `-f` flags.** The agent is defined only in
> `compose.cohort.yml`, and Docker does **not** load that file on its own — that was a deliberate
> choice back in Step 9.1. A plain `docker compose logs agent` replies `no such service: agent`, which
> reads like the container failed when in fact Docker simply cannot see it. Every command below spells
> the files out. There is no shortcut, and adding one would hide the thing that is easiest to get
> wrong.

```bash
# inside the VM
cd /opt/buzz/compose
sudo docker compose --env-file .env -f compose.yml -f compose.cohort.yml up -d --wait
sudo docker compose --env-file .env -f compose.yml -f compose.cohort.yml logs agent | tail -40
```

**Expect to see** lines like these, in this order:

```
buzz-acp starting: ...
connected to relay at ws://buzz-vm.test:3000
subscribed to membership notifications
discovered 1 channel(s)
subscribed to channel <a uuid>
```

**The channel count is the line that matters.** It must be **at least 1**. If it says
`discovered 0 channel(s)` — or you see `no channel subscriptions resolved — agent will sit idle` —
then Step 14.6 did not take effect. The agent is connected, healthy, and will receive nothing, ever.
Nothing else in the system complains; you have to read this line.

| What the log shows | Meaning |
|---|---|
| `connected to relay at …` then `discovered 1 channel(s)` | Correct — continue |
| `discovered 0 channel(s)`, or `no channel subscriptions resolved` | The agent is not a member of `agent-test`. Redo 14.6, then restart the agent |
| a name-resolution or DNS error for `buzz-vm.test` | The Step 9.1 alias is missing, or the stack was not restarted after it was added |
| the container restarting over and over | Read the first error in the log; it happens before the restart |

> **Do not look for the word "authenticated"** — the harness never logs it, and the only place that
> word appears in its code is inside an error message. `connected to relay at …` is the line that
> tells you authentication succeeded.

A clean start does **not** prove the model works — nothing calls OpenRouter until the first mention.
Test that leg separately, because a bad key and a silent agent look identical from the outside.

First check the key itself and what credit it has:

```bash
# inside the VM
sudo docker compose --env-file .env -f compose.yml -f compose.cohort.yml exec agent sh -c \
  'curl -s -H "Authorization: Bearer $OPENROUTER_API_KEY" https://openrouter.ai/api/v1/key'
```

**Expect to see** a JSON object describing your key, including its usage and any limit you set.

Then make a real model call — this is the part that proves the model you chose can actually be used:

```bash
# inside the VM
sudo docker compose --env-file .env -f compose.yml -f compose.cohort.yml exec agent sh -c \
  'curl -s -w "\nHTTP %{http_code}\n" \
   -H "Authorization: Bearer $OPENROUTER_API_KEY" \
   -H "Content-Type: application/json" \
   -d "{\"model\":\"$OPENROUTER_MODEL\",\"messages\":[{\"role\":\"user\",\"content\":\"say ok\"}],\"max_tokens\":10}" \
   https://openrouter.ai/api/v1/chat/completions'
```

**Expect to see** a JSON reply containing a short message, followed by `HTTP 200`.

| Result | Meaning |
|---|---|
| `HTTP 200` with content | Key, credit, model id and network path are all good |
| `HTTP 401` | The key is wrong, or was pasted with a trailing space or newline |
| `HTTP 402` | No credit on the account. The agent fails these immediately — there is no retry to wait through |
| `HTTP 404` | `OPENROUTER_MODEL` is not a valid model id. Check it against OpenRouter's model list |
| hangs, or a DNS error | The machine cannot reach `openrouter.ai` |

> **Why two calls and not one.** Listing models only proves the key exists. It says nothing about
> whether you have credit, or whether the specific model you named in `.env.agent` can be called. A
> wrong model id passes a model-list check and then fails on every single mention.

Check memory, because the agent is new and this VM is deliberately small:

```bash
# inside the VM
free -m | sed -n '2,3p'
```

**Expect to see** swap still at **0**.

> **If swap is being used, do not give the VM more RAM.** Matching the production server's memory is
> the entire reason this VM exists — raising it hides exactly the problem you just found.
>
> Find out what is actually using the memory first:
>
> ```bash
> sudo docker stats --no-stream --format "{{.Name}}  {{.MemUsage}}"
> ```
>
> Compare against the figures in Step 8.3. If the agent is the outlier, that is a genuine capacity
> finding and belongs on the issue, not in a workaround. **Do not simply lower `mem_limit`** — the
> agent is already at the smallest configuration this document uses (`BUZZ_ACP_AGENTS=1`), so a
> tighter limit does not reduce its work, it just gets the container killed mid-turn, which looks
> like a completely different bug.

---

## Step 16 — Call the agent

This is the step everything else was for.

In the desktop app, signed in as the **owner** (Step 12.4), open the `agent-test` channel
(Step 12.5).

**Do this exactly, because the obvious way does not work:**

1. Type `@buzz` — just that much, and stop.
2. A suggestion list appears. `buzzbot` should be in it, from Step 14.5.
3. **Select `buzzbot` from the list** — click it, or press Enter/Tab on it. The name should now appear
   as a highlighted chip or pill rather than as ordinary text.
4. Then type the rest of your message: ` say hello`
5. Send it.

**Expect to see** a reply from `buzzbot` in the channel within a few seconds.

> **Why select rather than just type.** A mention is a hidden tag attached to the message, not the
> text you see, and the agent's harness only reacts to messages carrying that tag.
>
> Selecting from the list is the reliable way to attach it. Typing the name as plain text *also* works
> — the app scans the outgoing text for the names of channel members and tags any it recognises — but
> it depends on the text matching the member's name exactly, so a typo, a different capitalisation of a
> name that is not an exact member name, or a member who is not in this channel all produce a message
> that looks right to you and is invisible to the agent. Selecting removes that whole class of
> mistake, which is why this step tells you to do it.
>
> If `buzzbot` does not appear in the suggestion list at all, that is Step 14.5 or 14.6 not having
> worked — the app suggests channel members, so the agent must both have a name and be in the channel.
> The member list is also cached for about 30 seconds, so if you have only just finished 14.6, wait a
> moment or switch channels and back before concluding anything.

Watch the log at the same time if you want to see it work:

```bash
# inside the VM
cd /opt/buzz/compose
sudo docker compose --env-file .env -f compose.yml -f compose.cohort.yml logs -f agent
```

### If nothing happens

Work down this list in order. It is sorted by how often each one is the real cause.

| What you see | Cause | Fix |
|---|---|---|
| `buzzbot` is not in the suggestion list | Cached member list, or it has no name / is not a channel member | Wait ~30 s and switch channels and back. Still missing: redo 14.5, then 14.6 |
| **Nothing in the agent log at all** | Most likely you are not the owner, so it was dropped before the agent ever saw it. This is the single most common cause | Check `BUZZ_ACP_AGENT_OWNER` (14.4) against the identity you launched the app with (12.2). Then check the log did report 1+ channels |
| Nothing in the agent log, and the mention was not highlighted when you sent it | Possibly no mention tag — but only if the text did not match the member name exactly | Resend, selecting `buzzbot` from the list. If that also gets nothing, it is the owner gate above, not the tag |
| Log reported **0 channels** at startup | Not a channel member | Redo 14.6, then restart the agent |
| Log shows a turn running, then ending, but no message posted | No tool server, so no way to reply | Check `BUZZ_ACP_MCP_COMMAND=buzz-dev-mcp` (14.4) |
| Turn starts, then an error from the model | Key, credit, or model id | Re-run both `curl` checks in Step 15 |
| `no such service: agent` from any command | The two `-f` flags are missing | Use the full command form shown in Step 15 |
| Container restarting repeatedly | Read the **first** error in the log, before the restarts | Usually the relay name not resolving — Step 9.1 alias |

Once it replies, take a final snapshot from your own computer, so the checklist below has something to
restore that actually includes the agent:

```bash
VBoxManage controlvm buzz-dev acpipowerbutton
until VBoxManage showvminfo buzz-dev --machinereadable | grep -q '^VMState="poweroff"'; do sleep 3; done
VBoxManage snapshot buzz-dev take buzz-agent-working \
  --description "Buzz hardened, agent replying in agent-test"
VBoxManage startvm buzz-dev --type headless
```

---

## Step 17 — The checklist

The deployment is finished when every one of these passes. Run them in order; each assumes the ones
above it.

**Every item is labelled with where it runs** — `[VM]` inside the virtual machine as `dev`, `[HOST]` in
a terminal on your own computer, `[APP]` in the desktop app.

For the `[VM]` ones, get into the right place and define the prefix the rest of the list uses:

```bash
# inside the VM, logged in as dev
cd /opt/buzz/compose
DC="sudo docker compose --env-file .env -f compose.yml -f compose.cohort.yml"
```

Everything below written as `$DC` means that full command. Set it again if you open a new session —
shell variables do not survive reconnecting.

**The machine** — note that parity with production means **CPU, memory and swap**, not disk. The VM's
disk is deliberately smaller (Step 2.5), so storage is explicitly excluded from what this checklist
claims.

1. **[VM]** `free -h` shows 1.9 Gi memory and 496 M swap; `uname -m` shows `x86_64`
2. **[VM]** `$DC ps -a` shows `postgres`, `redis`, `minio` and `relay` healthy, `agent` **running**, and `minio-init` `Exited` — the `-a` matters, without it stopped containers are not listed at all
3. **[VM]** `$DC logs relay | grep -i community` shows host `buzz-vm.test:3000`
4. **[VM]** The WebSocket check in 13.8 returns `101`
5. **[VM]** A deliberately wrong `Host` returns `relay: no community is configured for this host`
6. **[VM]** `free -m` at idle shows swap at `0`

**Hardening** — this is the dev-VM subset, not the full production baseline; see the note at the end of
Step 13

7. **[VM]** `sudo sshd -T | grep permitrootlogin` shows `no`
8. **[VM]** `ls /etc/ssh/sshd_config.d/01-dev-parity.conf` reports no such file
9. **[HOST]** `ssh -p 2222 root@127.0.0.1` is refused
10. **[HOST]** `ssh -p 2222 dev@127.0.0.1` succeeds
11. **[VM]** `sudo ufw status` shows `Status: active` and a default of `deny (incoming)`
12. **[VM]** `sudo sysctl kernel.kptr_restrict` shows `2`
13. **[VM]** `sudo dockerd --validate` reports the configuration is OK, and `docker inspect` shows the relay on the `journald` driver (13.7)

**Browser and desktop**

14. **[HOST]** <http://buzz-vm.test:3000> renders the web bundle
15. **[HOST]** <http://admin.buzz-vm.test:3000/reports> renders the admin dashboard
16. **[APP]** The desktop app is connected and shows the community
17. **[HOST]** The `buzz-desktop: configured identity pubkey …` line in the launch terminal matches the owner public key from Step 7.1 (12.4)
18. **[APP]** The `agent-test` channel exists and you can post in it

**The agent**

19. **[VM]** `sudo ./run.sh list-members` includes the agent's public key
20. **[VM]** `$DC logs agent` shows `connected to relay at …` and `discovered 1 channel(s)` — zero channels is a failure. Do not grep for "authenticated"; it is never logged
21. **[VM]** The agent's channel membership exists in the database — no secret key needed:
    `$DC exec -T postgres psql -U buzz -d buzz -c "select 1 from events where kind=39002 and tags::text like '%<agent public key>%' limit 1;"` returns a row
22. **[APP]** `buzzbot` appears in the `@` suggestion list in `agent-test`
23. **[VM]** The OpenRouter key check and the chat-completion call in Step 15 both succeed
24. **[APP]** **@mentioning `buzzbot` in `agent-test` — selecting it from the suggestion list — produces a reply** ← the acceptance test
25. **[APP]** The reply arrives within 30 seconds
26. **[VM]** `free -m` during that turn still shows swap at `0`

**Reproducibility**

27. **[HOST]** In `~/code/buzz`: `git grep -nE 'sk-or-v1-|OPENROUTER_API_KEY=sk'` finds nothing. (This
    must run on the host — the VM has no Git repository, only the copied `compose` folder, so `git grep`
    there fails with "not a git repository". Also do **not** test this as "`git status` is clean":
    activating Hermit legitimately modifies `bin/pnpm`, so the tree is dirty for reasons that have
    nothing to do with secrets)
28. **[VM]** `test ! -e /opt/buzz/compose/.keys.tmp` passes, and
    `stat -c '%a' /opt/buzz/compose/.env /opt/buzz/compose/.env.agent` shows `600` for both
29. **[HOST]** then **[VM]/[APP]**: restore the `buzz-agent-working` snapshot from Step 16 (host), then
    re-run items 1–26 in their own environments and all still pass

> **Item 29 uses `buzz-agent-working`, not `buzz-hardened`.** `buzz-hardened` was taken in Step 13.8,
> before the agent identity, its channel membership and `.env.agent` existed — restoring it would
> throw all of that away and items 19–26 could not possibly pass.

If item 24 does not pass, the deployment is not done, however healthy everything else looks.

---

## Step 18 — Everyday operation

All of these run inside the VM, from `/opt/buzz/compose`.

> **You log in as `dev` now, not `root`** — Step 13.3 turned root login off. That is why every
> command below has `sudo` in front of it. Connect with `ssh -p 2222 dev@127.0.0.1`.

Set the prefix once per session, as in Step 17:

```bash
cd /opt/buzz/compose
DC="sudo docker compose --env-file .env -f compose.yml -f compose.cohort.yml"
```

| Task | Command |
|---|---|
| See what is running | `$DC ps` |
| Watch relay logs | `$DC logs -f relay` |
| Watch agent logs | `$DC logs -f agent` |
| Stop everything, keep data | `$DC stop` |
| Start again | `$DC up -d --wait` |
| Restart just the agent | `$DC restart agent` |
| Change the agent's model or key | `sudo nano .env.agent`, then `$DC up -d --force-recreate agent` |
| Approve someone on the relay | `sudo ./run.sh add-member <key> --role member` |
| List approved people | `sudo ./run.sh list-members` |

> **Do not use `./run.sh start` or `./run.sh stop` from here on.** `run.sh` has a fixed list of
> configuration files built in and does not know about `compose.cohort.yml`. Starting with it brings up
> the relay **without the `buzz-vm.test` network alias and without the agent**; stopping with it leaves
> the agent running as an orphan. The `$DC` form above always acts on the whole stack. `run.sh` is
> still the right tool for the membership commands, which do not depend on the file list.
>
> **`.env.agent` is owned by root with mode `600`**, so editing it needs `sudo` — plain
> `nano .env.agent` as the `dev` user will refuse to save.

The harness also has two owner control commands, `!cancel` and `!shutdown`, but they are **harder to
send from the desktop app than they look** — the message body has to be *exactly* the command and
nothing else, while the app inserts the mention as visible text, so what actually gets published is
`@buzzbot !cancel`. That does not match, and the agent treats it as an ordinary question instead.

Use the terminal for these instead:

| Task | Command |
|---|---|
| Stop the current turn | `$DC restart agent` |
| Stop the agent | `$DC stop agent` |

On your own computer:

| Task | Command |
|---|---|
| Shut the VM down | `VBoxManage controlvm buzz-dev acpipowerbutton` |
| Start the VM | `VBoxManage startvm buzz-dev --type headless` |
| Save a restore point | `VBoxManage snapshot buzz-dev take <name>` |
| Go back to one | `VBoxManage snapshot buzz-dev restore <name>` |
| List restore points | `VBoxManage snapshot buzz-dev list` |

You took four restore points along the way: `docker-clean` in Step 5.3, `buzz-working` in 13.1,
`buzz-hardened` in 13.8, and `buzz-agent-working` at the end of Step 16. Restoring one discards
everything the VM has done since. `buzz-working` is the one to reach for if a hardening change locks
you out; `buzz-agent-working` is the only one that has the agent in it.

### Deleting everything and starting over

```bash
VBoxManage controlvm buzz-dev poweroff
VBoxManage unregistervm buzz-dev --delete
```

This destroys the VM and all data in it. The secrets you saved to your password manager in Step 7
are the only things that survive.

---

## What this document has and has not been proven to do

Honesty here is worth more than confidence, because the automation is written against this
document.

**Verified by running it end to end on 2026-08-11**, on a VM with 1 CPU, 1.9 GiB RAM and 496 MB
swap, using `ghcr.io/block/buzz:sha-96ae141`:

- Docker from Ubuntu's archive: Docker 29.1.3, Compose 2.40.3, containerd 2.2.1
- All five containers reaching healthy, in 18 seconds, peaking at 563 MB with **swap never touched**
- Community created as `buzz-vm.test:3000`, port preserved, owner approved automatically
- WebSocket accepted (`101`) on the right address; refused (`404`,
  `relay: no community is configured for this host`) on wrong ones — including from the Mac
- `/health`, `/_liveness` and the relay information document returning `200` **regardless** of
  address, which is why Step 8.2 exists
- Web bundle serving at the root address with `BUZZ_SERVE_GIT_WEB_GUI=true`, and at
  `/invite/<code>` without it
- Admin dashboard serving at `/reports` and `/api/admin/v1/reports` on the admin address, and
  returning `403` on the ordinary one

All of that was on **macOS**, using `scp` for both file transfers (verified, including that dotfiles
like `.env.example` come across and that `run.sh` keeps its executable bit).

**Windows has not been run at all.** The Windows instructions were written from documented behaviour,
not from execution. Every **On Windows** block in this document is unverified, and these are the
places most likely to need correcting:

| Step | Windows-specific risk |
|---|---|
| 0.1 | Whether Git Bash alone is sufficient throughout, and the exact severity of the Hyper-V/VirtualBox interaction |
| 0.4 | The VirtualBox install path, if not the default |
| 0.6 | Node and pnpm via `corepack` instead of Hermit |
| 2.2 | Whether pasted Windows paths with backslashes survive Git Bash's path handling |
| 3.6 | Chocolatey plus `mkisofs`, and whether its flags match those given |
| 10 | Editing the hosts file as administrator |
| 12 | Visual Studio C++ Build Tools and WebView2 for the Tauri build |

Whoever runs this on Windows first: correct this document as you go, and change these notes to say it
has been verified.

**Not verified on either platform. Treat with suspicion:**

- **Step 12, the desktop app.** The `wss://` behaviour was confirmed by reading the source
  (`desktop/src/features/communities/communityStorage.ts:140`), but the app has not been built or
  launched, and no client has connected. The Tauri build may need Xcode command line tools that
  this document does not mention.
- **Step 11, approving members.** No key has been added and no connection by a non-owner has been
  attempted. Whether an unapproved key is actually refused — and what that refusal looks like — is
  untested. It happens during authentication, *after* the WebSocket connects, so a `101` alone does
  not prove membership is being enforced.
- **Steps 1–4, building the VM from scratch by these exact commands.** The VM used for verification
  was built by an earlier script that these steps were derived from. The commands correspond, but
  this sequence has not been run start to finish on a clean machine. Note that Step 2 was
  restructured to read paths from VirtualBox rather than assume them, which is a change from what the
  script did — so it is a step further from anything that has been executed.
- Nothing about disk capacity. This VM's disk is far smaller than production's.
- Nothing about behaviour under load. The relay allows up to 10,000 connections by default and
  buffers 1,000 messages per connection, so memory use under real concurrency is unmeasured and
  could look very different.

**Steps 13 to 17 have not been run at all.** They were written from the relay and agent source, and
from the hardening research in `../../Research/`. Everything from Step 13 onwards should be treated
the way the **On Windows** blocks are treated — documented behaviour, not execution. The specific
claims and where each came from:

| Step | Claim | Source |
|---|---|---|
| 13.3 | `01-dev-parity.conf` must be deleted, because sshd takes the first value and `01-` sorts first | The reasoning is this document's own, at Step 3.5, which chose the `01-` prefix for exactly that effect. The inversion follows, but has not been executed |
| 13.4 | `ufw` does not govern Docker's published ports | Widely documented Docker behaviour; not demonstrated on this VM |
| 13.7 | `disable-legacy-registry` stops the daemon starting; `ip-forward-no-drop` disables Docker 28's default drop | `moby/moby#35751`, Docker's v28 networking announcement |
| 14.4 | An empty `BUZZ_ACP_MCP_COMMAND` gives the agent no tools at all | `crates/buzz-acp/src/config.rs:256`, `src/lib.rs:4512`, and the test `empty_mcp_command_returns_no_servers` |
| 14.4 | `owner-only` is the default and an agent with no owner ignores everything | `crates/buzz-acp/README.md`, `src/config.rs:242` |
| 14.4 | `BUZZ_ACP_AGENT_COMMAND` defaults to `goose`, so it must be set | `crates/buzz-acp/README.md` configuration table. Note `BUZZ_ACP_AGENT_ARGS` needs no special handling — `default_agent_args` (`src/config.rs:694`) already resolves `buzz-agent` to no arguments, and `normalize_agent_args` (`:775`) maps a legacy lone `acp` to empty. An earlier draft of this document wrongly claimed a non-empty value would crash the container |
| 14.5, 14.6 | `buzz users set-profile`, `channels list`, `channels add-member` and `channels members` all exist with the flags used, and the CLI ships in the sprig image but **not** in the relay image | `crates/buzz-cli/src/lib.rs:510-524, 647-665, 822-837`; `Dockerfile:169-171` versus `Dockerfile.sprig:34-38` |
| 9.1 | The relay image already contains both front-end bundles and sets their directory variables, so no build or copy is needed | `Dockerfile:145-146` (the `COPY --from=web-builder` lines) and `Dockerfile:151-152` (the `ENV` block) |
| 14.6 | The Docker network is named `buzz-prod_buzz-net` | `name: buzz-prod` at the top of `deploy/compose/compose.yml`, confirmed with `docker compose config` |
| 14.4 | OpenRouter variable names and that a missing key is a hard startup error | `crates/buzz-agent/README.md`, `src/config.rs:834-842` |
| 14 | The sprig image carries `buzz-acp`, `buzz-agent`, `buzz-dev-mcp` and `buzz` as one binary, running non-root | `Dockerfile.sprig`, `scripts/sprig-entrypoint.sh` |

**Corrections found by review, now applied.** Each of these was wrong in an earlier draft in a way
that would have stopped a reader reaching Step 16, and each is recorded so nobody reintroduces it:

- **Step 13.3's hardening file is named `00-`, not `99-`.** The cloud image's own
  `60-cloudimg-settings.conf` sets `PermitRootLogin prohibit-password`, and first value wins — so a
  `99-` file loses to it and root login stays possible by key. The earlier draft fell into a variant of
  the very trap the step is written to warn about.
- **Step 14.6 uses `--role member`, not `--role bot`.** The desktop app treats a `bot`-role member as
  an agent and only offers agents that appear in its agent directory, which is built from kind:10100
  events. `buzz-acp` publishes no kind:10100 (confirmed: no reference to it anywhere in the crate), so
  a `bot`-role agent is silently dropped from the `@` suggestion list — and with no suggestion to
  select there is no mention tag, and no reply. An earlier draft asserted `bot` was the deliberate
  choice; it was exactly backwards.
- **Step 0.6 needs a non-Hermit path for pnpm on macOS.** pnpm dropped its Intel-macOS build at 11.0.5
  and the repository pins 11.x, while Step 0.2 guarantees every macOS reader is on Intel. Hermit fails
  with a "no source" error that never mentions architecture.
- **Step 12.1 creates the sidecar placeholder files, and 12.2 must NOT use `just desktop-standalone`.**
  Tauri validates its six `externalBin` sidecars at compile time and `desktop/src-tauri/binaries` is
  gitignored with nothing tracked, so a plain build on a fresh clone fails on a missing binary. But
  `desktop-standalone` — the recipe that would fix that — runs `unset BUZZ_PRIVATE_KEY
  BUZZ_SHARE_IDENTITY` immediately before launching (`Justfile:515`), which throws away the identity
  the next point depends on. An earlier draft used it and was self-defeating. Hence: stubs separately,
  then `tauri dev` directly. Windows has no `just` at all, so 12.1 gives the raw equivalent.
- **Step 12.2 supplies the owner key via `BUZZ_SHARE_IDENTITY`, read with `read -rs`.** `buzz-admin`
  prints keys as 64 hex characters, but the desktop sign-in form only accepts the `nsec1` form and
  nothing in this document converts between them, so pasting the key as printed leaves the button
  disabled. `read -rs` keeps the key off the screen and out of shell history.
- **Step 13.7 recreates the containers after writing `daemon.json`.** Those settings are defaults for
  *new* containers; the existing ones were created earlier and `live-restore: true` deliberately leaves
  them running untouched, so without a `--force-recreate` the hardening applies to nothing that is
  actually running.
- **Step 14.4 warns that the model must support tool calling.** The agent's only route to a reply is a
  tool call, so a non-tool-calling model answers into the void — and Step 15's connection test returns
  `200` for it regardless.
- **Step 17's secret check runs on the host, not in the VM.** Step 6 copied only `deploy/compose`, so
  there is no Git repository in the VM and `git grep` there fails outright.
- **Step 14.6's field is `channel_id`.** `buzz channels list` emits no `id` field.
- **Step 18 no longer documents `!cancel` / `!shutdown` as chat messages.** The harness requires the
  message body to equal the command exactly, but the desktop inserts the mention as visible text, so
  the published content is `@buzzbot !cancel` and never matches.

**Settled by review, and now written into the steps** — these were open questions in an earlier draft
and are recorded here so nobody re-opens them:

- **Channel membership is required, not optional.** `crates/buzz-acp/src/relay.rs:672` onwards
  (`discover_channels`) queries kind:39002 filtered by `#p` = the agent's pubkey, and returns an empty
  map when nothing matches. Relay membership alone gives the agent zero channels. This is why Step
  14.6 exists. The `buzz-acp` README's note about there being no channel-member API is out of date —
  `buzz channels add-member` exists (`crates/buzz-cli/src/lib.rs:653-665`) with a `bot` role.
- **A mention must carry a `p` tag**, but selecting from the list is not the *only* way to get one.
  `event_mentions_agent` (`crates/buzz-acp/src/lib.rs:3062`) matches only on a `p` tag holding the
  agent's pubkey, and the mention filter is on by default. The desktop attaches that tag either from a
  suggestion you selected **or** from a send-time scan of the text against the names of *channel
  members* — `extractMentionPubkeys` (`desktop/src/features/messages/lib/useMentions.ts:794-832`) skips
  non-members and then matches on a word boundary. So plain-typed `@buzzbot` does work once 14.6 has
  run. Two earlier drafts asserted flatly that it never works, and derived a troubleshooting row from
  that which sent readers chasing a non-cause. Step 16 still recommends selecting, because it removes
  the exact-name-match requirement — not because typing is inert.
- **The agent cannot resolve `buzz-vm.test` without help.** Container DNS does not consult the VM's or
  the host's `/etc/hosts`, and the relay rejects any other Host value. Hence the network alias in Step
  9.1, whose merge with `compose.yml`'s short-form `networks:` list was confirmed with
  `docker compose config`.
- **`BUZZ_API_TOKEN` is not needed.** `crates/buzz-relay/src/api/bridge.rs:80` accepts NIP-98
  (`Authorization: Nostr …`) regardless of `BUZZ_REQUIRE_AUTH_TOKEN`; that flag only governs the
  `X-Pubkey` development fallback. Both the harness and the CLI sign NIP-98 on every REST call. No
  token needs issuing, and this document deliberately does not issue one.
- **`ghcr.io/block/buzz-sprig` publishes no `latest` tag.** Confirmed against GHCR: `latest` returns
  404, `main` returns 200. Step 14.7 uses `main` and explains the trade-off. It *does* publish
  immutable `sha-<7>` tags, and — correcting an earlier draft — they are ordinary Git commit SHAs from
  this same repository, checkable with the same manifest `curl` as Step 6.1. The reason the Step 6.1
  value cannot simply be reused is that `.github/workflows/sprig-image.yml` is paths-filtered, so a
  sprig tag exists only for commits that touched the agent code — often, but not always, including the
  one you pinned. Worth checking before falling back to `main`.

**Still genuinely open:**

- **Which sprig build to pin.** `main` moves. Nobody has yet established a `sha-` tag known to work
  with a given relay build. Pin one as soon as a working pair is confirmed, and record both here.
- **Everything from Step 13 onwards remains unexecuted**, including all five fixes listed below. They
  are corrections derived from source, not from running the document.
- **Disk capacity**, unchanged: this VM's 20 GB disk is deliberately smaller than production's 49.5 GB,
  so nothing here can prove Buzz *fits* on the real server — only that it does not fit if it fails
  here.

Whoever runs Steps 13 to 17 first: correct this document as you go, move what you confirmed into the
verified list above, and delete it from here. That list is what tells the next reader how far to
trust each section, and it is only useful if it stays honest.

Deeper background on the address-matching rules, and the source references behind them, is in
[`relay-build-list.md`](./relay-build-list.md). The production-facing hardening this VM rehearses —
including the parts deliberately left out of Step 13 because they only matter on a public host — is
in [`hardening-spec.md`](./hardening-spec.md).

---

## Appendix — running your own front-end changes

Step 9 uses the front-end bundles that ship inside the relay image, which is what you want unless you
are **changing** the web or admin interface code. If you are, build your version and mount it over the
image's copy.

Build on your own computer, from the repository root:

```bash
cd ~/code/buzz
pnpm install
pnpm -C web build
pnpm -C admin-web build
```

> **Do not set `VITE_RELAY_URL` when building.** These tools bake configuration in at build time. Left
> unset, the bundle works out the relay address from whatever address the browser used, which is what
> you want. Setting it hard-codes one relay into the files permanently.

Copy the results in:

```bash
ssh -p 2222 dev@127.0.0.1 'sudo rm -rf /opt/buzz/web /opt/buzz/admin-web'
scp -P 2222 -r web/dist dev@127.0.0.1:/tmp/web
scp -P 2222 -r admin-web/dist dev@127.0.0.1:/tmp/admin-web
ssh -p 2222 dev@127.0.0.1 'sudo mv /tmp/web /opt/buzz/web && sudo mv /tmp/admin-web /opt/buzz/admin-web && sudo chown -R root:root /opt/buzz/web /opt/buzz/admin-web'
```

> The copy goes via `/tmp` because after Step 13 you log in as `dev`, who cannot write to `/opt`
> directly. The `rm -rf` first matters: copying onto an existing folder nests the files *inside* it as
> `/opt/buzz/web/dist`, and the relay then cannot find `index.html`.

Add the mounts to the `relay` service in `compose.cohort.yml` — alongside the `networks:` block that is
already there, not as a second `relay:` key:

```yaml
    volumes:
      - /opt/buzz/web:/srv/buzz/web:ro
      - /opt/buzz/admin-web:/srv/buzz/admin-web:ro
```

Then restart with the command from Step 9.3 and re-run the checks in Step 9.4.

> **One thing to be aware of.** You are building from your fork's current code while running a relay
> pinned to the upstream merge-base commit from Step 6.1. If the front-end and the relay have diverged,
> that mismatch is yours to reason about — it is the reason the main path no longer does this.
