import assert from "node:assert/strict";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";

const desktopRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../..",
);
const wrapper = path.join(desktopRoot, "scripts/tauri-command.mjs");
const fakeCli = path.join(tmpdir(), `buzz-fake-tauri-${process.pid}.mjs`);

writeFileSync(
  fakeCli,
  `import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
const args = process.argv.slice(2);
const configIndex = args.lastIndexOf("--config");
const override = JSON.parse(args[configIndex + 1]);
const output = override.build.frontendDist;
mkdirSync(output, { recursive: true });
writeFileSync(path.join(output, "variant.txt"), process.env.VITE_BUZZ_BESTIE);
await new Promise((resolve) => setTimeout(resolve, 100));
const observed = readFileSync(path.join(output, "variant.txt"), "utf8");
writeFileSync(
  process.env.BUZZ_TEST_RESULT,
  JSON.stringify({ args, output, observed }),
);
`,
);

function packageVariant(variant, result, runnerArguments = []) {
  return new Promise((resolve, reject) => {
    const child = spawn(
      process.execPath,
      [wrapper, "build", ...runnerArguments],
      {
        cwd: desktopRoot,
        env: {
          ...process.env,
          BUZZ_TAURI_CLI_ENTRYPOINT: fakeCli,
          BUZZ_TEST_RESULT: result,
          VITE_BUZZ_BESTIE: variant,
        },
        stdio: "inherit",
      },
    );
    child.once("error", reject);
    child.once("exit", (code) =>
      code === 0 ? resolve() : reject(new Error(`wrapper exited ${code}`)),
    );
  });
}

test("opposite Tauri package variants own private frontend artifacts", async () => {
  const resultRoot = path.join(tmpdir(), `buzz-tauri-results-${process.pid}`);
  mkdirSync(resultRoot, { recursive: true });
  const ossResult = path.join(resultRoot, "oss.json");
  const internalResult = path.join(resultRoot, "internal.json");

  await Promise.all([
    packageVariant("0", ossResult),
    packageVariant("1", internalResult),
  ]);

  const oss = JSON.parse(readFileSync(ossResult, "utf8"));
  const internal = JSON.parse(readFileSync(internalResult, "utf8"));
  assert.equal(oss.observed, "0");
  assert.equal(internal.observed, "1");
  assert.notEqual(oss.output, internal.output);
});

test("private config precedes Cargo runner arguments", async () => {
  const result = path.join(
    tmpdir(),
    `buzz-tauri-runner-arguments-${process.pid}.json`,
  );
  await packageVariant("0", result, [
    "--config",
    '{"bundle":{"active":false}}',
    "--",
    "--locked",
  ]);

  const invocation = JSON.parse(readFileSync(result, "utf8"));
  const delimiterIndex = invocation.args.indexOf("--");
  const privateConfigIndex = invocation.args.lastIndexOf("--config");
  assert.ok(privateConfigIndex < delimiterIndex);
  assert.equal(invocation.args[delimiterIndex + 1], "--locked");
  assert.equal(
    JSON.parse(invocation.args[privateConfigIndex + 1]).build.frontendDist,
    invocation.output,
  );
});
