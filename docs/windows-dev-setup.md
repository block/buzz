# Windows Dev Setup

How to get a working Flutter/Dart toolchain for this repo on Windows, where
hermit does not run.

## Why hermit does not work here

Every toolchain in `bin/` is a hermit shim: `bin/flutter` is a symlink to
`bin/.flutter-3.41.7.pkg`, which is a symlink to `bin/hermit`. Running either
`bin/flutter` or `source bin/activate-hermit` first bootstraps hermit itself,
and that is where Windows falls over. `bin/hermit` derives the download name
from `uname -s`, which on Windows under Git Bash / MSYS is:

```
$ uname -s
MINGW64_NT-10.0-26200
```

so it asks for

```
https://github.com/cashapp/hermit/releases/download/stable/hermit-mingw64_nt-10.0-26200-amd64.gz
```

and gets **HTTP 404** — cashapp/hermit publishes `darwin` and `linux` archives
only, no Windows build at any version. The bootstrap fails, `bin/flutter` never
runs, and after `source bin/activate-hermit` the shell reports:

```
flutter: command not found
```

That is a packaging gap, not a broken checkout. The mobile code itself is
perfectly testable on Windows — you just need the SDK installed directly
instead of through hermit. (An automated verifier has already misread this
404 as "mobile is unverifiable on this machine". It isn't.)

## Bootstrap

`scripts/bootstrap-windows.ps1` installs exactly the Flutter version this repo
pins, without hermit:

```powershell
pwsh -File scripts/bootstrap-windows.ps1
```

What it does:

1. Reads the pinned version out of `bin/.flutter-<version>.pkg` — it never
   hardcodes a version, so it follows the repo when the pin moves. It fails
   loudly if that marker is missing or ambiguous.
2. Resolves the matching Windows archive from
   `https://storage.googleapis.com/flutter_infra_release/releases/releases_windows.json`,
   failing if that exact version has no Windows build. Flutter publishes no
   arm64 Windows SDK (that manifest has zero arm64 entries), so on Windows on
   ARM the script says so and uses the x64 archive, which runs under emulation.
3. Downloads the archive into the install root and **always** verifies its
   SHA256 against the manifest before extracting. A mismatch aborts.
4. Extracts, then runs `flutter --version` and asserts the reported version
   equals the pinned one.
5. Prints the `PATH` / `PUB_CACHE` lines to set.

It is idempotent and non-destructive: an archive that is already downloaded is
re-verified but not refetched, an SDK that is already extracted at the pinned
version is left alone, and the script never deletes a directory. Any failure
exits non-zero. There are no prompts, so it is safe to run from CI or from an
agent.

Options:

| Flag | Meaning |
|------|---------|
| `-InstallRoot <dir>` | Where the SDK and pub cache live. Default `E:\flutter-sdk`. |
| `-PrintEnvOnly` | Skip install, just print the environment lines for an existing install. |

```powershell
pwsh -File scripts/bootstrap-windows.ps1 -InstallRoot C:\sdks\flutter-sdk
pwsh -File scripts/bootstrap-windows.ps1 -PrintEnvOnly
```

If the SDK at `-InstallRoot` is a *different* version than the repo pins, the
script stops and tells you to move it aside or pick another root — it will not
silently overwrite or delete an existing SDK.

## Environment

The script does not touch your persistent `PATH`. Set these per shell (adjust
if you used a custom `-InstallRoot`):

```powershell
# PowerShell
$env:PATH = "E:\flutter-sdk\flutter\bin;$env:PATH"
$env:PUB_CACHE = "E:\flutter-sdk\pub-cache"
```

```bash
# Git Bash
export PATH="/e/flutter-sdk/flutter/bin:$PATH"
export PUB_CACHE='E:\flutter-sdk\pub-cache'
```

`PUB_CACHE` always stays a Windows-style path — Dart resolves it itself and
does not understand MSYS `/e/...` paths.

Or skip `PATH` entirely and call the binaries directly, which is the most
reliable option for scripted/agent use:

```
E:\flutter-sdk\flutter\bin\flutter.bat --version
E:\flutter-sdk\flutter\bin\dart.bat format .
```

Never use `bin/flutter`, `bin/dart`, or `bin/activate-hermit` on Windows.

## What works on Windows, and what does not

Works, using the SDK installed above:

- `flutter test` — the full `mobile/` widget and unit test suite.
- `flutter analyze` — static analysis.
- `dart format` (and `dart format --output=none --set-exit-if-changed .` for
  the CI-style check).

So the mobile quality gate is fully runnable here:

```powershell
$env:PATH = "E:\flutter-sdk\flutter\bin;$env:PATH"
$env:PUB_CACHE = "E:\flutter-sdk\pub-cache"
cd mobile
dart format --output=none --set-exit-if-changed .
flutter analyze
flutter test
```

Does **not** work on Windows:

- **iOS simulator work — anything involving the iOS simulator, `xcodebuild`,
  CocoaPods, or iOS screenshots is macOS-only.** `just mobile-dev` starts an
  iOS simulator and cannot run here.
- `just mobile-*` recipes in general, and any other `just` recipe that shells
  out to a `bin/` hermit shim — those go through hermit and will fail the same
  way. Run the underlying `flutter` / `dart` commands directly instead.
- Per repo policy, agents must not run `flutter run`, `flutter build`,
  `flutter clean`, or `flutter upgrade` on any platform.

## Scope

This script covers Flutter/Dart only, because that is the toolchain the mobile
gate needs. The other hermit-pinned tools (`node`, `pnpm`, `rust`, `just`,
`biome`, …) hit the same hermit gap on Windows and have to be installed
natively, from their own installers, at the versions pinned in `bin/`.

## Troubleshooting

**`flutter: command not found` after `source bin/activate-hermit`** — expected;
see above. Use the bootstrap script.

**`Bootstrapping ... 404`** — expected; hermit has no Windows release.

**SHA256 mismatch** — the download is corrupt or tampered with. The script
refuses to extract and leaves the file in place; delete it yourself and re-run.

**`Get-FileHash` takes a while** — the archive is ~1.8 GB, and it is verified
whenever the script has to extract, including when the download itself was
skipped because the archive was already on disk. That is deliberate. Runs that
find the SDK already extracted skip the archive entirely and finish in seconds.
