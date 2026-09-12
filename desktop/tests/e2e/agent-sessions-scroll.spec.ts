import { expect, test } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301";
const SESSION_ID = "long-session-transcript";

test.use({ viewport: { width: 1000, height: 600 } });

test("selected long session transcript scrolls", async ({ page }) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: AGENT_PUBKEY,
        name: "Scrollable Agent",
        status: "running",
        channelNames: ["agents"],
      },
    ],
  });
  await page.goto("/#/agent-sessions", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ === "function",
  );

  const events = Array.from({ length: 36 }, (_, index) => ({
    seq: index + 1,
    timestamp: new Date(Date.UTC(2025, 5, 15, 12, 0, index)).toISOString(),
    kind: "acp_read",
    agentIndex: 0,
    channelId: CHANNEL_ID,
    sessionId: SESSION_ID,
    turnId: `turn-${index + 1}`,
    payload: {
      method: "session/update",
      params: {
        sessionId: SESSION_ID,
        update: {
          sessionUpdate: "agent_message_chunk",
          messageId: `message-${index + 1}`,
          content: {
            type: "text",
            text: `Transcript entry ${index + 1}: ${"content ".repeat(12)}`,
          },
        },
      },
    },
  }));
  await page.evaluate(
    ({ agentPubkey, observerEvents }) => {
      window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
        agentPubkey,
        events: observerEvents,
      });
    },
    { agentPubkey: AGENT_PUBKEY, observerEvents: events },
  );

  await page.getByRole("button", { name: /long-ses/ }).click();
  const transcript = page.getByRole("log", { name: "Live ACP transcript" });
  await expect(transcript).toBeVisible();
  const scrollContainer = transcript.locator("../..");

  await expect
    .poll(() =>
      scrollContainer.evaluate((element) => ({
        overflowY: getComputedStyle(element).overflowY,
        scrollable: element.scrollHeight > element.clientHeight,
      })),
    )
    .toEqual({ overflowY: "auto", scrollable: true });

  await scrollContainer.evaluate((element) => {
    element.scrollTop = element.scrollHeight;
  });
  await expect
    .poll(() => scrollContainer.evaluate((element) => element.scrollTop))
    .toBeGreaterThan(0);
});
