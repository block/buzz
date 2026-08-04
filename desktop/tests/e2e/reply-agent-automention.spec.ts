import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const AGENT = "a".repeat(64);
const HUMAN = "c".repeat(64);

type CommandPayloadWindow = Window & {
  __BUZZ_E2E_COMMAND_PAYLOADS__?: Array<{
    command: string;
    payload: unknown;
  }>;
  __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
    channelName: string;
    content: string;
    pubkey?: string;
  }) => { id: string };
};

async function installAgentFixtures(page: Page) {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: AGENT,
        name: "Morgarita",
        status: "running",
        channelNames: ["general"],
      },
    ],
  });
}

async function openGeneral(page: Page) {
  await page.goto(`/#/channels/${CHANNEL_ID}`, {
    waitUntil: "domcontentloaded",
  });
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

async function emitRootMessage(page: Page, content: string, pubkey?: string) {
  const event = await page.evaluate(
    ({ message, author }) =>
      (window as CommandPayloadWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: message,
        pubkey: author,
      }),
    { message: content, author: pubkey },
  );
  if (!event) throw new Error("Mock message emitter is not installed");
  return event;
}

async function openThread(page: Page, threadRootId: string) {
  await page.goto(
    `/#/channels/${CHANNEL_ID}?messageId=${threadRootId}&thread=${threadRootId}`,
    { waitUntil: "domcontentloaded" },
  );
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();
}

function threadInput(page: Page) {
  return page
    .getByTestId("thread-composer-overlay")
    .getByTestId("message-input");
}

function sentChannelMessages(page: Page) {
  return page.evaluate(() =>
    ((window as CommandPayloadWindow).__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])
      .filter((entry) => entry.command === "send_channel_message")
      .map((entry) => entry.payload as { mentionPubkeys?: string[] }),
  );
}

test("replying to an agent's thread auto-inserts a removable mention chip", async ({
  page,
}) => {
  await installAgentFixtures(page);
  await openGeneral(page);
  const root = await emitRootMessage(page, "@Fulki 2", AGENT);

  await openThread(page, root.id);

  const input = threadInput(page);
  await expect(input).toHaveText("@Morgarita ");
  await expect(input.locator(".agent-mention-highlight")).toHaveCount(1);

  await input.click();
  await page.keyboard.press("End");
  await input.pressSequentially("how about 10+6?");
  await expect(input).toHaveText("@Morgarita how about 10+6?");
  await input.press("Enter");

  await expect
    .poll(async () => {
      const sends = await sentChannelMessages(page);
      return sends.at(-1)?.mentionPubkeys ?? null;
    })
    .toContain(AGENT);
});

test("removing the chip opts out and it is not re-inserted", async ({
  page,
}) => {
  await installAgentFixtures(page);
  await openGeneral(page);
  const root = await emitRootMessage(page, "@Fulki 2", AGENT);

  await openThread(page, root.id);

  const input = threadInput(page);
  await expect(input).toHaveText("@Morgarita ");

  // Replacing the content removes the chip — the auto-mention must respect
  // the opt-out and stay away for this reply target.
  await input.fill("how about 10+6?");
  await page.waitForTimeout(250);
  await expect(input).toHaveText("how about 10+6?");
  await input.press("Enter");

  await expect
    .poll(async () => {
      const sends = await sentChannelMessages(page);
      return sends.length;
    })
    .toBeGreaterThan(0);
  const sends = await sentChannelMessages(page);
  expect(sends.at(-1)?.mentionPubkeys ?? []).not.toContain(AGENT);
});

test("human-authored thread roots get no auto-mention", async ({ page }) => {
  await installAgentFixtures(page);
  await openGeneral(page);
  const root = await emitRootMessage(page, "lunch plans?", HUMAN);

  await openThread(page, root.id);

  const input = threadInput(page);
  // Give the insertion scheduler a frame to (incorrectly) fire before
  // asserting the composer stayed empty.
  await page.waitForTimeout(250);
  await expect(input).toHaveText("");
});
