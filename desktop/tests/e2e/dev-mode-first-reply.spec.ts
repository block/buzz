import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// Every prompt card in the developer-mode transcript shows the agent's first
// response inline — regardless of how old the prompt is — while any later
// side-chat messages stay collapsed behind a "… N more replies" affordance
// that opens the thread pane.

test("dev mode shows the first thread reply inline and collapses the rest", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();

  const channelName = "general";
  const rootText = "prompt: investigate the flaky deploy";
  const firstReplyText = "agent first response: found the flaky step";
  const laterReplyText = "side chat follow-up: can you elaborate?";
  const thirdReplyText = "agent elaboration: it is the cache key";

  // Seed a root prompt plus a thread under it before the channel window is
  // fetched, so the snapshot carries the 39005 summary for the root.
  await page.evaluate(
    ({
      channel,
      agent,
      rootText,
      firstReplyText,
      laterReplyText,
      thirdReplyText,
    }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) throw new Error("mock bridge missing");
      const now = Math.floor(Date.now() / 1000);
      const root = emit({
        channelName: channel,
        content: rootText,
        createdAt: now - 40,
      });
      emit({
        channelName: channel,
        content: firstReplyText,
        parentEventId: root.id,
        pubkey: agent,
        createdAt: now - 30,
      });
      emit({
        channelName: channel,
        content: laterReplyText,
        parentEventId: root.id,
        createdAt: now - 20,
      });
      emit({
        channelName: channel,
        content: thirdReplyText,
        parentEventId: root.id,
        pubkey: agent,
        createdAt: now - 10,
      });
    },
    {
      channel: channelName,
      agent: TEST_IDENTITIES.bob.pubkey,
      rootText,
      firstReplyText,
      laterReplyText,
      thirdReplyText,
    },
  );

  // ArrowUp steps through channel previews newest-first; walk until the
  // seeded channel is previewed, then Enter opens it.
  await composer.focus();
  const topBar = page.getByTestId("dev-mode-topbar-channel");
  for (let step = 0; step < 20; step += 1) {
    await page.keyboard.press("ArrowUp");
    const previewed = (await topBar.innerText()).replace(/^#\s*/, "").trim();
    if (previewed === channelName) break;
  }
  await expect(topBar).toContainText(channelName);
  await page.keyboard.press("Enter");
  await page.getByTestId("dev-mode-transcript").waitFor();

  const card = page
    .getByTestId("dev-mode-prompt-card")
    .filter({ hasText: rootText });
  await expect(card).toBeVisible();

  // The first reply renders inline; later thread messages do not.
  await expect(card).toContainText(firstReplyText);
  await expect(card).not.toContainText(laterReplyText);
  await expect(card).not.toContainText(thirdReplyText);

  // The collapsed indicator counts only the messages after the first reply
  // and never labels the first response itself as a hidden reply.
  const more = card.getByTestId("dev-mode-more-replies");
  await expect(more).toHaveText(/…\s*2 more replies/);

  // Opening the thread shows the complete conversation in the side pane.
  await more.click();
  const threadPanel = page.getByTestId("dev-mode-thread-panel");
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel).toContainText(firstReplyText);
  await expect(threadPanel).toContainText(laterReplyText);
  await expect(threadPanel).toContainText(thirdReplyText);
});
