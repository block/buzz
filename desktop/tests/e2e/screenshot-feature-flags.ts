import { expect, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const WATERCOLOR_CHANNEL_ID = "a27e1ee9-76a6-5bdf-a5d5-1d85610dad11";
const FORUM_POST_ID = "mock-forum-release-thread";

test.beforeEach(async ({ page }) => {
  // This spec exercises the Experiments toggle flow itself (each preview
  // feature is enabled via the settings UI), so don't pre-seed overrides.
  await installMockBridge(page, undefined, { seedPreviewFeatures: false });
});

// Helper: enable a preview feature via settings, then close settings
async function enableFeature(
  page: import("@playwright/test").Page,
  featureId: string,
) {
  await openSettings(page);
  await page.getByTestId("settings-nav-experimental").click();
  await expect(page.getByTestId("settings-experimental")).toBeVisible();
  await page.getByTestId(`feature-toggle-${featureId}`).click();
  await page.waitForTimeout(200);
  await page.getByTestId("settings-back-to-app").click();
  await page.waitForTimeout(300);
}

// --- Stable features (settings panels, always visible) ---

test("screenshot: Settings → Agents (stable)", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, "agents");
  await page.waitForTimeout(500);
  const view = page.getByTestId("settings-view");
  await view.screenshot({ path: "tests/e2e/screenshots/settings-agents.png" });
});

test("screenshot: Settings → Templates (stable)", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, "channel-templates");
  await page.waitForTimeout(500);
  const view = page.getByTestId("settings-view");
  await view.screenshot({
    path: "tests/e2e/screenshots/settings-templates.png",
  });
});

test("screenshot: Settings → Custom Emoji (stable)", async ({ page }) => {
  await page.goto("/");
  await openSettings(page);
  await page.getByTestId("settings-nav-custom-emoji").click();
  await page.waitForTimeout(500);
  const view = page.getByTestId("settings-view");
  await view.screenshot({
    path: "tests/e2e/screenshots/settings-custom-emoji.png",
  });
});

test("screenshot: Settings → Doctor (stable)", async ({ page }) => {
  await page.goto("/");
  await openSettings(page, "doctor");
  await page.waitForTimeout(500);
  const view = page.getByTestId("settings-view");
  await view.screenshot({ path: "tests/e2e/screenshots/settings-doctor.png" });
});

// --- Preview features (opt-in via Experiments) ---

test("screenshot: Workflows view (preview)", async ({ page }) => {
  await page.goto("/");
  await enableFeature(page, "workflows");
  await page.getByTestId("open-workflows-view").click();
  await page.waitForTimeout(500);
  await page.screenshot({
    path: "tests/e2e/screenshots/view-workflows.png",
    fullPage: false,
  });
});

test("screenshot: Projects view (preview)", async ({ page }) => {
  await page.goto("/");
  await enableFeature(page, "projects");
  await page.getByTestId("open-projects-view").click();
  await page.waitForTimeout(500);
  await page.screenshot({
    path: "tests/e2e/screenshots/view-projects.png",
    fullPage: false,
  });
});

test("screenshot: Pulse view (preview)", async ({ page }) => {
  await page.goto("/");
  await enableFeature(page, "pulse");
  await page.getByTestId("open-pulse-view").click();
  await page.waitForTimeout(500);
  await page.screenshot({
    path: "tests/e2e/screenshots/view-pulse.png",
    fullPage: false,
  });
});

test("screenshot: Forum post list (preview)", async ({ page }) => {
  await page.goto("/");
  await enableFeature(page, "forum");
  await page.goto(`/#/channels/${WATERCOLOR_CHANNEL_ID}`);
  await expect(
    page.getByText("Release checklist: async feedback thread."),
  ).toBeVisible();
  await page.waitForTimeout(500);
  await page.screenshot({
    path: "tests/e2e/screenshots/view-forum-posts.png",
    fullPage: false,
  });
});

test("screenshot: Forum with active thread (preview)", async ({ page }) => {
  await page.goto("/");
  await enableFeature(page, "forum");
  await page.goto(
    `/#/channels/${WATERCOLOR_CHANNEL_ID}/posts/${FORUM_POST_ID}`,
  );
  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
  await page.waitForTimeout(600);
  await page.screenshot({
    path: "tests/e2e/screenshots/view-forum-thread.png",
    fullPage: false,
  });
});

test("screenshot: Settings → Compute (stable)", async ({ page }) => {
  await page.goto("/");
  await openSettings(page);
  await page.getByTestId("settings-nav-compute").click();
  await page.waitForTimeout(500);
  const view = page.getByTestId("settings-view");
  await view.screenshot({ path: "tests/e2e/screenshots/settings-compute.png" });
});

// --- Experiments panel ---

test("screenshot: Settings → Experiments (default)", async ({ page }) => {
  await page.goto("/");
  await openSettings(page);
  await page.getByTestId("settings-nav-experimental").click();
  await expect(page.getByTestId("settings-experimental")).toBeVisible();
  await page.waitForTimeout(500);
  const view = page.getByTestId("settings-view");
  await view.screenshot({
    path: "tests/e2e/screenshots/settings-experiments-default.png",
  });
});

test("screenshot: Settings → Experiments (all on)", async ({ page }) => {
  await page.goto("/");
  await openSettings(page);
  await page.getByTestId("settings-nav-experimental").click();
  await expect(page.getByTestId("settings-experimental")).toBeVisible();
  await page.getByTestId("feature-toggle-workflows").click();
  await page.getByTestId("feature-toggle-projects").click();
  await page.getByTestId("feature-toggle-pulse").click();
  await page.getByTestId("feature-toggle-forum").click();
  await page.waitForTimeout(500);
  const view = page.getByTestId("settings-view");
  await view.screenshot({
    path: "tests/e2e/screenshots/settings-experiments-all-on.png",
  });
});
