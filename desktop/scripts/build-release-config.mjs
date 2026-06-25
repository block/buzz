import { writeFileSync } from "node:fs";
import { resolve } from "node:path";

// Write a tauri.release.conf.json with release-only overrides.
//
// Tauri's --config flag merges the provided JSON on top of the base
// tauri.conf.json, so this file must contain ONLY the delta fields —
// not a copy of the base config.
//
// For OSS release builds this script emits:
// 1. bundle.macOS.minimumSystemVersion = "10.15" for broad compatibility.
// 2. bundle.createUpdaterArtifacts = true so Tauri produces the .tar.gz
//    archive and .sig signature during the build.
// 3. plugins.updater with the public key and endpoint from env vars.
//    Both BUZZ_UPDATER_PUBLIC_KEY and BUZZ_UPDATER_ENDPOINT are required -
//    the script fails if either is missing (OSS builds always ship with updater).
//
// Apple code signing and notarization happen post-build via
// block/apple-codesign-action in release.yml, so no signingIdentity is
// emitted here and the Tauri build is invoked with --no-sign.

const outputConfigPath = resolve(
  process.cwd(),
  "src-tauri/tauri.release.conf.json",
);

const updaterPubkey = process.env.BUZZ_UPDATER_PUBLIC_KEY;
const updaterEndpoint = process.env.BUZZ_UPDATER_ENDPOINT;

const missing = [];
if (!updaterPubkey) missing.push("BUZZ_UPDATER_PUBLIC_KEY");
if (!updaterEndpoint) missing.push("BUZZ_UPDATER_ENDPOINT");
if (missing.length > 0) {
  console.error(
    `Error: required environment variable(s) missing: ${missing.join(", ")}`,
  );
  process.exit(1);
}

const releaseConfig = {
  bundle: {
    macOS: {
      minimumSystemVersion: "10.15",
    },
    createUpdaterArtifacts: true,
  },
  plugins: {
    updater: {
      pubkey: updaterPubkey,
      endpoints: [updaterEndpoint],
    },
  },
};

// Windows-only: bundle the full PortableGit toolchain (bash runtime + git +
// curl/coreutils in mingw64/, plus a vendored standalone jq.exe) as a resource so
// the MCP shell tool always has a genuine, non-WSL bash AND a real dev toolchain
// to spawn on a bare host (the app must be self-contained — we cannot assume Git
// for Windows is installed).
//
// This is emitted ONLY on the Windows runner because the static tauri.conf.json
// uses `targets: "all"` with a shared bundle block — a bare `resources` entry
// there would ship the (now ~350MB+) tree into the macOS .dmg and Linux packages
// too. The release build runs THIS generator on each platform's own runner and
// merges the output via --config, so guarding on process.platform keeps the tree
// off mac/Linux.
//
// PATH CONTRACT (keep byte-identical across three files):
//   - source `binaries/git-bash` (relative to src-tauri/) is staged by
//     scripts/bundle-sidecars.sh.
//   - target `git-bash` is the install-root subdir; Tauri's Windows installer
//     stages it next to the exe, and crates/buzz-dev-mcp/src/shell.rs resolves
//     `git-bash\bin\bash.exe` relative to its own executable at runtime.
if (process.platform === "win32") {
  releaseConfig.bundle.resources = { "binaries/git-bash": "git-bash" };
}

console.log(`Updater enabled -> ${updaterEndpoint}`);

writeFileSync(outputConfigPath, `${JSON.stringify(releaseConfig, null, 2)}\n`);
console.log(`Wrote ${outputConfigPath}`);
