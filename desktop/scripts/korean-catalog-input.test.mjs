import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import {
  KOREAN_CATALOG_SOURCE,
  loadKoreanCatalog,
  parseKoreanCatalogPatch,
  validateKoreanCatalog,
} from "./korean-catalog-input.mjs";

const marker =
  "diff --git a/desktop/src/shared/i18n/ko.generated.ts b/desktop/src/shared/i18n/ko.generated.ts";
function patch(expression) {
  return (
    marker +
    "\n--- /dev/null\n+++ b/desktop/src/shared/i18n/ko.generated.ts\n@@ -0,0 +1 @@\n+const KO_TABLE = " +
    expression +
    ";\n"
  );
}

test("the committed snapshot retains the pinned literal source", () => {
  const { table, source } = loadKoreanCatalog();
  assert.equal(source.ref, KOREAN_CATALOG_SOURCE.ref);
  assert.equal(Object.keys(table.exact).length, 2446);
  assert.equal(table.patterns.length, 321);
});

test("the parser stops before another diff and never runs its code", () => {
  const text =
    patch(
      '{ exact: { "Settings": "설정" }, patterns: [["By {0}", "{0} 작성"]] }',
    ) + "diff --git a/other.ts b/other.ts\n+const KO_TABLE = executeCode();\n";
  assert.deepEqual(parseKoreanCatalogPatch(text), {
    exact: { Settings: "설정" },
    patterns: [["By {0}", "{0} 작성"]],
  });
});

test("ambiguous diffs, duplicate properties and executable initializers are rejected", () => {
  assert.throws(
    () => parseKoreanCatalogPatch("not a catalogue"),
    /exactly one/,
  );
  assert.throws(
    () => parseKoreanCatalogPatch(patch("{}") + patch("{}")),
    /exactly one/,
  );
  assert.throws(
    () =>
      parseKoreanCatalogPatch(patch("{ exact: {}, exact: {}, patterns: [] }")),
    /Duplicate/,
  );
  assert.throws(
    () =>
      parseKoreanCatalogPatch(patch("{ exact: { a: run() }, patterns: [] }")),
    /Non-literal/,
  );
  assert.throws(
    () =>
      parseKoreanCatalogPatch(patch("{ exact: { ...other }, patterns: [] }")),
    /Non-literal/,
  );
  assert.throws(
    () => parseKoreanCatalogPatch(patch("{}") + "+const KO_TABLE = {};\n"),
    /one KO_TABLE/,
  );
});

test("catalogues require string pairs and a known top-level structure", () => {
  for (const value of [
    null,
    [],
    {},
    { exact: [], patterns: [] },
    { exact: { a: 4 }, patterns: [] },
    { exact: {}, patterns: [["a"]] },
    { exact: {}, patterns: [["a", ""]] },
    { exact: {}, patterns: [], code: "unexpected" },
  ]) {
    assert.throws(() => validateKoreanCatalog(value), /catalogue|Catalogue/);
  }
});

test("generation is current and an unpinned input cannot alter outputs", () => {
  const script = path.join(import.meta.dirname, "import-korean-catalog.mjs");
  const current = spawnSync(process.execPath, [script, "--check"], {
    encoding: "utf8",
  });
  assert.equal(current.status, 0, current.stderr);
  assert.equal(JSON.parse(current.stdout).changed, 0);
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), "buzz-catalog-test-"));
  const output = path.resolve(import.meta.dirname, "../src/locales/ko.ui.json");
  const before = fs.readFileSync(output);
  try {
    const input = path.join(temp, "tampered.patch");
    fs.writeFileSync(
      input,
      patch('{ exact: { "Settings": "바뀐 값" }, patterns: [] }'),
    );
    const rejected = spawnSync(process.execPath, [script, input], {
      encoding: "utf8",
    });
    assert.notEqual(rejected.status, 0);
    assert.match(rejected.stderr, /source hash mismatch/);
    assert.deepEqual(fs.readFileSync(output), before);
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
});
