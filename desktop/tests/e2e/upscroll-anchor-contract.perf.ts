import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

/**
 * ANCHOR-CONTRACT GUARD (Wren #4) — companion to `upscroll-jitter.perf.ts`.
 *
 * The jitter gate *forces* `overflow-anchor: none` on the scroller before it
 * measures, to reproduce shipped WKWebView (which has no `overflow-anchor` and
 * so corrects nothing) on anchoring Chromium. That force is a test convenience,
 * not a proof: it would stay green even if production ever stopped shipping the
 * property, silently handing correction back to Chromium's native anchoring and
 * masking a WKWebView-only regression on device.
 *
 * This test closes that gap by reading the PRODUCTION computed style — no test
 * override — on the real conversation scroller and asserting it resolves to
 * `none`. We run on Chromium, whose default `overflow-anchor` is `auto`, so a
 * resolved value of `none` can only have come from the shipped stylesheet
 * (`utilities.css` `[data-buzz-conversation-scroll]`). The engine-support
 * assert keeps the check honest: if it ever runs somewhere without the property
 * at all, we want the loud failure, not a vacuous pass on an empty string.
 */
test("CONTRACT: production ships overflow-anchor:none on the conversation scroller", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  await page.getByTestId("channel-jitter-corpus").click();
  await expect(page.getByTestId("chat-title")).toHaveText("jitter-corpus");
  const timeline = page.getByTestId("message-timeline");
  await expect(timeline.locator("[data-message-id]").first()).toBeVisible();

  // Sanity: Chromium DOES support overflow-anchor (default `auto`), so a
  // resolved `none` below is the production stylesheet's doing, not the engine
  // returning an empty/unsupported value.
  const anchorSupported = await page.evaluate(() =>
    typeof CSS !== "undefined" && typeof CSS.supports === "function"
      ? CSS.supports("overflow-anchor", "auto")
      : false,
  );
  expect(anchorSupported).toBe(true);

  const resolvedOverflowAnchor = await timeline.evaluate(
    (element) => getComputedStyle(element).overflowAnchor,
  );
  expect(resolvedOverflowAnchor).toBe("none");
});
