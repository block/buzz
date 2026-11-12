import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

// Structural tripwire (plan D4, defense-in-depth): NIP-49 backup material
// handling in the webview is confined to the identity/backup/import UI and
// its API wrappers. Anything else in `desktop/src` touching `ncryptsec` is
// structural drift toward an unguarded egress path and must be reviewed —
// the runtime guarantee lives in src-tauri's egress guard, this scan only
// keeps the blob from quietly spreading through the frontend.
//
// Mirror of the Rust-side scan in
// `src-tauri/src/egress_guard_tests.rs::ncryptsec_handling_is_confined_to_allowlisted_files`.

const SRC_ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);

const ALLOWLIST = [
  "shared/api/tauriIdentity.ts",
  "features/onboarding/lib/encryptedBackup.ts",
  "features/onboarding/lib/encryptedBackup.test.mjs",
  "features/onboarding/lib/keyImportInput.ts",
  "features/onboarding/lib/keyImportInput.test.mjs",
  "features/onboarding/ui/BackupStep.tsx",
  "features/onboarding/ui/BackupPasswordTimeline.tsx",
  "features/onboarding/ui/BackupTestFlow.tsx",
  "features/onboarding/ui/EncryptedBackupCreator.tsx",
  "features/onboarding/ui/NostrKeyImportForm.tsx",
  "features/onboarding/ui/NsecMaskedDisplay.tsx",
  "features/settings/EncryptedBackupProvider.tsx",
  "features/settings/lib/encryptedBackup.ts",
  "features/settings/lib/encryptedBackup.test.mjs",
  "features/settings/ui/BackupTestFlow.tsx",
  "features/settings/ui/EncryptedBackupCreator.tsx",
  "features/settings/ui/ProfileSettingsCard.tsx",
  // e2e-only mock bridge (never in the production bundle):
  "testing/e2eBridge.ts",
  // this scan:
  "shared/lib/ncryptsecSourceScan.test.mjs",
];

function* walk(dir) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      yield* walk(full);
    } else if (/\.(ts|tsx|mjs|js|jsx)$/.test(entry.name)) {
      yield full;
    }
  }
}

test("ncryptsec handling is confined to allowlisted frontend files", () => {
  const violations = [];
  for (const file of walk(SRC_ROOT)) {
    const rel = path.relative(SRC_ROOT, file).replaceAll("\\", "/");
    if (ALLOWLIST.includes(rel)) continue;
    const content = fs.readFileSync(file, "utf8");
    if (content.toLowerCase().includes("ncryptsec")) {
      violations.push(rel);
    }
  }
  assert.deepEqual(
    violations,
    [],
    `NIP-49 material outside allowlisted files — wire it through the ` +
      `identity layer (and its egress-guarded Rust commands) instead:\n` +
      violations.join("\n"),
  );
});
