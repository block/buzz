/**
 * PR screenshots for the Documents feature.
 *
 * Each shot is scoped to its subject with `locator.screenshot()` rather than a
 * full-page capture, so two shots of the same screen cannot come out
 * byte-identical — the failure mode CLAUDE.md warns about.
 */
import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const OUT = "test-results/documents-screenshots";
const VAULT = "/mock/vault";

/** A note rich enough to show what the editor actually does with a vault. */
const RICH_NOTE = [
  "# Documents",
  "",
  "A local markdown vault, edited in place. Files stay plain `.md` on disk, so",
  "Obsidian and git see exactly what you would expect.",
  "",
  "## What works today",
  "",
  "- [x] Live preview and raw source mode",
  "- [x] Wikilinks like [[Meeting notes]], with backlinks",
  "- [ ] Callouts and footnotes",
  "",
  "## Round-trip safety",
  "",
  "Every note is parsed and re-serialized before the editor touches it. If the",
  "result would differ, the note opens in ==source mode== instead. #documents",
  "",
  "| Construct | Round-trips |",
  "| --- | --- |",
  "| Tables | yes |",
  "| Callouts | not yet |",
  "",
].join("\n");

/** A note the round-trip guard refuses, so it opens in source mode. */
const LOSSY_NOTE = [
  "# Release checklist",
  "",
  "> [!warning] Callouts do not round-trip yet",
  "> Opening this in live preview would rewrite it, so it stays in source.",
  "",
  "Raw <div align=\"center\">HTML</div> has the same problem.",
  "",
].join("\n");

/** A second note that links back, so the backlinks panel has something in it. */
const LINKING_NOTE = [
  "# Meeting notes",
  "",
  "- Ship [[Documents]] behind the preview flag",
  "- Revisit callouts after that",
  "",
].join("\n");

async function openVault(page: Page) {
  // Must run before installMockBridge: React reads this state on mount, and
  // the bridge is what triggers mount.
  await page.addInitScript(() => {
    window.localStorage.setItem(
      "buzz-feature-overrides-v1",
      JSON.stringify({ documents: true }),
    );
  });
  await installMockBridge(page);
  await page.goto("/");

  // The bridge assigns its helpers during app bootstrap, which can land after
  // `goto` resolves — waiting makes the seed deterministic rather than a
  // silently-skipped optional call.
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_MOCK_VAULT_FILE__ === "function",
  );
  await page.evaluate(
    ([vault, rich, lossy, linking]) => {
      const seed = window.__BUZZ_E2E_SEED_MOCK_VAULT_FILE__;
      if (!seed) throw new Error("mock vault seed helper is unavailable");
      seed(`${vault}/Documents.md`, rich);
      seed(`${vault}/Release checklist.md`, lossy);
      seed(`${vault}/Notes/Meeting notes.md`, linking);
    },
    [VAULT, RICH_NOTE, LOSSY_NOTE, LINKING_NOTE] as const,
  );

  await page.getByTestId("open-documents-view").click();
  await page.getByTestId("documents-choose-vault").click();
  await expect(page.getByTestId("documents-tree")).toBeVisible();
}

/** The Documents surface itself, without the community sidebar. */
function surface(page: Page) {
  return page.locator("main").last();
}

test("documents screenshots", async ({ page }) => {
  await openVault(page);

  // 1 — a note in live preview, with the outline and backlinks rail.
  await page.getByTestId("documents-file-Documents.md").click();
  await expect(page.getByTestId("documents-live-editor")).toBeVisible();
  await waitForAnimations(page);
  await surface(page).screenshot({ path: `${OUT}/01-live-preview.png` });

  // 2 — backlinks: the note that links here shows up as a linked mention.
  await expect(page.getByTestId("documents-backlinks")).toBeVisible();
  await waitForAnimations(page);
  await page
    .getByTestId("documents-backlinks")
    .screenshot({ path: `${OUT}/02-backlinks.png` });

  // 3 — a note the round-trip guard refuses, opened in source mode.
  await page.getByTestId("documents-file-Release checklist.md").click();
  await expect(page.getByTestId("documents-source-editor")).toBeVisible();
  await waitForAnimations(page);
  await surface(page).screenshot({ path: `${OUT}/03-source-mode.png` });
});
