import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

const SAMPLE_NSEC =
  "nsec1u70xptkumvfc4k4hu0rc4fnzcexvw63zvq2ng9vmqujsaayhparqu8eju9";

// --buzz-onboarding-backup-ink (#717106), the olive key ink shared with the
// backup step.
const BACKUP_INK = "rgb(113, 113, 6)";

test("key import masks the key with a reveal toggle", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await page.getByRole("button", { name: "Use an existing key" }).click();
  const input = page.getByTestId("nostr-import-nsec-input");
  await expect(input).toBeVisible();
  await waitForAnimations(page);

  // Masked by default; no toggle until there is input; key text uses the
  // shared backup ink.
  const toggle = page.getByTestId("nostr-import-reveal-toggle");
  await expect(input).toHaveAttribute("type", "password");
  await expect(toggle).toHaveCSS("opacity", "0");
  await expect(input).toHaveCSS("color", BACKUP_INK);

  // The toggle is absolutely positioned: its appearance must not resize the
  // input or shift the centered text.
  const widthBefore = await input.evaluate(
    (el) => el.getBoundingClientRect().width,
  );
  await input.fill(SAMPLE_NSEC);
  await expect(toggle).toHaveCSS("opacity", "1");
  const widthAfter = await input.evaluate(
    (el) => el.getBoundingClientRect().width,
  );
  expect(widthAfter).toBe(widthBefore);

  // Reveal, then clear: a sticky reveal must never carry over to newly
  // pasted content, so the next key starts masked again.
  await toggle.click();
  await expect(input).toHaveAttribute("type", "text");
  await input.fill("");
  await expect(toggle).toHaveCSS("opacity", "0");
  await input.fill(SAMPLE_NSEC);
  await expect(input).toHaveAttribute("type", "password");

  // Re-masking via the toggle still works.
  await toggle.click();
  await expect(input).toHaveAttribute("type", "text");
  await toggle.click();
  await expect(input).toHaveAttribute("type", "password");

  // Narrow viewport: the absolutely positioned toggle must not cause
  // horizontal overflow.
  await page.setViewportSize({ width: 720, height: 620 });
  await waitForAnimations(page);
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth > window.innerWidth,
  );
  expect(overflow).toBe(false);
});
