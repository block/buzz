import assert from "node:assert/strict";
import test from "node:test";

import {
  assertCommandAdviserArtifactNames,
  checkCommandAdviserSourceIdentity,
} from "./check-command-adviser-branding.mjs";

test("macOS product identity is Command Adviser without changing stable internals", async () => {
  await checkCommandAdviserSourceIdentity(new URL("../", import.meta.url));
});

test("native artifact names expose Command Adviser", () => {
  assert.doesNotThrow(() =>
    assertCommandAdviserArtifactNames(
      "/tmp/Command Adviser.app",
      "/tmp/Command Adviser_0.4.24_aarch64.dmg",
    ),
  );
  assert.throws(
    () =>
      assertCommandAdviserArtifactNames(
        "/tmp/Buzz.app",
        "/tmp/Buzz_0.4.24_aarch64.dmg",
      ),
    /Command Adviser/,
  );
});
