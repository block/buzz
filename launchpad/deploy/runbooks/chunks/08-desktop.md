# Chunk 08 -- desktop app (MANUAL)

**What it does**

Builds the Tauri desktop app on your own Mac, launches it signed in as the relay owner, points it at
`wss://buzz-vm.test:8443`, and proves the identity it actually loaded is the owner. It is the only
chunk with no artifact to run: the app is a GUI, the build is interactive, and confirming ownership
means reading a line of terminal output — so this document is the chunk.

**SOP steps covered:** 12 (12.1, 12.2, 12.3, 12.4, 12.5), with 12.3's address as corrected by
hardening-spec.md §A.3 / Part G item 2 -- rationale lives there, not here

**Preconditions**

- **Chunk 07** has run: the stack is up with Caddy, and the community `buzz-vm.test:8443` exists.
- **Chunk 03, both jobs.** Job 1 makes the name resolve; job 2 (`--ca-only` plus the printed
  `security add-trusted-cert`) makes the host trust Caddy's `tls internal` root. Job 2 is a **hard**
  requirement here, not a nicety -- see Traps.
- **Chunk 09 is not required.** The owner pubkey in `.env` is approved automatically when the relay
  seeds the community (SOP Step 11), so the owner can connect before anyone else is rostered.
- **The owner secret key**, generated on the target by chunk 06 (SOP Step 7.1) and saved by you at
  that point -- a password manager, not a file in this repo. If you did not save it, look for
  `/opt/buzz/compose/.keys.tmp` on the VM before doing anything else: SOP Step 17 item 28 requires
  that file to be gone before the deployment counts as finished, so it is a one-time window.
- **Build tools, once per machine** (SOP Step 0.6 and Step 12 preamble): Xcode command line tools
  (`xcode-select --install`), Rust via `rustup`, and Node + pnpm. On this hardware pnpm needs SOP
  Step 0.6's Intel workaround -- the repo pins pnpm 11.x and pnpm dropped its Intel-macOS build at
  11.0.5, so either `hermit install pnpm-10.34.5` (it then upgrades itself) or the Hermit-free route,
  Node from nodejs.org plus `corepack enable pnpm`.
- macOS only. The chunked path is macOS-only by construction (chunk 00); SOP Step 12.1 carries the
  Windows sidecar-stub equivalent, including the `.exe` suffix that `just _ensure-sidecar-stubs`
  omits.

**Run**

```bash
cd /Users/jeff/group-build-project/buzz
. ./bin/activate-hermit

# 12.1 -- the six sidecar placeholders Tauri validates at compile time.
just _ensure-sidecar-stubs
ls desktop/src-tauri/binaries/

# 12.2 -- read the owner SECRET key without echoing it or storing it in history.
# Written the long way on purpose: `read -rsp` is a bashism (see Traps).
printf 'Owner secret key: '; stty -echo; read -r BUZZ_PRIVATE_KEY; stty echo; echo
export BUZZ_PRIVATE_KEY BUZZ_SHARE_IDENTITY=1
[ ${#BUZZ_PRIVATE_KEY} -eq 64 ] && echo "key loaded, 64 characters" || echo "WRONG LENGTH: ${#BUZZ_PRIVATE_KEY}"

# Build and launch. The first run compiles Rust and takes several minutes.
pnpm install
pnpm -C desktop tauri dev
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
after `RELAY_OWNER_PUBKEY=`.

| What you see | Meaning |
|---|---|
| The line, matching | Correct -- continue |
| The line, different key | Wrong secret pasted. Quit and relaunch |
| `buzz-desktop: invalid BUZZ_PRIVATE_KEY: …` | Mistyped or truncated; the app silently fell back to another identity. Relaunch |
| No such line at all | The variables never reached the app. Check `export` in 12.2, relaunch |

The remaining checks are SOP Step 17 items 16-18, and are what chunk 11 cannot assert for you:

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
- `tauri dev` uses the standard app identifier and keyring, so it can share stored data with another Buzz install on this Mac -- `desktop-standalone` is what isolates those, and it is unusable here (SOP 12.2).
- The `agent-test` name comes from SOP Step 16, which no chunk implements (chunks stop at Step 13). The channel is still wanted, for checklist item 18 -- the name is inherited, not meaningful here (`README.md` Scope).
- Posting failures mean the relay is refusing you, not that the app is broken: connection succeeds first and authentication happens after it, so a connected app is no proof of an approved key (SOP 12.5, Step 11).
- Nothing in Step 12 has ever been executed -- the app has not been built or launched by anyone on this path, and the `wss://` handling was read from `desktop/src/features/communities/communityStorage.ts:140` (SOP, "not verified on either platform").
