import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

/** Semver with optional prerelease (e.g. 0.5.23 or 0.5.23-hula.2). */
export const VERSION_PATTERN = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/;

/**
 * Patch every desktop version surface the UI / Tauri / Cargo read at build time.
 * Paths are relative to the desktop/ cwd (same as the release workflow).
 *
 * @param {string} version
 * @param {{ cwd?: string }} [options]
 * @returns {{ version: string, files: string[] }}
 */
export function setVersionFromTag(version, options = {}) {
  if (!version) {
    throw new Error("Usage: node scripts/set-version-from-tag.mjs <version>");
  }

  if (!VERSION_PATTERN.test(version)) {
    throw new Error(
      `Invalid version "${version}". Expected semver format (e.g. 1.2.3 or 1.2.3-hula.2)`,
    );
  }

  const cwd = options.cwd ?? process.cwd();
  const files = [];

  const packageJsonPath = resolve(cwd, "package.json");
  const tauriConfigPath = resolve(cwd, "src-tauri/tauri.conf.json");
  const cargoTomlPath = resolve(cwd, "src-tauri/Cargo.toml");
  const cargoLockPath = resolve(cwd, "src-tauri/Cargo.lock");

  const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf8"));
  packageJson.version = version;
  writeFileSync(packageJsonPath, `${JSON.stringify(packageJson, null, 2)}\n`);
  files.push("package.json");

  const tauriConfig = JSON.parse(readFileSync(tauriConfigPath, "utf8"));
  tauriConfig.version = version;
  writeFileSync(tauriConfigPath, `${JSON.stringify(tauriConfig, null, 2)}\n`);
  files.push("src-tauri/tauri.conf.json");

  const cargoToml = readFileSync(cargoTomlPath, "utf8");
  if (!/^\[package\]/m.test(cargoToml)) {
    throw new Error(`No [package] section in ${cargoTomlPath}`);
  }
  const updatedCargoToml = cargoToml.replace(
    /(\[package\][\s\S]*?^version = ")[^"]*(")/m,
    `$1${version}$2`,
  );
  if (
    updatedCargoToml === cargoToml &&
    !cargoToml.includes(`version = "${version}"`)
  ) {
    throw new Error(`Failed to update [package].version in ${cargoTomlPath}`);
  }
  writeFileSync(cargoTomlPath, updatedCargoToml);
  files.push("src-tauri/Cargo.toml");

  // Keep the workspace lock entry aligned so cargo metadata / tooling agree
  // without requiring a full `cargo update` (release CI still runs that).
  if (existsSync(cargoLockPath)) {
    const cargoLock = readFileSync(cargoLockPath, "utf8");
    const lockPattern =
      /(\[\[package\]\]\r?\nname = "buzz-desktop"\r?\n)version = "[^"]*"/;
    if (!lockPattern.test(cargoLock)) {
      throw new Error(
        `Cargo.lock missing [[package]] name = "buzz-desktop" stanza`,
      );
    }
    const updatedLock = cargoLock.replace(
      lockPattern,
      `$1version = "${version}"`,
    );
    writeFileSync(cargoLockPath, updatedLock);
    files.push("src-tauri/Cargo.lock");
  }

  return { version, files };
}

const isDirectRun =
  process.argv[1] != null &&
  fileURLToPath(import.meta.url) === resolve(process.argv[1]);

if (isDirectRun) {
  try {
    const result = setVersionFromTag(process.argv[2]);
    for (const file of result.files) {
      console.log(`Set ${file} to ${result.version}`);
    }
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  }
}
