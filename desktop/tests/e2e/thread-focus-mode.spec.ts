import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

async function seedLongThread(page: import("@playwright/test").Page) {
  await expect
    .poll(() =>
      page.evaluate(
        () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
      ),
    )
    .toBe(true);
  return page.evaluate(() => {
    const root = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: "Focus mode integration thread",
      createdAt: 1_700_900_000,
    });
    if (!root) throw new Error("Failed to seed focus thread root");

    for (let index = 0; index < 48; index += 1) {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `Focus reply ${index}: this deliberately wraps across several lines so changing the thread measure causes real layout reflow.`,
        parentEventId: root.id,
        createdAt: 1_700_900_001 + index,
      });
    }
    return root.id;
  });
}

async function topVisibleMessageId(
  body: import("@playwright/test").Locator,
): Promise<string> {
  return body.evaluate((element) => {
    const top = element.getBoundingClientRect().top;
    const row = Array.from(
      element.querySelectorAll<HTMLElement>("[data-message-id]"),
    ).find((candidate) => candidate.getBoundingClientRect().bottom > top);
    if (!row?.dataset.messageId) throw new Error("No visible thread anchor");
    return row.dataset.messageId;
  });
}

/**
 * The channel header must own its pixels, not merely be "visible".
 *
 * Regression guard for the focus-mode launch: a `z-0` on the channel section
 * created a stacking context that flattened the header's `z-30` beneath the
 * sibling shared header backdrop (also `z-30`), painting the backdrop over the
 * name and actions. Neither `toBeVisible()` nor `elementFromPoint` can catch
 * this — the backdrop is `pointer-events-none`, so hit-testing skips it. We
 * compare CSS paint order directly: walk each element's stacking-context
 * chain, find the branches under their common stacking context, and check the
 * header's branch wins (higher z-index, or later in DOM order on a tie).
 */
async function expectChannelHeaderUnobscured(
  page: import("@playwright/test").Page,
) {
  const title = page.getByTestId("chat-title");
  await expect(title).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const titleEl = document.querySelector('[data-testid="chat-title"]');
        const backdropEl = document.querySelector(
          '[data-testid="channel-shared-header-backdrop"]',
        );
        if (!titleEl) return "missing title";
        if (!backdropEl) return "missing backdrop";

        const createsStackingContext = (el: Element): boolean => {
          const style = getComputedStyle(el);
          if (style.position !== "static" && style.zIndex !== "auto")
            return true;
          if (parseFloat(style.opacity) < 1) return true;
          if (style.transform !== "none") return true;
          if (style.filter !== "none") return true;
          const backdropFilter =
            style.backdropFilter ??
            (style as unknown as { webkitBackdropFilter?: string })
              .webkitBackdropFilter;
          if (backdropFilter && backdropFilter !== "none") return true;
          if (style.isolation === "isolate") return true;
          if (
            style.contain.includes("paint") ||
            style.contain.includes("strict")
          )
            return true;
          return false;
        };

        // Chain of stacking-context roots from the element up to <html>.
        const stackingChain = (el: Element): Element[] => {
          const chain: Element[] = [el];
          let current: Element | null = el.parentElement;
          while (current) {
            if (
              createsStackingContext(current) ||
              current === document.documentElement
            ) {
              chain.push(current);
            }
            current = current.parentElement;
          }
          return chain;
        };

        const titleChain = stackingChain(titleEl);
        const backdropChain = stackingChain(backdropEl);
        const common = titleChain.find((el) => backdropChain.includes(el));
        if (!common) return "no common stacking context";

        // Branch = the child-of-common entry each element paints through.
        const titleBranch = titleChain[titleChain.indexOf(common) - 1];
        const backdropBranch = backdropChain[backdropChain.indexOf(common) - 1];
        if (!titleBranch || !backdropBranch) return "degenerate chain";

        const effectiveZ = (el: Element): number => {
          const z = getComputedStyle(el).zIndex;
          return z === "auto" ? 0 : parseInt(z, 10);
        };
        const titleZ = effectiveZ(titleBranch);
        const backdropZ = effectiveZ(backdropBranch);
        if (titleZ !== backdropZ) {
          return titleZ > backdropZ ? true : "backdrop paints above header";
        }
        // Tie: later in DOM order paints on top.
        const order = backdropBranch.compareDocumentPosition(titleBranch);
        return (order & Node.DOCUMENT_POSITION_FOLLOWING) !== 0
          ? true
          : "backdrop paints above header";
      }),
    )
    .toBe(true);
}

test("focus and split preserve reading context and interaction ownership", async ({
  page,
}) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.addInitScript(() => {
    localStorage.setItem("buzz.channels.threadViewMode", "focus");
  });
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey:
          "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f",
        name: "alice",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");
  const rootId = await seedLongThread(page);

  await page.getByTestId("channel-general").click();
  await expectChannelHeaderUnobscured(page);
  const summary = page.locator(
    `[data-testid="message-thread-summary"][data-thread-head-id="${rootId}"]`,
  );
  await expect(summary).toBeVisible();
  await summary.click();

  const channel = page.getByTestId("channel-drop-zone");
  const drawer = page.getByTestId("focus-thread-drawer");
  const body = page.getByTestId("message-thread-body");
  await expect(drawer).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() =>
        Boolean(
          document
            .querySelector('[data-testid="focus-thread-drawer"]')
            ?.contains(document.activeElement),
        ),
      ),
    )
    .toBe(true);
  await expect(channel).toHaveAttribute("inert", "");

  await body.evaluate((element) => {
    element.scrollTop = element.scrollHeight * 0.4;
    element.dispatchEvent(new Event("scroll", { bubbles: true }));
  });
  const anchorId = await topVisibleMessageId(body);

  await page
    .getByRole("button", { name: "Show thread beside channel" })
    .click();
  await expect(drawer).toHaveCount(0);
  await expect(channel).not.toHaveAttribute("inert", "");
  await expectChannelHeaderUnobscured(page);
  await expect(page.getByTestId("thread-view-mode-toggle")).toBeFocused();
  await expect(
    body.locator(`[data-message-id="${anchorId}"]`),
  ).toBeInViewport();
  await expect(
    body.locator(`[data-message-id="${anchorId}"]`),
  ).not.toHaveAttribute("data-highlighted", "true");

  await page.getByRole("button", { name: "Expand thread" }).click();
  await expect(drawer).toBeVisible();
  await expect(channel).toHaveAttribute("inert", "");
  await expect(page.getByTestId("thread-view-mode-toggle")).toBeFocused();
  await expect(
    body.locator(`[data-message-id="${anchorId}"]`),
  ).toBeInViewport();

  // Focus mode owns Escape even while the rich-text composer and one of its
  // nested controls has focus: one press exits the focused thread.
  const threadInput = page
    .getByTestId("message-thread-panel")
    .getByTestId("message-input");
  await threadInput.click();
  await threadInput.pressSequentially("@al");
  await expect(
    page
      .getByTestId("message-thread-panel")
      .getByTestId("mention-autocomplete"),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("focus-thread-drawer-overlay")).toHaveCount(0);
  await expect(channel).not.toHaveAttribute("inert", "");

  await summary.click();
  await expect(drawer).toBeVisible();
  await page.getByTestId("focus-thread-drawer-scrim").click({
    position: { x: 24, y: 200 },
  });
  await expect(page.getByTestId("focus-thread-drawer-overlay")).toHaveCount(0);
  await expect(channel).not.toHaveAttribute("inert", "");
});

test("narrow threads do not offer an unavailable layout switch", async ({
  page,
}) => {
  await page.setViewportSize({ width: 860, height: 720 });
  await installMockBridge(page);
  await page.goto("/");
  const rootId = await seedLongThread(page);
  await page.getByTestId("channel-general").click();
  const summary = page.locator(
    `[data-testid="message-thread-summary"][data-thread-head-id="${rootId}"]`,
  );
  await expect(summary).toBeVisible();
  await summary.click();
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
  await expect(page.getByTestId("thread-view-mode-toggle")).toHaveCount(0);
});
