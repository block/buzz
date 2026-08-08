import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const VAULT = "/mock/vault";

/**
 * Opens Documents against a seeded vault.
 *
 * Seeding waits for the bridge helper rather than calling it optionally — the
 * helpers are assigned during app bootstrap, which can land after `goto`
 * resolves, and an optional call would silently no-op.
 */
async function openVaultWith(
  page: Page,
  files: Array<[path: string, content: string]>,
) {
  await page.addInitScript(() => {
    window.localStorage.removeItem("buzz.documents.vaultPath.v1");
  });
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_MOCK_VAULT_FILE__ === "function",
  );
  await page.evaluate((entries) => {
    const seed = window.__BUZZ_E2E_SEED_MOCK_VAULT_FILE__;
    if (!seed) throw new Error("mock vault seed helper is unavailable");
    for (const [path, content] of entries) seed(path, content);
  }, files);

  await page.getByTestId("open-documents-view").click();
  await page.getByTestId("documents-choose-vault").click();
  await expect(page.getByTestId("documents-tree")).toBeVisible();
}

function editorSurface(page: Page) {
  return page.locator('[data-testid="documents-live-editor"] .ProseMirror');
}

test.describe("Documents wikilinks", () => {
  test("a resolving wikilink opens the linked note when clicked", async ({
    page,
  }) => {
    await openVaultWith(page, [
      [`${VAULT}/Hub.md`, "# Hub\n\nGo to [[Target]] now.\n"],
      [`${VAULT}/Target.md`, "# Target\n\nArrived.\n"],
    ]);

    await page.getByTestId("documents-file-Hub.md").click();
    const link = editorSurface(page).locator(".wikilink").first();
    await expect(link).toBeVisible();
    await expect(link).not.toHaveClass(/wikilink-broken/);

    await link.click();

    // Clicking opens the target in its own tab, without closing the source.
    await expect(page.getByTestId("documents-tab-Target")).toBeVisible();
    await expect(page.getByTestId("documents-tab-Hub")).toBeVisible();
    await expect(editorSurface(page)).toContainText("Arrived.");
  });

  test("a link to a missing note renders as broken and does not navigate", async ({
    page,
  }) => {
    await openVaultWith(page, [
      [`${VAULT}/Hub.md`, "# Hub\n\nSee [[Nowhere]].\n"],
    ]);

    await page.getByTestId("documents-file-Hub.md").click();
    const link = editorSurface(page).locator(".wikilink").first();
    await expect(link).toHaveClass(/wikilink-broken/);

    await link.click();
    // Clicking a broken link must not create a note or open a tab.
    await expect(page.getByTestId("documents-tab-Nowhere")).toHaveCount(0);
  });

  test("wikilinks survive a save unescaped", async ({ page }) => {
    // The serializer escapes brackets; if the escape-stripping regressed, the
    // file on disk would gain backslashes and break in Obsidian.
    await openVaultWith(page, [
      [`${VAULT}/Hub.md`, "# Hub\n\nSee [[Target]].\n"],
      [`${VAULT}/Target.md`, "# Target\n"],
    ]);

    await page.getByTestId("documents-file-Hub.md").click();
    await editorSurface(page).click();
    await page.keyboard.press("End");
    await page.keyboard.type(" Edited.");
    await page.keyboard.press("ControlOrMeta+s");
    await expect(page.getByTestId("documents-tab-dirty-Hub")).toHaveCount(0, {
      timeout: 3_000,
    });

    const saved = await page.evaluate(
      () =>
        window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("read_vault_file", {
          path: "/mock/vault/Hub.md",
        }) as unknown as string,
    );
    expect(saved).toContain("[[Target]]");
    expect(saved).not.toContain("\\[\\[");
  });
});

test.describe("Documents backlinks", () => {
  test("lists linked and unlinked mentions separately", async ({ page }) => {
    await openVaultWith(page, [
      [`${VAULT}/Target.md`, "# Target\n"],
      [`${VAULT}/Linker.md`, "Refers to [[Target]] properly.\n"],
      [`${VAULT}/Mentioner.md`, "Just says Target in passing.\n"],
    ]);

    await page.getByTestId("documents-file-Target.md").click();
    await expect(page.getByTestId("documents-backlinks")).toBeVisible();

    await expect(page.getByTestId("documents-linked-mentions")).toContainText(
      "Linker",
    );
    await expect(page.getByTestId("documents-unlinked-mentions")).toContainText(
      "Mentioner",
    );
    // The linking note must not also appear as an unlinked mention.
    await expect(
      page.getByTestId("documents-unlinked-mentions"),
    ).not.toContainText("Linker");
  });

  test("clicking a backlink opens that note", async ({ page }) => {
    await openVaultWith(page, [
      [`${VAULT}/Target.md`, "# Target\n"],
      [`${VAULT}/Linker.md`, "Refers to [[Target]].\n"],
    ]);

    await page.getByTestId("documents-file-Target.md").click();
    await page
      .getByTestId("documents-linked-mentions")
      .getByRole("button", { name: "Linker", exact: true })
      .click();

    await expect(page.getByTestId("documents-tab-Linker")).toBeVisible();
  });
});

test.describe("Documents outline and Obsidian syntax", () => {
  test("lists headings and scrolls to one when clicked", async ({ page }) => {
    await openVaultWith(page, [
      [
        `${VAULT}/Structured.md`,
        "# Top\n\nIntro.\n\n## Middle\n\nBody.\n\n### Deep\n\nMore.\n",
      ],
    ]);

    await page.getByTestId("documents-file-Structured.md").click();
    const outline = page.getByTestId("documents-outline");
    await expect(outline).toBeVisible();

    for (const heading of ["Top", "Middle", "Deep"]) {
      await expect(
        page.getByTestId(`documents-outline-item-${heading}`),
      ).toBeVisible();
    }

    // Clicking must not throw and must keep the note open.
    await page.getByTestId("documents-outline-item-Middle").click();
    await expect(editorSurface(page)).toContainText("Body.");
  });

  test("decorates highlights and tags in live preview", async ({ page }) => {
    // No callout here on purpose: highlights and tags round-trip cleanly, so
    // this note opens in live preview where decorations apply.
    const source = "# Syntax\n\nA ==highlight== and a #project tag.\n";
    await openVaultWith(page, [[`${VAULT}/Syntax.md`, source]]);

    await page.getByTestId("documents-file-Syntax.md").click();
    const surface = editorSurface(page);

    await expect(surface.locator(".obsidian-highlight")).toHaveCount(1);
    await expect(surface.locator(".obsidian-tag")).toHaveCount(1);
    // Decoration-only, so the guard leaves the note in live preview.
    await expect(page.getByTestId("documents-round-trip-banner")).toHaveCount(
      0,
    );
  });

  test("a callout note opens in source mode, and decorates once opted in", async ({
    page,
  }) => {
    // Callouts are decorated but do NOT round-trip: the serializer merges the
    // blockquote's lines. The guard therefore opens the note in source mode,
    // and callout styling only appears if the user opts into live preview.
    // Pinning both halves so a future serializer fix is a visible change here.
    const source = "# Doc\n\n> [!warning] Careful\n> Body text.\n";
    await openVaultWith(page, [[`${VAULT}/Callout.md`, source]]);

    await page.getByTestId("documents-file-Callout.md").click();
    await expect(page.getByTestId("documents-round-trip-banner")).toBeVisible();
    await expect(page.getByTestId("documents-source-editor")).toBeVisible();

    // The header toggle is the single control for switching modes; the notice
    // itself is informational only.
    await page.getByTestId("documents-toggle-view-mode").click();
    await expect(editorSurface(page).locator(".callout-warning")).toHaveCount(
      1,
    );
  });
});

test.describe("Documents session restore", () => {
  test("reopens the previous tabs and re-reads them from disk", async ({
    page,
  }) => {
    // Uses the bridge's default vault rather than seeded files: `resetMockVault`
    // runs on every page load, so seeded fixtures would vanish across the
    // reload this test depends on.
    await page.addInitScript(() => {
      window.localStorage.setItem("buzz.documents.vaultPath.v1", "/mock/vault");
    });
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId("open-documents-view").click();
    await expect(page.getByTestId("documents-tree")).toBeVisible();

    await page.getByTestId("documents-file-Welcome.md").click();
    await page.getByTestId("documents-folder-Notes").click();
    await page.getByTestId("documents-file-Meeting notes.md").click();
    await expect(page.getByTestId("documents-tab-Welcome")).toBeVisible();
    await expect(page.getByTestId("documents-tab-Meeting notes")).toBeVisible();

    const stored = await page.evaluate(() =>
      window.localStorage.getItem("buzz.documents.session.v1"),
    );
    expect(stored).toContain("Welcome.md");

    await page.reload();
    await page.getByTestId("open-documents-view").click();

    // Both tabs come back, and the expanded folder with them.
    await expect(page.getByTestId("documents-tab-Welcome")).toBeVisible();
    await expect(page.getByTestId("documents-tab-Meeting notes")).toBeVisible();
    await expect(
      page.getByTestId("documents-file-Meeting notes.md"),
    ).toBeVisible();
  });
});

test.describe("Documents right rail", () => {
  test("toggles the outline and backlinks rail, remembering the choice", async ({
    page,
  }) => {
    await openVaultWith(page, [[`${VAULT}/Note.md`, "# Note\n\nBody.\n"]]);
    await page.getByTestId("documents-file-Note.md").click();
    await expect(page.getByTestId("documents-outline")).toBeVisible();

    await page.getByTestId("documents-toggle-rail").click();
    await expect(page.getByTestId("documents-outline")).toHaveCount(0);
    await expect(page.getByTestId("documents-backlinks")).toHaveCount(0);

    await page.getByTestId("documents-toggle-rail").click();
    await expect(page.getByTestId("documents-outline")).toBeVisible();
  });
});

test.describe("Documents outline hygiene", () => {
  test("switching to a source-mode note clears the previous note's outline", async ({
    page,
  }) => {
    await page.addInitScript(() => {
      window.localStorage.removeItem("buzz.documents.vaultPath.v1");
    });
    await installMockBridge(page);
    await page.goto("/");

    await page.waitForFunction(
      () => typeof window.__BUZZ_E2E_SEED_MOCK_VAULT_FILE__ === "function",
    );
    await page.evaluate(() => {
      const seed = window.__BUZZ_E2E_SEED_MOCK_VAULT_FILE__;
      if (!seed) throw new Error("mock vault seed helper is unavailable");
      seed("/mock/vault/Outlined.md", "# One\n\n## Two\n\nBody.\n");
      // Raw HTML fails the round-trip guard, so this opens in source mode.
      seed("/mock/vault/Rawish.md", '# Raw\n\n<div align="center">x</div>\n');
    });

    await page.getByTestId("open-documents-view").click();
    await page.getByTestId("documents-choose-vault").click();
    await expect(page.getByTestId("documents-tree")).toBeVisible();

    await page.getByTestId("documents-file-Outlined.md").click();
    await expect(page.getByTestId("documents-outline-item-One")).toBeVisible();

    // The outline belongs to the note being viewed. A source-mode note has no
    // live editor to publish one, so the previous note's headings must not
    // linger — they would scroll into a document that is no longer mounted.
    await page.getByTestId("documents-file-Rawish.md").click();
    await expect(page.getByTestId("documents-source-editor")).toBeVisible();
    await expect(page.getByTestId("documents-outline-item-One")).toHaveCount(0);
  });
});
