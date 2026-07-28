# Desktop Local macOS Signing

Local Tauri builds are ad-hoc signed by default. macOS Keychain then trusts the
exact `buzz-desktop` binary hash, so a rebuild can bring back the Keychain
prompt even after selecting "Always Allow".

Buzz keeps the reusable fix in the repo:

- `desktop/scripts/install-local-macos-app.sh` signs and installs `Buzz.app`.
- `just desktop-install-local-macos` runs the installer.
- `just desktop-signing-status` verifies the installed signature and Keychain
  readiness.

## First Setup On A Mac

Build the app, create the local signing identity once, install, then verify:

```bash
. ./bin/activate-hermit
just desktop-release-build
just desktop-install-local-macos --create-identity
just desktop-signing-status
```

The first launch may still show one Keychain prompt for `buzz-desktop`. Choose
"Always Allow" once. Later local installs signed with the same identity should
reuse the same Keychain authorization.

## Later Local Rebuilds

After the identity exists on that machine:

```bash
. ./bin/activate-hermit
just desktop-release-build
just desktop-install-local-macos
just desktop-signing-status
```

## New Computer

Use the same first-setup flow on the new Mac. This creates a new local signing
identity on that computer, then you approve `buzz-desktop` in Keychain once.

If the login Keychain was migrated from an old Mac and you want the exact same
certificate leaf hash, export/import the `Buzz Local Code Signing` identity
manually as an encrypted `.p12` and keep it outside the repo. Do not commit
`.p12`, `.pfx`, `.pem`, or private-key files.

## Expected Healthy Status

`just desktop-signing-status` should report:

```text
signature=valid
keychain_acl_stability=stable
local_signing_identity=present
```

The designated requirement must look like:

```text
designated => identifier "xyz.block.buzz.app" and certificate leaf = H"..."
```

If it says `designated => cdhash`, the installed app is still ad-hoc signed and
Keychain prompts can return after each rebuild.
