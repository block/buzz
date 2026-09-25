import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { runInNewContext } from "node:vm";

const workflow = readFileSync(
  new URL("../.github/workflows/ci.yml", import.meta.url),
  "utf8",
);
const gates = [
  "rust-lint",
  "unit-tests",
  "windows-rust",
  "desktop",
  "desktop-build-macos",
  "desktop-e2e-relay",
  "desktop-e2e-integration",
  "backend-integration",
  "postgres-tests",
  "relay-e2e",
  "web",
  "mobile",
  "security",
];
for (const gate of gates) {
  const body = workflow.match(
    new RegExp(`^  ${gate}:\\n([\\s\\S]*?)(?=^  [\\w-]+:|$(?![\\s\\S]))`, "m"),
  )[1];
  const condition = body.match(/^ {4}if: (.+)$/m)[1];
  const command = body.match(/^ {8}run: (.+)$/m)[1];
  function shouldRun(
    selection,
    selected = false,
    event = "pull_request",
    artifacts = "skipped",
  ) {
    // These workflow conditions use only booleans, string equality and grouping.
    // Evaluate the actual expression after substituting its GitHub context values.
    const expression = condition
      .replace(/always\(\)/g, "true")
      .replace(
        /github\.event_name|needs\.[\w-]+\.(?:result|outputs\.[\w-]+)/g,
        (key) => {
          if (key === "github.event_name") return JSON.stringify(event);
          if (key === "needs.changes.result") return JSON.stringify(selection);
          if (key === "needs.relay-artifacts-domain.result")
            return JSON.stringify(artifacts);
          assert.match(key, /^needs\.changes\.outputs\./);
          return JSON.stringify(selected ? "true" : "false");
        },
      );
    return runInNewContext(expression, {}, { timeout: 100 });
  }
  function check(selection, result) {
    assert.match(body, /SELECTION_RESULT: \$\{\{ needs.changes.result \}\}/);
    assert.match(body, /RESULT: \$\{\{ needs\.[\w-]+\.outputs\.[\w_]+ \}\}/);
    return spawnSync("bash", ["-c", command], {
      env: { ...process.env, SELECTION_RESULT: selection, RESULT: result },
      timeout: 1000,
    }).status;
  }
  test(`${gate}: selector failures run and fail the required check`, () => {
    for (const selection of ["failure", "cancelled", "skipped"]) {
      for (const artifacts of ["skipped", "success"]) {
        assert.equal(
          shouldRun(selection, false, "pull_request", artifacts),
          true,
        );
      }
      for (const result of ["", "skipped", "success"]) {
        assert.notEqual(check(selection, result), 0);
      }
    }
  });
  test(`${gate}: successful selection preserves path gating and suite results`, () => {
    assert.equal(shouldRun("success"), false);
    assert.equal(shouldRun("success", true, "pull_request", "success"), true);
    assert.equal(shouldRun("success", false, "push", "success"), true);
    assert.equal(check("success", "success"), 0);
    for (const result of ["", "failure", "cancelled", "skipped"]) {
      assert.notEqual(check("success", result), 0);
    }
  });
}
