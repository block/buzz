import assert from "node:assert/strict";
import test from "node:test";
import ts from "typescript";

const sourcePath = new URL("./workflowEditorPane.ts", import.meta.url);
const source = await import("node:fs/promises").then((fs) =>
  fs.readFile(sourcePath, "utf8"),
);
const compiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.ESNext,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText;
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString("base64")}`;
const { parseWorkflowEditorPane, serializeWorkflowEditorPane } = await import(
  moduleUrl
);

test("parses trigger and stable step-id panes", () => {
  assert.deepEqual(parseWorkflowEditorPane("trigger"), { type: "trigger" });
  assert.deepEqual(parseWorkflowEditorPane("step:notify_team"), {
    type: "step",
    stepId: "notify_team",
  });
  const maxId = "a".repeat(64);
  assert.deepEqual(parseWorkflowEditorPane(`step:${maxId}`), {
    type: "step",
    stepId: maxId,
  });
});

test("rejects malformed pane parameters", () => {
  for (const value of [
    undefined,
    null,
    "",
    "step:",
    "step:%E0%A4%A",
    "step-0",
    "step:notify%20team",
    "step:bad-id",
    `step:${"a".repeat(65)}`,
    `step:${"%41".repeat(65)}`,
  ])
    assert.equal(parseWorkflowEditorPane(value), null);
});

test("serializes stable step-id panes", () => {
  assert.equal(serializeWorkflowEditorPane(null), undefined);
  assert.equal(serializeWorkflowEditorPane({ type: "trigger" }), "trigger");
  assert.equal(
    serializeWorkflowEditorPane({ type: "step", stepId: "notify_team" }),
    "step:notify_team",
  );
  assert.equal(
    serializeWorkflowEditorPane({ type: "step", stepId: "bad-id" }),
    undefined,
  );
});
