# Chunk 08 -- desktop app (MANUAL)

**What it does**

Builds the Tauri desktop app on your own Mac, launches it signed in as the relay owner, points it at
`wss://buzz-vm.test:8443`, and proves the identity it actually loaded is the owner. It is the only
chunk with no artifact to run: the window cannot be driven from a CLI (see Traps for the three
concrete reasons), and confirming ownership means reading a line of terminal output — so this
document is the chunk.

**SOP steps covered:** 12 (12.1, 12.2, 12.3, 12.4, 12.5), with 12.3's address as corrected by
hardening-spec.md §A.3 / Part G item 2 -- rationale lives there, not here

**Preconditions**

- **Chunk 07** has run: the stack is up with Caddy, and the community `buzz-vm.test:8443` exists.
- **Chunk 03, both jobs.** Job 1 makes the name resolve; job 2 (`--ca-only` plus the printed
  `security add-trusted-cert`) makes the host trust Caddy's `tls internal` root. Job 2 is a **hard**
  requirement here, not a nicety -- see Traps.
- **Chunk 09 is not required.** The owner pubkey in `.env` is approved automatically when the relay
  seeds the community (SOP Step 11), so the owner can connect before anyone else is rostered.
- **The owner secret key.** On the chunked path chunk 06 leaves it at `/root/owner-key.txt` on the VM
  (mode 0600), which is what the primary Run command below reads; on the SOP's manual path it is
  `/opt/buzz/compose/.keys.tmp` (SOP Step 7.1). Either way it belongs in a password manager, not in
  this repo, and SOP Step 17 item 28 requires the `.keys.tmp` copy to be gone before the deployment
  counts as finished -- so if you are on the manual path that is a one-time window.
- **Build tools, once per machine** (SOP Step 0.6 and Step 12 preamble): Xcode command line tools
  (`xcode-select --install`), Rust via `rustup`, `cmake` via `brew install cmake`, and Node + pnpm.
- **Do not activate Hermit for this chunk.** Verified 2026-08-12 on an Intel Mac: Node v22.23.1 and
  pnpm 11.4.0 already outside Hermit are enough (`pnpm install` reports `Already up to date`), while
  Hermit would swap in the pinned pnpm 11.x that has no Intel-macOS build (SOP Step 0.6). Two
  consequences, both handled in Run: Hermit also supplies `just`, so check the sidecar stubs by hand
  instead of running `just _ensure-sidecar-stubs`; and Hermit also supplies `cmake`, so install it
  from Homebrew -- it is a **hard** requirement, not an optional extra (see Traps).
- macOS only. The chunked path is macOS-only by construction (chunk 00); SOP Step 12.1 carries the
  Windows sidecar-stub equivalent, including the `.exe` suffix that `just _ensure-sidecar-stubs`
  omits.

**Run**

```bash
cd /Users/jeff/group-build-project/buzz

# Put cargo on PATH in THIS shell. rustup only edits ~/.zshenv and ~/.profile,
# which do not reach a shell that was already open (see Traps).
. "$HOME/.cargo/env"

# 12.1 -- the six sidecar placeholders Tauri validates at compile time.
# Check before building them: on 2026-08-12 all six were already present, and
# `just _ensure-sidecar-stubs` needs Hermit, which this chunk does not activate.
ls desktop/src-tauri/binaries/
# expect six: buzz-, buzz-acp-, buzz-agent-, buzz-backend-kubernetes-,
# buzz-dev-mcp-, git-credential-nostr- (each suffixed -x86_64-apple-darwin).
# Only if they are missing: . ./bin/activate-hermit && just _ensure-sidecar-stubs

# 12.2 -- load the owner SECRET key straight off the VM. It is never displayed,
# never enters the clipboard, and never enters shell history.
export BUZZ_PRIVATE_KEY=$(ssh -p 2222 dev@127.0.0.1 'sudo sed -n "s/^secret *//p" /root/owner-key.txt' | tr -d '[:space:]')
export BUZZ_SHARE_IDENTITY=1
[ ${#BUZZ_PRIVATE_KEY} -eq 64 ] && echo "key loaded, 64 characters" || echo "WRONG LENGTH: ${#BUZZ_PRIVATE_KEY}"

# Build and launch. The first run compiles Rust: 6m25s measured on 2026-08-12.
pnpm install
pnpm -C desktop tauri dev
```

Fallback, for when the key lives in a password manager rather than on the VM. Written the long way
on purpose: `read -rsp` is a bashism (see Traps).

```bash
printf 'Owner secret key: '; stty -echo; read -r BUZZ_PRIVATE_KEY; stty echo; echo
export BUZZ_PRIVATE_KEY BUZZ_SHARE_IDENTITY=1
[ ${#BUZZ_PRIVATE_KEY} -eq 64 ] && echo "key loaded, 64 characters" || echo "WRONG LENGTH: ${#BUZZ_PRIVATE_KEY}"
```

To get the key *into* that password manager without ever displaying it, pipe the same `ssh` command
into the clipboard and clear it straight after pasting:

```bash
ssh -p 2222 dev@127.0.0.1 'sudo sed -n "s/^secret *//p" /root/owner-key.txt' | tr -d '\n' | pbcopy
# paste into the password manager, then:
printf '' | pbcopy
```

Then, in the app (SOP 12.3, 12.5):

1. Choose **Join a community** -- or **I already have a community** then **I'm a member or admin**.
2. In the single address field type exactly `wss://buzz-vm.test:8443`. Leave any invite code empty.
3. Work through the display-name and welcome screens until you reach the channel list.
4. Create a channel called `agent-test` and post a message in it.

**Verify**

The owner check is a comparison between two terminal outputs. Both sides are **public** keys, so
neither is a secret.

```bash
# Left side: what the app actually loaded. In the terminal you launched from.
# (scroll back through the startup output)
#   buzz-desktop: configured identity pubkey <64 hex>

# Right side: what the relay was configured with. RELAY_OWNER_PUBKEY is public.
ssh -p 2222 dev@127.0.0.1 'sudo grep ^RELAY_OWNER_PUBKEY /opt/buzz/compose/.env'
```

Expected: the 64 hex characters after `configured identity pubkey` are **identical** to the value
after `RELAY_OWNER_PUBKEY=`. Executed on 2026-08-12: the line appears and the two values matched.

| What you see | Meaning |
|---|---|
| The line, matching | Correct -- continue |
| The line, different key | Wrong secret pasted. Quit and relaunch |
| `buzz-desktop: invalid BUZZ_PRIVATE_KEY: …` | Mistyped or truncated; the app silently fell back to another identity. Relaunch |
| No such line at all | The variables never reached the app. Check `export` in 12.2, relaunch |

The remaining checks are SOP Step 17 items 16-18, and are what chunk 11 cannot assert for you.
**None of them has been executed yet** — as of 2026-08-12 the build, the launch and the owner match
above are proven, and everything below this line is documented behaviour only:

- the app is connected and shows the community
- the `agent-test` channel exists
- a message posted in it appears, with no error

**Rollback**

Quit the app and relaunch it with the correct key. `BUZZ_PRIVATE_KEY` takes precedence over any
identity saved from a previous run, so a wrong-identity launch is fully recovered by relaunching --
there is nothing to clean up on the host.

No VM snapshot applies. The only guest-side writes this chunk makes are the channel and the messages
you post, both of which are ordinary relay events. If you do restore a snapshot for some other
reason, restore `buzz-working` (SOP 13.1) and repeat this chunk; the desktop side needs no rebuild,
only a relaunch.

**Traps**

- The address is `wss://buzz-vm.test:8443`, **not** the SOP's literal `ws://buzz-vm.test:3000`: the chunks run Caddy with `tls internal` and `compose.caddy.yml`'s `ports: !reset []` deletes the relay's published 3000 entirely (hardening-spec.md §A.3, §B2).
- SOP 12.3's "you must type `ws://`" warning is **inverted** on this path -- it applies to the plaintext path only, and that SOP edit is still owed (hardening-spec.md Part G items 1-2).
- `:8443` is the part that gets dropped by muscle memory: the relay strips only a trailing `:443` or `:80`, so the community really is `buzz-vm.test:8443` and a bare `wss://buzz-vm.test` both mismatches the community and aims at a host port nothing forwards (SOP Step 7.2; `ansible/inventory/group_vars/dev.yml`).
- Without chunk 03's job 2 the connection is simply refused: Tauri's Rust TLS stack validates against the system keychain and offers no "proceed anyway", so a self-signed `tls internal` CA fails with an error that never mentions certificates (chunk 03 Traps; SOP 12.3 as corrected).
- **Do not use `just desktop-standalone`.** It builds the sidecars properly and then runs `unset BUZZ_PRIVATE_KEY BUZZ_SHARE_IDENTITY` immediately before launching (`Justfile:515`), so the identity is discarded and you land on a sign-in screen you cannot get past (SOP 12.2).
- `read -rsp "…" VAR` fails in zsh -- `-p` means "read from a coprocess" there, so you get `read: -p: no coprocess`, the variable is never set, and the app launches on a throwaway identity (SOP 12.2).
- **Do not choose "Create a community" or "I own the community."** Both open a sign-in for Block's hosted service and neither offers an address field; being told at length that you are the owner is exactly what makes the wrong button the obvious one (SOP 12.3).
- **Do not click "Create a new identity key"** if offered -- it mints a different identity, and the failure is silent (SOP 12.2).
- Do not compare identities using the app's profile display: it shows `npub1…` and nothing on this path converts between the two forms, so the comparison is not a check you can actually perform (SOP 12.4).
- Reaching the community without a sign-in prompt proves nothing -- a malformed key makes the app reuse a previously saved identity, which also skips that screen (SOP 12.4).
- Skipping 12.1 fails the build with a missing `binaries/buzz-acp-…` file, which reads like a corrupt checkout rather than a missing build step; `desktop/src-tauri/binaries` is gitignored with nothing tracked (SOP 12.1).
- **The shared keyring is the most likely reason this chunk fails, and it does not look like an identity problem.** `tauri dev` uses the standard app identifier and the shared `buzz-desktop-dev` macOS keyring, so on a Mac with any previous Buzz install the app can authenticate as a *saved* identity and reconnect a *saved* community — while still logging your `BUZZ_PRIVATE_KEY` owner key as the "configured identity". `BUZZ_PRIVATE_KEY` sets the configured identity; it does **not** override a stored community's identity. Confirmed 2026-08-12: startup logged the owner pubkey, the relay then refused a different pubkey with `relay_membership_required` (SOP 12.2, 12.4).
- Diagnose that case from the **relay**, not the app: `$DC logs relay | grep -E 'not a relay member|WebSocket connection'` shows the pubkey that actually authenticated. If it is not `RELAY_OWNER_PUBKEY`, the app is using a stored identity and no amount of relaunching with the right key will change it.
- The app's data directory is `~/Library/Application Support/xyz.block.buzz.app` and is shared for the same reason; identities live in the **macOS keyring**, not on disk, because the dev build runs `--features system-keyring`. An empty-looking data dir is not an empty state.
- **The fix is one export, and it is documented nowhere else.** `BUZZ_DEV_KEYRING_SERVICE` overrides the keyring service name (`desktop/src-tauri/src/app_state_keyring.rs:13`); any value starting with `buzz-desktop-dev.` is accepted, anything else silently falls back to the shared default. Setting it gives this run its own keyring namespace and leaves an existing install's entry untouched:
  ```bash
  export BUZZ_DEV_KEYRING_SERVICE=buzz-desktop-dev.buzzvm
  ```
  Verified 2026-08-12: with it set, the app authenticated as `RELAY_OWNER_PUBKEY` and the relay logged no refusals; without it, the same shell authenticated as a stale identity. This is preferable to `desktop-standalone` (which unsets `BUZZ_PRIVATE_KEY`) and to deleting keychain entries.
- Note what it does **not** namespace: the webview's `localStorage`, which holds the community list (`buzz-communities`) and lives at `~/Library/WebKit/buzz-desktop` — keyed by PROCESS NAME, so every dev build shares it. If a stale community still appears, leave it in the UI and add the correct one; moving that directory aside is a last resort because it takes any other install's app state with it.
- The `agent-test` name comes from SOP Step 16, which no chunk implements (chunks stop at Step 13). The channel is still wanted, for checklist item 18 -- the name is inherited, not meaningful here (`README.md` Scope).
- Posting failures mean the relay is refusing you, not that the app is broken: connection succeeds first and authentication happens after it, so a connected app is no proof of an approved key (SOP 12.5, Step 11).
- **`cmake` is a hard requirement and nothing warns you up front**: `aws-lc-sys` (in both `Cargo.lock` and `desktop/src-tauri/Cargo.lock`, alongside `zstd-sys` and `libsqlite3-sys`) builds AWS-LC through CMake, so without Hermit the build dies deep inside a C compile rather than at a dependency check -- `brew install cmake` (verified: cmake 4.4.2 at `/usr/local/bin/cmake`).
- **`cargo metadata … No such file or directory (os error 2)` is a PATH problem, not a broken Tauri or cargo**: rustup writes `~/.zshenv` and `~/.profile`, which only affect *new* shells, so a shell or tmux pane opened before rustup was installed has no `~/.cargo/bin` -- `. "$HOME/.cargo/env"` in that shell (Run, step 12.2 preamble).
- `rustc --version` reports **1.95.0 inside the repo and 1.97.1 outside it**, because `rust-toolchain.toml` pins `channel = "1.95.0"` and rustup fetches it automatically; the mismatch is benign and needs no action (SOP Step 0.6).
- Typing the key by hand is the error-prone path, which is why the `ssh`-into-`export` form is primary: a wrong-length key does not stop the launch, the app falls back to another identity silently (SOP 12.2; Verify table).
- **The window cannot be driven by GUI automation**: Tauri renders in a WKWebView, whose macOS accessibility tree typically collapses to a single `AXWebArea` with no named controls, so any script is reduced to blind coordinate clicking (`README.md` chunk map, row 08 "GUI -- not scriptable").
- **Playwright cannot substitute for it either**, though it is installed (`desktop/node_modules/.bin/playwright`) with a large spec suite: `installE2eBridgeIfConfigured` (`desktop/src/main.tsx:100`) compiles in only under `--mode e2e` and fakes Tauri IPC, so the specs can never reach a real relay -- and opening the Vite dev server at `localhost:1420` in a plain browser hits the same wall (CLAUDE.md, "Writing E2E Screenshot Specs").
- Driving the real window would additionally need macOS TCC grants (Privacy & Security -> Accessibility, and Screen Recording) that cannot be granted from a CLI, so relay-side verification belongs in `buzz-cli`, not in GUI automation -- note `buzz-cli` is not built yet: `cargo build --release -p buzz-cli` (CLAUDE.md, "Agent CLI").
- Notifications are silently disabled under `tauri dev` because it runs from `target/debug/buzz-desktop` rather than an app bundle; macOS grants notification permission per bundle, so this is expected here and not a defect (observed 2026-08-12).
- The data directory really is the shared `~/Library/Application Support/xyz.block.buzz.app` -- printed at startup, confirming the warning above that `tauri dev` shares stored data with any other Buzz install on this Mac (SOP 12.2).
- **Step 12 is only half executed.** Proven on 2026-08-12: the build (6m25s), the launch, and the owner-identity match. **Not proven:** that the app connects to `wss://buzz-vm.test:8443` at all, that Tauri accepts the `tls internal` certificate, or that channel creation and posting work -- what chunk 03 established is only that *the host* trusts the CA at the `curl` level, and the `wss://` handling is still read from `desktop/src/features/communities/communityStorage.ts:140` (SOP, "not verified on either platform").
