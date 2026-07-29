import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

// Developer mode must show who is in the chat: the top bar lists the
// channel's members, and member join/leave system messages (kind:40099)
// render as narration rows in the transcript.

test("dev mode lists channel members and narrates joins/leaves", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.addInitScript(() => {
    localStorage.setItem("buzz.displayStyle", "developer");
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });

  const composer = page.getByTestId("dev-mode-composer");
  await composer.waitFor();

  // ArrowUp steps through channel previews newest-first; walk until the
  // multi-member mock channel ("general": tyler, alice, bob, mira) is
  // previewed, then Enter opens it.
  await composer.focus();
  const topBar = page.getByTestId("dev-mode-topbar-channel");
  const channelName = "general";
  for (let step = 0; step < 20; step += 1) {
    await page.keyboard.press("ArrowUp");
    const previewed = (await topBar.innerText()).replace(/^#\s*/, "").trim();
    if (previewed === channelName) break;
  }
  await expect(topBar).toContainText(channelName);
  await page.keyboard.press("Enter");
  await page.getByTestId("dev-mode-transcript").waitFor();

  const members = page.getByTestId("dev-mode-channel-members");
  await expect(members).toContainText("alice");
  await expect(members).toContainText("bob");

  await page.waitForFunction(
    (name) =>
      window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
        channelName: name,
      }) ?? false,
    channelName,
  );

  const membershipRows = page.getByTestId("dev-mode-membership-row");

  await page.evaluate(
    ({ channel, actor }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: channel,
        kind: 40099,
        pubkey: actor,
        content: JSON.stringify({ type: "member_left", actor }),
      });
    },
    { channel: channelName, actor: TEST_IDENTITIES.bob.pubkey },
  );
  await expect(membershipRows.filter({ hasText: "bob left" })).toBeVisible();

  await page.evaluate(
    ({ channel, target }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: channel,
        kind: 40099,
        pubkey: target,
        content: JSON.stringify({
          type: "member_joined",
          actor: target,
          target,
        }),
      });
    },
    { channel: channelName, target: TEST_IDENTITIES.bob.pubkey },
  );
  await expect(membershipRows.filter({ hasText: "bob joined" })).toBeVisible();
});
