import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { FEATURE_OVERRIDES_STORAGE_KEY } from "../helpers/features";

const MOCK_VAULT_PATH = "/mock/vault";

/**
 * Documents persists its vault path per-machine in localStorage. Specs must
 * start from "no vault chosen" or the empty state never renders.
 */
async function clearStoredVault(page: Page) {
  await page.addInitScript(() => {
    window.localStorage.removeItem("buzz.documents.vaultPath.v1");
  });
}

test.describe("Documents preview feature gate", () => {
  test("the sidebar entry is hidden until the feature is enabled", async ({
    page,
  }) => {
    // `addInitScript` must run before installMockBridge — React reads storage
    // on mount and the bridge triggers mount.
    await page.addInitScript(
      ([key]) => {
        window.localStorage.setItem(key, JSON.stringify({ documents: false }));
      },
      [FEATURE_OVERRIDES_STORAGE_KEY],
    );
    // The second parameter is the mock config; bridge options are the third.
    await installMockBridge(page, undefined, { seedPreviewFeatures: false });

    await page.goto("/");
    await expect(page.getByTestId("open-agents-view")).toBeVisible();
    await expect(page.getByTestId("open-documents-view")).toHaveCount(0);
  });

  test("enabling the feature surfaces Documents in the sidebar", async ({
    page,
  }) => {
    await installMockBridge(page);

    await page.goto("/");
    await expect(page.getByTestId("open-documents-view")).toBeVisible();
  });
});

test.describe("Documents vault empty state", () => {
  test.beforeEach(async ({ page }) => {
    await clearStoredVault(page);
    await installMockBridge(page);
  });

  test("prompts for a folder, then lists the vault once one is chosen", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByTestId("open-documents-view").click();
    await expect(page).toHaveURL(/#\/documents$/);

    // No vault yet, so the view asks for one rather than showing an empty tree.
    await expect(page.getByTestId("documents-empty-state")).toBeVisible();
    await expect(page.getByTestId("documents-tree")).toHaveCount(0);

    await page.getByTestId("documents-choose-vault").click();

    // The mock picker resolves to MOCK_VAULT_PATH, which activates the vault.
    await expect(page.getByTestId("documents-tree")).toBeVisible();
    await expect(page.getByTestId("documents-empty-state")).toHaveCount(0);

    // Directories sort before files, matching the Rust walker.
    await expect(page.getByTestId("documents-folder-Notes")).toBeVisible();
    await expect(page.getByTestId("documents-file-Welcome.md")).toBeVisible();

    // The stored path is the one the backend accepted.
    const storedPath = await page.evaluate(() =>
      window.localStorage.getItem("buzz.documents.vaultPath.v1"),
    );
    expect(storedPath).toBe(MOCK_VAULT_PATH);
  });

  test("a cancelled folder picker leaves the empty state in place", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByTestId("open-documents-view").click();
    await expect(page.getByTestId("documents-empty-state")).toBeVisible();

    await page.evaluate(() => {
      window.__BUZZ_E2E_SET_MOCK_VAULT_PICKER__?.(null);
    });
    await page.getByTestId("documents-choose-vault").click();

    await expect(page.getByTestId("documents-empty-state")).toBeVisible();
    const storedPath = await page.evaluate(() =>
      window.localStorage.getItem("buzz.documents.vaultPath.v1"),
    );
    expect(storedPath).toBeNull();
  });

  test("expanding a folder reveals its notes and opens one for editing", async ({
    page,
  }) => {
    await page.goto("/");
    await page.getByTestId("open-documents-view").click();
    await page.getByTestId("documents-choose-vault").click();
    await expect(page.getByTestId("documents-tree")).toBeVisible();

    // Collapsed folders contribute a row but not their subtree.
    await expect(
      page.getByTestId("documents-file-Meeting notes.md"),
    ).toHaveCount(0);

    await page.getByTestId("documents-folder-Notes").click();
    await expect(
      page.getByTestId("documents-file-Meeting notes.md"),
    ).toBeVisible();
    // The nested folder is listed but still collapsed.
    await expect(page.getByTestId("documents-folder-Archive")).toBeVisible();
    await expect(page.getByTestId("documents-file-Old note.md")).toHaveCount(0);

    // Opening a note now mounts the editor rather than a read-only preview.
    await page.getByTestId("documents-file-Meeting notes.md").click();
    await expect(page.getByTestId("documents-tab-Meeting notes")).toBeVisible();
    await expect(
      page.locator('[data-testid="documents-live-editor"] .ProseMirror'),
    ).toContainText("Ship Documents");
  });
});
