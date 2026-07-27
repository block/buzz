import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";

test("macOS product identity is Command Adviser without changing stable internals", async () => {
  const config = JSON.parse(
    await readFile(new URL("../src-tauri/tauri.conf.json", import.meta.url)),
  );
  const plist = await readFile(
    new URL("../src-tauri/Info.plist", import.meta.url),
    "utf8",
  );

  assert.equal(config.productName, "Command Adviser");
  assert.equal(config.identifier, "xyz.block.buzz.app");
  assert.deepEqual(config.plugins["deep-link"].desktop.schemes, ["buzz"]);
  assert.match(plist, /<string>Command Adviser<\/string>/);
  assert.doesNotMatch(plist, />Buzz needs|>Buzz can read/);
});
