import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";

// The mock bridge signs in as this identity, so a card `p`-tagged to it renders
// in the interactive (owner-locked) state with tappable options.
const MOCK_IDENTITY_PUBKEY = "deadbeef".repeat(8);
const AGENT_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
const CHANNEL_NAME = "engineering";

type MockMessageWindow = Window & {
  __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
    channelName: string;
    content: string;
    pubkey?: string;
    kind?: number;
    extraTags?: string[][];
  }) => { id: string } | undefined;
  __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
    channelName: string;
  }) => boolean;
};

const KIND_ELICITATION_REQUEST = 44300;

// A single /interview-style question card (one card per question).
const CARD_CONTENT = JSON.stringify({
  v: 1,
  questionKey: "question_0",
  header: "Project weight",
  prompt: "How heavy is this — throwaway, real, or flagship?",
  multiSelect: false,
  allowCustom: true,
  options: [
    { label: "QUICK", description: "hack / throwaway" },
    { label: "STANDARD", description: "a real project" },
    { label: "FLAGSHIP", description: "revenue / public / client-facing" },
  ],
});

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(() =>
      page.evaluate(
        (name) =>
          (
            window as MockMessageWindow
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: name }) ??
          false,
        channelName,
      ),
    )
    .toBe(true);
}

test.describe("interactive question card", () => {
  test.use({ viewport: { width: 1280, height: 720 } });

  test("renders an owner-locked question card with tappable options", async ({
    page,
  }) => {
    await installMockBridge(page);
    await page.goto("/");
    await page.getByTestId(`channel-${CHANNEL_NAME}`).click();
    await expect(page.getByTestId("chat-title")).toHaveText(CHANNEL_NAME);
    await waitForMockLiveSubscription(page, CHANNEL_NAME);

    // The agent posts a 44300 card, locked to the current (owner) identity.
    await page.evaluate(
      ({ channelName, content, agent, owner, kind }) => {
        (window as MockMessageWindow).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName,
          content,
          pubkey: agent,
          kind,
          extraTags: [["p", owner]],
        });
      },
      {
        channelName: CHANNEL_NAME,
        content: CARD_CONTENT,
        agent: AGENT_PUBKEY,
        owner: MOCK_IDENTITY_PUBKEY,
        kind: KIND_ELICITATION_REQUEST,
      },
    );

    const card = page.getByTestId("question-card");
    await expect(card).toBeVisible();
    await expect(card).toHaveAttribute("data-state", "open");
    // Each option renders as its own tappable button, plus the custom field.
    await expect(card.getByRole("button", { name: /QUICK/ })).toBeVisible();
    await expect(card.getByRole("button", { name: /FLAGSHIP/ })).toBeVisible();
    await expect(card.getByPlaceholder("Other…")).toBeVisible();

    await waitForAnimations(page);
    await card.screenshot({
      path: "test-results/screenshots/question-card.png",
    });
  });
});
