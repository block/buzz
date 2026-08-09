#!/usr/bin/env node

import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { access, readFile, readdir } from "node:fs/promises";
import { basename, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);
const EXPECTED_NAME = "Command Adviser";
const EXPECTED_IDENTIFIER = "xyz.block.buzz.app";

function asPath(value) {
  return value instanceof URL ? fileURLToPath(value) : value;
}

export function assertCommandAdviserArtifactNames(appPath, dmgPath) {
  assert.equal(
    basename(appPath),
    `${EXPECTED_NAME}.app`,
    `Application path must expose ${EXPECTED_NAME}`,
  );
  if (dmgPath) {
    assert.match(
      basename(dmgPath),
      /^Command Adviser(?:_|-).+\.dmg$/,
      `DMG path must expose ${EXPECTED_NAME}`,
    );
  }
}

export async function checkCommandAdviserSourceIdentity(desktopRoot) {
  const root = asPath(desktopRoot);
  const config = JSON.parse(
    await readFile(join(root, "src-tauri", "tauri.conf.json"), "utf8"),
  );
  const plist = await readFile(join(root, "src-tauri", "Info.plist"), "utf8");

  assert.equal(config.productName, EXPECTED_NAME);
  assert.equal(config.identifier, EXPECTED_IDENTIFIER);
  assert.deepEqual(config.plugins["deep-link"].desktop.schemes, ["buzz"]);
  assert.match(plist, /<string>Command Adviser<\/string>/);
  assert.doesNotMatch(plist, />Buzz needs|>Buzz can read/);

  return config;
}

async function plistValue(plistPath, key) {
  const { stdout } = await execFileAsync("/usr/bin/plutil", [
    "-extract",
    key,
    "raw",
    "-o",
    "-",
    plistPath,
  ]);
  return stdout.trim();
}

export async function checkCommandAdviserBundle(desktopRoot, appPath, dmgPath) {
  assertCommandAdviserArtifactNames(appPath, dmgPath);
  const config = await checkCommandAdviserSourceIdentity(desktopRoot);
  const plistPath = join(appPath, "Contents", "Info.plist");

  assert.equal(
    await plistValue(plistPath, "CFBundleDisplayName"),
    EXPECTED_NAME,
  );
  assert.equal(await plistValue(plistPath, "CFBundleName"), EXPECTED_NAME);
  assert.equal(
    await plistValue(plistPath, "CFBundleIdentifier"),
    EXPECTED_IDENTIFIER,
  );

  const iconFile = await plistValue(plistPath, "CFBundleIconFile");
  const iconName = extname(iconFile) ? iconFile : `${iconFile}.icns`;
  await access(join(appPath, "Contents", "Resources", iconName));

  const executables = await readdir(join(appPath, "Contents", "MacOS"));
  for (const configuredSidecar of config.bundle.externalBin) {
    const sidecar = basename(configuredSidecar);
    assert.ok(
      executables.some(
        (candidate) =>
          candidate === sidecar || candidate.startsWith(`${sidecar}-`),
      ),
      `Bundle is missing configured sidecar ${sidecar}`,
    );
  }
}

function optionValue(argv, option) {
  const index = argv.indexOf(option);
  return index >= 0 ? argv[index + 1] : undefined;
}

async function main() {
  const desktopRoot = resolve(fileURLToPath(new URL("../", import.meta.url)));
  const appPath = optionValue(process.argv, "--app");
  const dmgPath = optionValue(process.argv, "--dmg");

  if (appPath) {
    await checkCommandAdviserBundle(desktopRoot, appPath, dmgPath);
  } else {
    await checkCommandAdviserSourceIdentity(desktopRoot);
  }

  process.stdout.write(
    `${EXPECTED_NAME} branding verified${appPath ? `: ${appPath}` : ""}\n`,
  );
}

if (
  process.argv[1] &&
  resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main().catch((error) => {
    process.stderr.write(`${error instanceof Error ? error.message : error}\n`);
    process.exitCode = 1;
  });
}
