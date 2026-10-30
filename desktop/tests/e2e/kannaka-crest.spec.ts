/**
 * The crest actually reaches the page.
 *
 * WHY THIS EXISTS
 * ---------------
 * The crest is injected at runtime by features/kannaka/mount.ts rather than
 * imported by a component, which is what stopped it conflicting on every
 * upstream sync. The cost of that trade is that every way it can break is
 * SILENT: a renamed asset, a dropped script tag in index.html, a `:has()`
 * anchor that no longer matches — none of them throw. The build stays green and
 * onboarding just quietly looks like stock Buzz.
 *
 * check-kannaka-theme.mjs asserts the chain is wired and mount.test.mjs asserts
 * the injection logic, but neither observes a real page. Until this spec existed
 * nothing in CI would have caught the crest simply being absent.
 *
 * Lives in its own file on purpose: our brand pins inside upstream's
 * identity-lost.spec.ts are themselves a recurring merge-conflict surface, and
 * a Kannaka-owned spec adds none.
 */

import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/** Artwork geometry, mirroring src/features/kannaka/crest.ts. */
const ARTWORK_WIDTH = 777;
const CROWN_HEADROOM = 70;

test("the Kannaka crest is mounted over the landing wordmark", async ({
  page,
}) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await expect(page.getByTestId("machine-onboarding-gate")).toBeVisible();

  const crest = page.locator(".kannaka-crest");
  const wordmark = page.locator('img[src$="buzz-wordmark.png"]');

  // Present at all — the failure the runtime mount makes possible.
  await expect(wordmark).toBeVisible();
  await expect(crest).toHaveCount(1);
  await expect(crest).toBeVisible();

  // It is the lockup, not an empty SVG: the word has to be in there.
  await expect(crest.locator("title")).toHaveText("Kannaka");

  // Geometry is asserted through expect.poll rather than a single read. The
  // landing runs a `mask-reveal-up` transition, and boundingBox() reflects
  // in-flight transforms — measuring mid-animation reports the image a few
  // pixels off its resting position. That is a slow-runner race, not a layout
  // bug, so the fix is to wait for it to settle rather than widen the
  // tolerance until the real failure could slip through too.
  //
  // Returns the WORST deviation across all three geometric claims so one poll
  // covers them together:
  //   - width/x: the crest spans the artwork. Drift here means
  //     `min(100%, 600px)` stopped tracking upstream's `w-full max-w-[600px]`.
  //   - lift: the crown rises above the artwork by exactly the headroom. This
  //     is the load-bearing one — it proves the translateY resolved against the
  //     crest's own height and not the parent flex column's, which is the bug
  //     that would put the crown somewhere unrelated. Resolving against the
  //     parent would be wrong by tens of pixels, so a 2px bound still catches
  //     it with room to spare.
  await expect
    .poll(
      async () => {
        const c = await crest.boundingBox();
        const w = await wordmark.boundingBox();
        if (!c || !w || c.width === 0 || c.height === 0) {
          return Number.POSITIVE_INFINITY;
        }
        const expectedLift = (w.width * CROWN_HEADROOM) / ARTWORK_WIDTH;
        return Math.max(
          Math.abs(c.width - w.width),
          Math.abs(c.x - w.x),
          Math.abs(w.y - c.y - expectedLift),
        );
      },
      { message: "crest geometry never settled against the wordmark" },
    )
    .toBeLessThanOrEqual(2);

  // Directional check, stated separately so a failure reads plainly as "the
  // crown is not above the artwork" rather than as a tolerance number.
  const crestBox = await crest.boundingBox();
  const wordBox = await wordmark.boundingBox();
  if (!crestBox || !wordBox) throw new Error("crest or wordmark has no box");
  expect(crestBox.y).toBeLessThan(wordBox.y);
});

test("the crest is not injected twice across a re-render", async ({ page }) => {
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  await expect(page.getByTestId("machine-onboarding-gate")).toBeVisible();
  await expect(page.locator(".kannaka-crest")).toHaveCount(1);

  // Advancing the flow re-renders the landing subtree, which fires the
  // MutationObserver again. Exactly one crest must survive that.
  // Upstream's flow now explains the key on a content card before creating
  // it; reaching that card is the re-render this test is about.
  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await expect(page.getByTestId("onboarding-content-card")).toBeVisible();

  await expect(page.locator(".kannaka-crest")).toHaveCount(
    await page.locator('img[src$="buzz-wordmark.png"]').count(),
  );
});
