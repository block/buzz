import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const panelSectionsSource = await readFile(
  new URL("./UserProfilePanelSections.tsx", import.meta.url),
  "utf8",
);
const panelTabsSource = await readFile(
  new URL("./UserProfilePanelTabs.tsx", import.meta.url),
  "utf8",
);

test("profile runtime surfaces never synthesize preview agent data", () => {
  for (const forbiddenPattern of [
    /UserProfileRuntimePreview/,
    /fillRuntimePreview/,
    /showRuntimePreview/,
    /UserProfileConfigPreview/,
    /import\.meta\.env\.DEV/,
  ]) {
    assert.doesNotMatch(panelSectionsSource, forbiddenPattern);
  }
});

test("runtime rows do not add interactive preview-only controls", () => {
  for (const forbiddenPattern of [
    /previewStartOnLaunchEnabled/,
    /isRuntimePreview/,
    /showPreviewHarnessLog/,
    /diagnostics-ingress-preview/,
  ]) {
    assert.doesNotMatch(panelTabsSource, forbiddenPattern);
  }
});
