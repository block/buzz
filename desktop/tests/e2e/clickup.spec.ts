import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test("shows assigned ClickUp work grouped by urgency and opens details", async ({
  page,
}) => {
  await installMockBridge(page, { clickupConnected: true });
  await page.goto("/");

  await page.getByTestId("open-clickup-view").click();

  await expect(page).toHaveURL(/#\/clickup$/);
  await expect(page.getByRole("heading", { name: "ClickUp" })).toBeVisible();
  await expect(page.getByText("Read-only", { exact: true })).toBeVisible();
  await expect(page.getByLabel("Select ClickUp Workspace")).toHaveValue(
    "workspace-1",
  );
  await expect(page.getByRole("heading", { name: "Overdue" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Today" })).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Next 7 days" }),
  ).toBeVisible();
  await expect(page.getByRole("heading", { name: "Later" })).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "No due date" }),
  ).toBeVisible();

  await page.getByTestId("clickup-task-task-overdue").click();

  const detail = page.getByTestId("clickup-task-detail");
  await expect(detail).toContainText("Resolve launch blocker");
  await expect(detail).toContainText(
    "Verify the route, keyring boundary, pagination, and error states.",
  );
  await expect(detail).toContainText("Ready for a focused read-only review.");
  await expect(
    detail.getByRole("button", { name: "Open in ClickUp" }),
  ).toBeEnabled();
});

test("connects locally without recording the personal token in E2E logs", async ({
  page,
}) => {
  await installMockBridge(page, { clickupConnected: false });
  await page.goto("/#/clickup");

  const secret = "pk_test-secret-never-log";
  await page.getByTestId("clickup-token-input").fill(secret);
  await page.getByTestId("connect-clickup").click();

  await expect(page.getByText("Read-only", { exact: true })).toBeVisible();
  await expect(page.getByTestId("clickup-connect-card")).not.toBeVisible();

  const logs = await page.evaluate(() => ({
    commands: window.__BUZZ_E2E_COMMAND_PAYLOADS__,
    log: window.__BUZZ_E2E_COMMAND_LOG__,
  }));
  const serialized = JSON.stringify(logs);
  expect(serialized).not.toContain(secret);
  expect(serialized).toContain("[REDACTED]");
});

test("keeps transient connection failures out of the credential form", async ({
  page,
}) => {
  await installMockBridge(page, {
    clickupConnected: true,
    clickupConnectionError: "clickup:network::Buzz could not reach ClickUp.",
  });
  await page.goto("/#/clickup");

  await expect(
    page.getByRole("heading", { name: "ClickUp is temporarily unavailable" }),
  ).toBeVisible();
  await expect(page.getByTestId("clickup-connect-card")).not.toBeVisible();
  await expect(page.getByRole("button", { name: "Try again" })).toBeVisible();
});

test("clears a rejected personal token before native IPC settles", async ({
  page,
}) => {
  await installMockBridge(page, {
    clickupConnected: false,
    clickupConnectError:
      "clickup:unauthorized::ClickUp rejected the personal token.",
  });
  await page.goto("/#/clickup");

  const secret = "pk_rejected-secret-never-retain";
  const input = page.getByTestId("clickup-token-input");
  await input.fill(secret);
  await page.getByTestId("connect-clickup").click();

  await expect(input).toHaveValue("");
  await expect(page.getByRole("alert")).toContainText(
    "Enter a valid personal token",
  );
  const serialized = JSON.stringify(
    await page.evaluate(() => window.__BUZZ_E2E_COMMAND_LOG__),
  );
  expect(serialized).not.toContain(secret);
});

test("does not show an empty-success state when the first task read fails", async ({
  page,
}) => {
  await installMockBridge(page, {
    clickupConnected: true,
    clickupTasksError: "clickup:server::ClickUp is temporarily unavailable.",
  });
  await page.goto("/#/clickup");

  await expect(page.getByTestId("clickup-task-error")).toBeVisible();
  await expect(
    page.getByText("You have no open assigned tasks in this Workspace."),
  ).not.toBeVisible();
  await expect(page.getByRole("button", { name: "Try again" })).toBeVisible();
});

test("opens narrow-window details in a focused dialog", async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 700 });
  await installMockBridge(page, { clickupConnected: true });
  await page.goto("/#/clickup");

  const row = page.getByTestId("clickup-task-task-overdue");
  await row.click();

  const dialog = page.getByRole("dialog", { name: "ClickUp task details" });
  await expect(dialog).toBeVisible();
  const heading = dialog.getByRole("heading", {
    name: "Resolve launch blocker",
  });
  await expect(heading).toBeFocused();
  await dialog.getByRole("button", { name: "Close task details" }).click();
  await expect(row).toBeFocused();
});
