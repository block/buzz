import { expect, test, type Page } from "@playwright/test";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const AGENT = TEST_IDENTITIES.charlie.pubkey;
const CHANNEL = "94a444a4-c0a3-5966-ab05-530c6ddc2301";
const SESSION = "codex-existing-conversation";
const config = (value: string) => ({
  liveEffortSwitching: true,
  effortSessionToken: "fbf7259f-74df-4859-8b53-c0d8b77fb21e",
  configOptions: [
    {
      id: "reasoning_effort",
      name: "Reasoning effort",
      type: "select",
      category: "thought_level",
      currentValue: value,
      options: [
        { value: "low", name: "Low" },
        { value: "high", name: "High" },
      ],
    },
  ],
});

async function seed(
  page: Page,
  kind: string,
  payload: unknown,
  seq: number,
  sessionId = SESSION,
) {
  await page.evaluate(
    ({ agentPubkey, channelId, sessionId, kind, payload, seq }) => {
      window.__BUZZ_E2E_SEED_LIVE_OBSERVER_EVENTS__?.({
        agentPubkey,
        events: [
          {
            seq,
            timestamp: new Date().toISOString(),
            kind,
            payload,
            agentIndex: 0,
            channelId,
            sessionId,
            turnId: null,
          },
        ],
      });
    },
    { agentPubkey: AGENT, channelId: CHANNEL, sessionId, kind, payload, seq },
  );
}

async function openActivity(page: Page, status: string) {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: AGENT,
        name: "Codex Colleague",
        status: "running",
        channelNames: ["agents"],
      },
    ],
    observerControlResults: [{ type: "switch_effort", status }],
  });
  await page.goto(`/#/channels/${CHANNEL}?agentSession=${AGENT}`);
  await expect(page.getByTestId("agent-session-thread-panel")).toBeVisible();
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_SEED_LIVE_OBSERVER_EVENTS__ === "function",
  );
  await seed(page, "session_config_captured", config("low"), 1);
  await seed(
    page,
    "acp_read",
    {
      method: "session/update",
      params: {
        sessionId: SESSION,
        update: {
          sessionUpdate: "agent_message_chunk",
          content: {
            type: "text",
            text: "I have the conversation context and am checking the remaining cases.",
          },
        },
      },
    },
    2,
  );
  const picker = page.getByTestId("conversation-effort-picker");
  await expect(picker).toBeVisible();
  await expect(page.getByTestId("agent-session-thread-panel")).toContainText(
    "I have the conversation context",
  );
  return picker;
}

test("live effort waits for the exact adapter receipt and keeps the conversation", async ({
  page,
}, testInfo) => {
  const picker = await openActivity(page, "queued");
  await picker.getByLabel("Thinking level").click();
  await page.getByRole("menuitemradio", { name: "High", exact: true }).click();
  await expect(picker.getByRole("status")).toContainText("Queued");
  await expect(picker.getByLabel("Thinking level")).toContainText("Low");
  await expect(picker.getByLabel("Thinking level")).toBeDisabled();
  await waitForAnimations(page);
  await page
    .getByTestId("agent-session-thread-panel")
    .screenshot({ path: testInfo.outputPath("live-queued.png") });
  const request = await page.evaluate(() =>
    window.__BUZZ_E2E_OBSERVER_CONTROLS__?.at(-1),
  );
  expect(request).toMatchObject({
    agentPubkey: AGENT,
    payload: {
      type: "switch_effort",
      channelId: CHANNEL,
      sessionId: SESSION,
      effort: "high",
      requestId: expect.any(String),
    },
  });
  if (!request) throw new Error("Expected a recorded effort request");
  const payload = request.payload as Record<string, unknown>;
  await seed(
    page,
    "control_result",
    { ...payload, status: "applied", requestId: "old-replayed-pick" },
    3,
  );
  await expect(picker.getByRole("status")).toContainText("Queued");
  await seed(page, "session_config_captured", config("high"), 4);
  await seed(page, "control_result", { ...payload, status: "applied" }, 5);
  await expect(picker.getByRole("status")).toHaveText(
    "Thinking level applied to this conversation.",
  );
  await expect(picker.getByLabel("Thinking level")).toContainText("High");
  await expect(picker.getByLabel("Thinking level")).toBeEnabled();
  await waitForAnimations(page);
  await page.getByTestId("agent-session-thread-panel").screenshot({
    path: testInfo.outputPath("live-applied.png"),
  });
});

test("adapter rejection leaves the reported level unchanged", async ({
  page,
}) => {
  const picker = await openActivity(page, "rejected");
  await picker.getByLabel("Thinking level").click();
  await page.getByRole("menuitemradio", { name: "High", exact: true }).click();
  await expect(picker.getByRole("status")).toContainText("rejected");
  await expect(picker.getByLabel("Thinking level")).toContainText("Low");
  await expect(picker.getByLabel("Thinking level")).toBeEnabled();
});

test("multiple reported sessions require a conversation choice", async ({
  page,
}) => {
  const picker = await openActivity(page, "queued");
  await seed(
    page,
    "session_config_captured",
    config("low"),
    2,
    "sibling-conversation",
  );
  await expect(
    picker.getByLabel("Conversation", { exact: true }),
  ).toContainText("Choose a conversation");
  await expect(picker.getByLabel("Thinking level")).toBeDisabled();
  expect(
    await page.evaluate(() => window.__BUZZ_E2E_OBSERVER_CONTROLS__?.length),
  ).toBe(0);
  await picker.getByLabel("Conversation", { exact: true }).click();
  await page.getByRole("menuitemradio").first().click();
  await picker.getByLabel("Thinking level").click();
  await page.getByRole("menuitemradio", { name: "High", exact: true }).click();
  const control = await page.evaluate(() =>
    window.__BUZZ_E2E_OBSERVER_CONTROLS__?.at(-1),
  );
  expect(control?.payload).toMatchObject({
    channelId: CHANNEL,
    sessionId: "sibling-conversation",
    effort: "high",
  });
});

test("a session without the live capability has no effort control", async ({
  page,
}) => {
  await openActivity(page, "queued");
  await seed(
    page,
    "session_config_captured",
    { ...config("low"), liveEffortSwitching: false },
    20,
  );
  await expect(page.getByTestId("conversation-effort-picker")).toHaveCount(0);
  expect(
    await page.evaluate(() => window.__BUZZ_E2E_OBSERVER_CONTROLS__?.length),
  ).toBe(0);
});

test("recreated sessions reject late receipts for the prior conversation", async ({
  page,
}) => {
  const picker = await openActivity(page, "queued");
  await picker.getByLabel("Thinking level").click();
  await page.getByRole("menuitemradio", { name: "High", exact: true }).click();
  await expect(picker.getByRole("status")).toContainText("Queued");
  const control = await page.evaluate(() =>
    window.__BUZZ_E2E_OBSERVER_CONTROLS__?.at(-1),
  );
  expect(control).toBeTruthy();
  await seed(
    page,
    "session_config_captured",
    {
      ...config("low"),
      effortSessionToken: "79674911-359b-49ab-ae85-89d6c31d5e0f",
    },
    20,
  );
  await expect(picker.getByRole("status")).toContainText("session has ended");
  await expect(picker.getByLabel("Thinking level")).toBeEnabled();
  await seed(
    page,
    "control_result",
    { ...control?.payload, status: "applied" },
    21,
  );
  await expect(picker.getByRole("status")).toContainText("session has ended");
  await expect(picker.getByLabel("Thinking level")).toContainText("Low");
});
