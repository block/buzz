import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import ts from "typescript";

// These tables contain authored display copy, not wire values or user names.
// Keep protocol IDs, CLI brand labels and feedback payload metadata outside it.
const tables = {
  "features/communities/ui/CommunitySwitcher.tsx": ["CONNECTION_STATE_LABEL"],
  "features/channels/ui/ChannelTypePicker.tsx": ["LIFECYCLE_LABEL"],
  "features/admin-console/AdminConsoleActionsTab.tsx": ["ACTION_LABELS"],
  "features/mesh-compute/ui/MeshComputeSettingsCard.tsx": ["FIT_LABEL"],
  "features/agents/ui/AgentConfigPanel.tsx": ["NORMALIZED_LABELS"],
  "features/agents/ui/buzzAgentModelTuningFields.tsx": [
    "NUMERIC_KIND_LABELS",
    "NUMERIC_KIND_DESCRIPTIONS",
  ],
  "features/profile/ui/ProfileAvatarModeTabs.tsx": ["MODE_TAB_LABELS"],
  "features/workflows/ui/WorkflowTriggerConditions.tsx": ["OPERATOR_LABELS"],
  "features/workflows/ui/WorkflowDialog.tsx": [
    "TITLES",
    "SUBMIT_LABELS",
    "PENDING_LABELS",
  ],
  "features/projects/ui/ProjectsWorkItemsLoadNotice.tsx": ["SECTION_LABELS"],
  "features/home/ui/InboxListPane.tsx": [
    "INBOX_EMPTY_STATE_TITLES",
    "INBOX_UNREAD_EMPTY_STATE_TITLES",
  ],
};

test("owned display tables register their strings and translate every lookup", () => {
  const keys = JSON.parse(
    fs.readFileSync(
      path.join(import.meta.dirname, "locales/ui-keys.json"),
      "utf8",
    ),
  );
  const errors = [];
  for (const [file, names] of Object.entries(tables)) {
    const ast = ts.createSourceFile(
      file,
      fs.readFileSync(path.join(import.meta.dirname, file), "utf8"),
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX,
    );
    const found = new Set();
    function visit(node) {
      if (
        ts.isVariableDeclaration(node) &&
        names.includes(node.name.getText(ast)) &&
        ts.isObjectLiteralExpression(node.initializer)
      ) {
        found.add(node.name.getText(ast));
        for (const property of node.initializer.properties)
          if (
            ts.isPropertyAssignment(property) &&
            ts.isStringLiteralLike(property.initializer) &&
            !keys[property.initializer.text]
          )
            errors.push(`${file}: unregistered ${property.initializer.text}`);
      }
      if (
        ts.isElementAccessExpression(node) &&
        names.includes(node.expression.getText(ast))
      ) {
        let parent = node.parent;
        let translated = false;
        while (parent && !ts.isStatement(parent)) {
          if (
            ts.isCallExpression(parent) &&
            parent.expression.getText(ast) === "uiT"
          )
            translated = true;
          parent = parent.parent;
        }
        if (!translated)
          errors.push(`${file}: untranslated ${node.getText(ast)}`);
      }
      ts.forEachChild(node, visit);
    }
    visit(ast);
    assert.deepEqual(
      [...found].sort(),
      [...names].sort(),
      `Audit table changed in ${file}`,
    );
  }
  assert.deepEqual(errors, []);
});
