import assert from "node:assert/strict";
import {
  mkdtempSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { describe, it } from "node:test";
import { loadEnv } from "vite";

import {
  buildArtifactMatrix,
  selectInternalVariant,
} from "../../scripts/build-protected-feature-artifacts.mjs";

const INTERNAL_MARKER = "Try a personal agent that is always close at hand";

function fakeBuilder(calls) {
  return ({ internal, output }) => {
    calls.push(internal);
    rmSync(output, { recursive: true, force: true });
    mkdirSync(output, { recursive: true });
    writeFileSync(
      path.join(output, "index.js"),
      internal ? INTERNAL_MARKER : "public desktop artifact",
    );
  };
}

describe("protected feature production artifact selection", () => {
  it("honors env-file selection while process overrides retain the requested dist", () => {
    const root = mkdtempSync(path.join(tmpdir(), "buzz-protected-build-test-"));
    const envRoot = path.join(root, "env");
    mkdirSync(envRoot);
    writeFileSync(path.join(envRoot, ".env.local"), "VITE_BUZZ_BESTIE=1\n");

    try {
      const modeEnv = loadEnv("production", envRoot, "");
      const internalOutput = path.join(root, "internal-dist");
      const internalAlternate = path.join(root, "internal-alternate");
      const internalCalls = [];
      const fileSelectedInternal = selectInternalVariant({
        processEnv: {},
        modeEnv,
      });

      assert.equal(fileSelectedInternal, true);
      buildArtifactMatrix({
        selectedInternalVariant: fileSelectedInternal,
        selectedOutput: internalOutput,
        alternateOutput: internalAlternate,
        build: fakeBuilder(internalCalls),
      });
      assert.deepEqual(internalCalls, [false, true]);
      assert.match(
        readFileSync(path.join(internalOutput, "index.js"), "utf8"),
        /personal agent/u,
      );
      assert.doesNotMatch(
        readFileSync(path.join(internalAlternate, "index.js"), "utf8"),
        /personal agent/u,
      );

      const ossOutput = path.join(root, "oss-dist");
      const ossAlternate = path.join(root, "oss-alternate");
      const ossCalls = [];
      const processSelectedOss = selectInternalVariant({
        processEnv: { VITE_BUZZ_BESTIE: "0" },
        modeEnv,
      });

      assert.equal(processSelectedOss, false);
      buildArtifactMatrix({
        selectedInternalVariant: processSelectedOss,
        selectedOutput: ossOutput,
        alternateOutput: ossAlternate,
        build: fakeBuilder(ossCalls),
      });
      assert.deepEqual(ossCalls, [true, false]);
      assert.doesNotMatch(
        readFileSync(path.join(ossOutput, "index.js"), "utf8"),
        /personal agent/u,
      );
      assert.match(
        readFileSync(path.join(ossAlternate, "index.js"), "utf8"),
        /personal agent/u,
      );
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
