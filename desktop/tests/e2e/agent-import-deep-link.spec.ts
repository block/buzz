import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const LONG_PROMPT = Array.from(
  { length: 60 },
  (_, index) =>
    `Section ${index + 1}: Inspect the requested work, preserve unrelated changes, and report concrete evidence before acting.`,
).join("\n\n");

test("agent import deep link opens a bounded review dialog", async ({
  page,
}) => {
  await installMockBridge(page, {
    pendingAgentSnapshotImports: [
      {
        id: "agent-import-1",
        fileBytes: [123, 125],
        fileName: "long-prompt.agent.json",
      },
    ],
    agentSnapshotPreviewSystemPrompt: LONG_PROMPT,
  });

  await page.goto("/");

  const dialog = page.getByTestId("agent-snapshot-import-dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("Imported Agent");
  await expect(dialog).toContainText(
    `Instructions · ${LONG_PROMPT.length.toLocaleString()} characters`,
  );

  const excerpt = dialog.getByTestId("agent-snapshot-import-prompt-excerpt");
  await expect(excerpt).toBeVisible();
  await expect(excerpt).not.toContainText("Section 60");
  await expect(
    dialog.getByTestId("agent-snapshot-import-confirm"),
  ).toBeVisible();

  const toggle = dialog.getByTestId("agent-snapshot-import-prompt-toggle");
  await expect(toggle).toHaveText("Review full instructions");
  await toggle.click();

  const fullPrompt = dialog.getByTestId("agent-snapshot-import-full-prompt");
  await expect(fullPrompt).toBeVisible();
  await expect(fullPrompt).toContainText("Section 60");
  const dimensions = await fullPrompt.evaluate((element) => ({
    clientHeight: element.clientHeight,
    scrollHeight: element.scrollHeight,
  }));
  expect(dimensions.scrollHeight).toBeGreaterThan(dimensions.clientHeight);

  await expect(toggle).toHaveText("Hide full instructions");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_COMMANDS__?.filter(
            (command) =>
              command === "acknowledge_pending_agent_snapshot_import",
          ).length ?? 0,
      ),
    )
    .toBe(1);
});
