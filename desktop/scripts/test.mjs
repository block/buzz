import { spawnSync } from "node:child_process";
import { statSync } from "node:fs";

const targets = process.argv.slice(2).flatMap((target) => {
  try {
    return statSync(target).isDirectory()
      ? [`${target}/**/*.test.mjs`]
      : [target];
  } catch {
    return [target];
  }
});
const result = spawnSync(
  process.execPath,
  [
    "--import",
    "./test-loader.mjs",
    "--experimental-strip-types",
    "--test",
    ...(targets.length ? targets : ["src/**/*.test.mjs"]),
  ],
  { stdio: "inherit" },
);
process.exit(result.status ?? 1);
