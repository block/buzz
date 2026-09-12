/**
 * Regression test: a long message-author display name must truncate instead
 * of running past the message header into the timeline edge and the hover
 * action bar. The author button sits inside the UserProfilePopover trigger
 * wrapper — a flex item whose `min-width: auto` refused to shrink below the
 * nowrap width of the name, so the button's `truncate` never engaged.
 *
 * Run: pnpm build:e2e && pnpm exec playwright test --project=smoke \
 *        tests/e2e/message-author-overlap.spec.ts
 * Output: test-results/message-author-overlap/
 */
import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/message-author-overlap";

// Long single-word name: no break opportunities, so the nowrap min-content
// width far exceeds the header row at a 900px viewport.
const LONG_AUTHOR_NAME =
  "Alexandrina-Wolfeschlegelsteinhausenbergerdorff-Keihanaikukauakahihuliheekahaunaele-The-Magnificent-Third";

const MESSAGE_CONTENT = "Author-name truncation regression probe";

test.describe("message author name overflow", () => {
  test.use({ viewport: { width: 900, height: 700 } });

  test("long author name truncates instead of overflowing the header row", async ({
    page,
  }) => {
    await installMockBridge(page, {
      mode: "mock",
      searchProfiles: [
        {
          pubkey: TEST_IDENTITIES.alice.pubkey,
          displayName: LONG_AUTHOR_NAME,
        },
      ],
    });

    await page.goto("/", { waitUntil: "domcontentloaded" });
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await page.waitForFunction(
      () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
    );
    await page.evaluate(
      ({ pubkey, content }) => {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "general",
          content,
          pubkey,
        });
      },
      { content: MESSAGE_CONTENT, pubkey: TEST_IDENTITIES.alice.pubkey },
    );

    const row = page
      .getByTestId("message-row")
      .filter({ hasText: MESSAGE_CONTENT });
    await expect(row).toBeVisible();
    const author = row.getByTestId("message-author");
    await expect(author).toHaveText(LONG_AUTHOR_NAME);
    await waitForAnimations(page);

    // The ellipsis carrier is the popover-trigger <button> around the author
    // span — `truncate` on the inline span itself cannot clip.
    const button = author.locator("xpath=ancestor::button[1]");
    await expect(button).toBeVisible();

    const buttonBox = await button.boundingBox();
    const headerBox = await row.getByTestId("message-header").boundingBox();
    if (!buttonBox || !headerBox) {
      throw new Error("Author button or message header not rendered.");
    }

    // Guard against a vacuous pass: the author must still render with real
    // width, and the squeeze must actually engage — the button's ellipsis is
    // active only when its content overflows. If layout constants or fonts
    // ever change so the full name fits, this fails loudly instead of
    // silently no longer testing truncation.
    expect(buttonBox.width).toBeGreaterThan(0);
    const isTruncated = await button.evaluate(
      (element) => element.scrollWidth > element.clientWidth,
    );
    expect(isTruncated).toBe(true);

    await page.screenshot({
      clip: {
        height: 72,
        width: 900,
        x: 0,
        y: Math.max(0, headerBox.y - 12),
      },
      path: `${SHOTS}/01-author-header.png`,
    });

    // The name's right edge must stay inside the header row instead of
    // running under the pane edge / hover action bar.
    expect(buttonBox.x + buttonBox.width).toBeLessThanOrEqual(
      headerBox.x + headerBox.width + 1,
    );

    // The action rail (reactions/reply controls) is absolutely positioned OVER
    // the row and appears on hover and on focus-within. In both states the
    // author's painted box must end before the rail's left edge — a name that
    // merely stays inside the header can still be painted over by the rail.
    const actionBar = row.locator('[data-testid^="message-action-bar-"]');

    const expectAuthorClearOfActionBar = async (state: string) => {
      // Playwright's toBeVisible() passes at opacity 0; the rail's shown/hidden
      // states are driven by opacity, so assert the computed style directly.
      await expect(actionBar).toHaveCSS("opacity", "1");
      const authorBox = await button.boundingBox();
      const actionBarBox = await actionBar.boundingBox();
      if (!authorBox || !actionBarBox) {
        throw new Error(`Author or action bar not rendered on ${state}.`);
      }
      expect(actionBarBox.width).toBeGreaterThan(0);
      // The squeeze must still be a real ellipsis, not a vacuous fit.
      const stillTruncated = await button.evaluate(
        (element) => element.scrollWidth > element.clientWidth,
      );
      expect(stillTruncated).toBe(true);
      expect(authorBox.x + authorBox.width).toBeLessThanOrEqual(actionBarBox.x);
    };

    // Pointer hover.
    await row.hover();
    await waitForAnimations(page);
    await page.screenshot({
      clip: {
        height: 72,
        width: 900,
        x: 0,
        y: Math.max(0, headerBox.y - 12),
      },
      path: `${SHOTS}/02-author-header-hovered.png`,
    });
    await expectAuthorClearOfActionBar("hover");

    // Reset the pointer, then re-reveal the rail via keyboard focus-within.
    await page.mouse.move(0, 0);
    await expect(actionBar).toHaveCSS("opacity", "0");
    await button.focus();
    await waitForAnimations(page);
    await page.screenshot({
      clip: {
        height: 72,
        width: 900,
        x: 0,
        y: Math.max(0, headerBox.y - 12),
      },
      path: `${SHOTS}/03-author-header-focused.png`,
    });
    await expectAuthorClearOfActionBar("focus-within");
  });
});
