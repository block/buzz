import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const desktopRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const tauriPackageJsonPath = fileURLToPath(
  import.meta.resolve("@tauri-apps/cli/package.json"),
);
const tauriPackage = JSON.parse(readFileSync(tauriPackageJsonPath, "utf8"));
const defaultTauriEntrypoint = path.resolve(
  path.dirname(tauriPackageJsonPath),
  tauriPackage.bin.tauri,
);

function runTauri(args, options = {}) {
  const entrypoint =
    process.env.BUZZ_TAURI_CLI_ENTRYPOINT ?? defaultTauriEntrypoint;
  const result = spawnSync(process.execPath, [entrypoint, ...args], {
    cwd: desktopRoot,
    env: { ...process.env, ...options.env },
    stdio: "inherit",
  });
  if (result.error) throw result.error;
  return result.status ?? 1;
}

export function runTauriCommand(args) {
  if (args[0] !== "build") return runTauri(args);

  // Tauri runs beforeBuildCommand and then consumes frontendDist. Give the
  // entire invocation a private directory so concurrent OSS/internal packages
  // cannot replace one another's assets between those two operations.
  const invocationRoot = mkdtempSync(
    path.join(tmpdir(), "buzz-tauri-package-assets-"),
  );
  const frontendDist = path.join(invocationRoot, "dist");
  const outputOverride = JSON.stringify({ build: { frontendDist } });

  try {
    const delimiterIndex = args.indexOf("--");
    const configIndex = delimiterIndex === -1 ? args.length : delimiterIndex;
    const tauriArgs = [...args];
    tauriArgs.splice(configIndex, 0, "--config", outputOverride);
    return runTauri(tauriArgs, {
      env: { BUZZ_PROTECTED_BUILD_OUTPUT: frontendDist },
    });
  } finally {
    rmSync(invocationRoot, { recursive: true, force: true });
  }
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  process.exitCode = runTauriCommand(process.argv.slice(2));
}
