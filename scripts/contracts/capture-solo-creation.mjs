// Capture the current desktop's production create call, without a relay write.
// Usage: node --experimental-transform-types scripts/contracts/capture-solo-creation.mjs /path/to/buzz-app
import { registerHooks } from "node:module";
import { pathToFileURL } from "node:url";
import { resolve } from "node:path";
import { execFileSync } from "node:child_process";
import assert from "node:assert/strict";
const root = resolve(process.argv[2]);
execFileSync("git", ["-C", root, "diff", "--exit-code", "HEAD", "--", "src"]);
registerHooks({
  resolve(specifier, context, next) {
    try {
      return next(specifier, context);
    } catch (error) {
      if (specifier.startsWith(".") && !/\.[a-z]+$/.test(specifier))
        return next(`${specifier}.ts`, context);
      throw error;
    }
  },
});
const { createChannelWrites } = await import(
  pathToFileURL(`${root}/src/features/relay/channel-writes.ts`)
);
let creation;
const outbox = {
  supports: () => true,
  send: (event) => {
    creation = event;
    return "captured";
  },
};
const reader = {};
createChannelWrites(
  outbox,
  {},
  reader,
  new AbortController().signal,
  undefined,
  undefined,
  undefined,
  undefined,
  reader,
  true,
).create("11111111-1111-4111-8111-111111111111", "Desktop Solo");
assert.equal(creation.kind, 9007);
assert.ok(
  creation.tags.some(([key, value]) => key === "t" && value === "buzz.solo"),
);
console.log(
  JSON.stringify(
    {
      producer: "https://github.com/block/buzz-app",
      revision: execFileSync("git", ["-C", root, "rev-parse", "HEAD"], {
        encoding: "utf8",
      }).trim(),
      creation,
      creator: "11".repeat(32),
      identity_tags: [
        ["t", "stream"],
        ["t", "buzz.solo"],
        ["P", "11".repeat(32)],
      ],
    },
    null,
    2,
  ),
);
