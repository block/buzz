# Dev deployment SOP — from nothing to a working Buzz environment

**Audience:** you have a **Mac or a Windows PC**, a terminal, and no particular experience with
virtual machines, Bash, or networking. Every command is written out. Nothing is hidden in a script.

**What you will have at the end:** a virtual machine on your own computer running the Buzz relay,
Postgres, Redis and MinIO, reachable from your browser; the admin dashboard; the web bundle served
by the relay; and the desktop app connected to it.

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
| **shell** | The program that reads your typed commands. Bash is the one used here |

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

Check it works:

```bash
bash --version
```

**Expect to see** a line beginning `GNU bash, version 5`. The exact version does not matter.

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

```bash
# inside the VM
ssh -p 2222 root@127.0.0.1

apt-get update
DEBIAN_FRONTEND=noninteractive apt-get install -y \
  docker.io docker-compose-v2 containerd
```

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
sleep 30
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
production stack. That is all the VM needs — about 44 KB. The relay itself arrives later as a
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

```bash
curl -s -o /dev/null -w "%{http_code}\n" \
  -H "Authorization: Bearer $(curl -s 'https://ghcr.io/token?scope=repository:block/buzz:pull&service=ghcr.io' | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')" \
  -H "Accept: application/vnd.oci.image.index.v1+json" \
  https://ghcr.io/v2/block/buzz/manifests/sha-96ae141
```

**Expect to see** `200`. If you see `404`, that commit has no published image; try the commit
before it with `git log --first-parent -5 <the-commit>` and repeat.

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

**Expect to see** eight or nine files including `compose.yml`, `.env.example` and `run.sh`, and a
total size around `44K`. `run.sh` must show an `x` in its permissions.

---

## Step 7 — Create the configuration and secrets

This is the step where mistakes are most expensive, so it is broken down finely.

Log into the VM and start from the supplied example:

```bash
# inside the VM
ssh -p 2222 root@127.0.0.1
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
IMG=ghcr.io/block/buzz:sha-96ae141
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
INFO Deployment community ensured  host="buzz-vm.test:3000" community=d97ea868-...
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

## Step 9 — Add the browser front-ends

Two web interfaces exist. The relay serves both from folders, so we build them on your own computer
and copy the results in.

| Interface | What it is | Address |
|---|---|---|
| **Web bundle** | Invite landing pages, and optionally a repository browser | `buzz-vm.test:3000` |
| **Admin dashboard** | **Read-only** view of moderation reports and product feedback | `admin.buzz-vm.test:3000` |

> **The admin dashboard is not a control panel.** It cannot add users, change settings, or manage
> the roster. It only displays reports and feedback. Adding people is done from the command line in
> Step 11.

### 9.1 Build both on your own computer

```bash
cd ~/code/buzz
```

**On macOS only**, activate the pinned toolchain (Windows users installed Node directly in 0.6 and
skip this line):

```bash
. ./bin/activate-hermit
```

Then, on both platforms:

```bash
pnpm install
pnpm -C web build
pnpm -C admin-web build
```

`pnpm install` downloads the build tools. It only needs doing once — it covers both interfaces and
the desktop app in Step 12 — but on a fresh clone the builds cannot run without it. Expect it to
take a few minutes the first time.

**Expect to see** each build finishing with `✓ built in ...` and a list of output files.

> **Do not set `VITE_RELAY_URL` when building.** These tools bake configuration into the files at
> build time. Left unset, the web bundle works out the relay address from whatever address the
> browser used — which is what we want. Setting it would permanently hard-code one relay into the
> files.

### 9.2 Copy the results into the VM

From the repository root:

```bash
ssh -p 2222 root@127.0.0.1 'rm -rf /opt/buzz/web /opt/buzz/admin-web'

scp -P 2222 -r web/dist root@127.0.0.1:/opt/buzz/web
scp -P 2222 -r admin-web/dist root@127.0.0.1:/opt/buzz/admin-web

ssh -p 2222 root@127.0.0.1 'chown -R root:root /opt/buzz/web /opt/buzz/admin-web && ls -l /opt/buzz/web/index.html /opt/buzz/admin-web/index.html'
```

The `rm -rf` first matters. Copying onto a folder that already exists would nest the new files
*inside* it as `/opt/buzz/web/dist`, and the relay would not find `index.html` where it expects.

**Expect to see** two `index.html` files listed. Both must exist — the relay refuses to start if the
admin folder has no `index.html`.

### 9.3 Tell Docker to make those folders visible to the relay

The supplied `compose.yml` does not know about these folders, and **we must not edit it** — it
belongs to the upstream project and every local change becomes a merge conflict later. Instead add a
separate file alongside it:

```bash
# inside the VM
cat > /opt/buzz/compose/compose.cohort.yml <<'EOF'
# Our own additions. Deliberately NOT named compose.override.yml: Docker loads
# that name automatically for some commands but not others, which would give
# two different stacks depending on which command you ran. This name is never
# loaded automatically, so it is always explicit.
services:
  relay:
    volumes:
      - /opt/buzz/web:/srv/buzz/web:ro
      - /opt/buzz/admin-web:/srv/buzz/admin-web:ro
EOF
```

`:ro` means read-only — the relay can read these files but not change them.

### 9.4 Then add the settings

Order matters. The folders must exist and be visible **before** these settings are applied, or the
relay will fail to start with a configuration error.

```bash
# inside the VM
cat >> /opt/buzz/compose/.env <<'EOF'

# Relay-served front-ends. These paths are inside the container, provided by
# compose.cohort.yml.
BUZZ_WEB_DIR=/srv/buzz/web
BUZZ_ADMIN_HOST=admin.buzz-vm.test:3000
BUZZ_ADMIN_WEB_DIR=/srv/buzz/admin-web
BUZZ_SERVE_GIT_WEB_GUI=true
EOF
```

`BUZZ_SERVE_GIT_WEB_GUI=true` is what makes the web bundle appear at the root address. Without it
the bundle only answers invite links.

### 9.5 Restart, using both files

```bash
# inside the VM
cd /opt/buzz/compose
docker compose --env-file .env -f compose.yml -f compose.cohort.yml up -d --wait
```

> **Why not `./run.sh restart` here?** `run.sh` has a fixed list of configuration files built in
> and no way to add ours, so it would start the relay without the front-end folders. From this
> point on, use the longer command above whenever you restart. Note that `run.sh` also checks for
> unfilled secrets and this command does not, so re-run the check from Step 7.4 yourself if you
> have edited `.env`.

**Expect to see** every container reporting `Healthy`.

### 9.6 Verify all four surfaces

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
> not set — revisit 9.4. The same address deliberately serves different things to different
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

Both also need Rust, which Tauri's page covers via `rustup`.

Then:

```bash
cd ~/code/buzz
```

**On macOS only:**

```bash
. ./bin/activate-hermit
```

Then on both platforms:

```bash
pnpm install
pnpm -C desktop tauri dev
```

The first run compiles Rust components and is slow — expect several minutes, possibly much longer on
a first-ever Rust build. Subsequent runs are far faster.

**Expect to see** a desktop window open.

### 12.1 Point it at your relay — type the address in full

In the app, add a community or relay, and enter **exactly**:

```
ws://buzz-vm.test:3000
```

> **You must type `ws://` at the front.** If you leave the scheme off, the app assumes `wss://`,
> which means an encrypted connection. This dev relay does not use encryption, so the connection
> will fail with an error that does not explain why. Production will use `wss://`; this VM does not.

**Expect to see** the app connect and show the community.

If it does not connect, check in order: the relay is healthy (Step 8), you typed `ws://`, you typed
the port `:3000`, and your key has been approved (Step 11).

---

## Step 13 — Everyday operation

All of these run inside the VM, from `/opt/buzz/compose`.

| Task | Command |
|---|---|
| See what is running | `docker compose ps` |
| Watch relay logs | `./run.sh logs` |
| Stop everything, keep data | `./run.sh stop` |
| Start again | `docker compose --env-file .env -f compose.yml -f compose.cohort.yml up -d --wait` |
| Approve someone | `./run.sh add-member <key> --role member` |
| List approved people | `./run.sh list-members` |

On your own computer:

| Task | Command |
|---|---|
| Shut the VM down | `VBoxManage controlvm buzz-dev acpipowerbutton` |
| Start the VM | `VBoxManage startvm buzz-dev --type headless` |
| Save a restore point | `VBoxManage snapshot buzz-dev take <name>` |
| Go back to one | `VBoxManage snapshot buzz-dev restore <name>` |

You took the `docker-clean` restore point in Step 5.3. `VBoxManage snapshot buzz-dev list` shows
what you have; restoring one discards everything the VM has done since.

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

Deeper background on the address-matching rules, and the source references behind them, is in
[`relay-build-list.md`](./relay-build-list.md).
