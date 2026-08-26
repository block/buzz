import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";

import {
  installMockBridge,
  openChannelBrowser,
  TEST_IDENTITIES,
} from "../helpers/bridge";

const MOCK_VIEWER_PUBKEY = "deadbeef".repeat(8);
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

const IN_CHANNEL_MANAGED_AGENT_PUBKEY =
  "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OUT_OF_CHANNEL_MANAGED_AGENT_PUBKEY =
  "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const OUT_OF_CHANNEL_PROVIDER_AGENT_PUBKEY =
  "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const REUSABLE_PERSONA_AGENT_PUBKEY =
  "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const ALLOWLIST_RELAY_AGENT_PUBKEY =
  "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const DELAYED_RELAY_AGENT_PUBKEY =
  "9999999999999999999999999999999999999999999999999999999999999999";
const CASEY_PROFILE_PUBKEY =
  "1111111111111111111111111111111111111111111111111111111111111111";
const PROFILE_ONLY_AGENT_PUBKEY =
  "8f83d6b7f3d74f7d933ae3a54dd8c6cc85c7f98e531c16e5a827b953441a8d67";
const OWNED_AGENT_PROFILE_PUBKEY =
  "1212121212121212121212121212121212121212121212121212121212121212";
const SYSTEM_MESSAGE_KIND = 40099;
const DM_THREAD_AGENT_MENTION_ERROR_TEXT =
  "Agents must already be in a DM to be mentioned in its threads. Start a new conversation that includes the agent.";
const DM_THREAD_MEMBERS_LOADING_ERROR_TEXT =
  "Checking conversation members. Try again in a moment.";

/** Locator scoped to the mention autocomplete dropdown inside the composer. */
function autocomplete(page: import("@playwright/test").Page) {
  return page
    .getByTestId("message-composer")
    .getByTestId("mention-autocomplete");
}

async function readCommandLog(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    return (
      (window as Window & { __BUZZ_E2E_COMMANDS__?: string[] })
        .__BUZZ_E2E_COMMANDS__ ?? []
    );
  });
}

async function readCommandPayloadLog(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    return (
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: Array<{
            command: string;
            payload: unknown;
          }>;
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? []
    );
  });
}

async function readOutgoingMentionPubkeys(
  page: import("@playwright/test").Page,
  content: string,
) {
  return page.evaluate((expectedContent) => {
    const signedEvent = (
      window as Window & {
        __BUZZ_E2E_SIGNED_EVENTS__?: Array<{
          content?: string;
          tags?: string[][];
        }>;
      }
    ).__BUZZ_E2E_SIGNED_EVENTS__?.find(
      (event) => event.content === expectedContent,
    );
    if (signedEvent) {
      return (signedEvent.tags ?? [])
        .filter((tag) => tag[0] === "p" && tag[1])
        .map((tag) => tag[1]);
    }

    const entries =
      (
        window as Window & {
          __BUZZ_E2E_COMMAND_LOG__?: Array<{
            command: string;
            payload: unknown;
          }>;
        }
      ).__BUZZ_E2E_COMMAND_LOG__ ?? [];

    for (const entry of entries) {
      if (entry.command === "send_channel_message") {
        const payload = entry.payload as
          | { content?: string; mentionPubkeys?: string[] }
          | undefined;
        if (payload?.content === expectedContent) {
          return payload.mentionPubkeys ?? [];
        }
      }

      if (entry.command === "sign_event") {
        const unsignedEvent = entry.payload as
          | { content?: string; tags?: string[][] }
          | undefined;
        if (unsignedEvent?.content !== expectedContent) continue;
        return (unsignedEvent.tags ?? [])
          .filter((tag) => tag[0] === "p" && tag[1])
          .map((tag) => tag[1]);
      }

      if (entry.command !== "plugin:websocket|send") continue;
      const data = (
        entry.payload as { message?: { data?: string } } | undefined
      )?.message?.data;
      if (!data) continue;

      try {
        const frame = JSON.parse(data) as [
          string,
          { content?: string; tags?: string[][] },
        ];
        if (frame[0] !== "EVENT" || frame[1]?.content !== expectedContent) {
          continue;
        }
        return (frame[1].tags ?? [])
          .filter((tag) => tag[0] === "p" && tag[1])
          .map((tag) => tag[1]);
      } catch {}
    }

    return null;
  }, content);
}

function commandCount(commands: string[], command: string) {
  return commands.filter((entry) => entry === command).length;
}

async function emitMockMessage(
  page: import("@playwright/test").Page,
  channelName: string,
  content: string,
  options?: {
    kind?: number;
    mentionPubkeys?: string[];
    parentEventId?: string;
    pubkey?: string;
  },
) {
  const event = await page.evaluate(
    ({ ch, kind, mentionPubkeys, msg, parentEventId, pubkey }) => {
      return (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            kind?: number;
            mentionPubkeys?: string[];
            parentEventId?: string | null;
            pubkey?: string;
          }) => { id: string; created_at: number; pubkey: string };
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: ch,
        content: msg,
        kind,
        mentionPubkeys,
        parentEventId: parentEventId ?? undefined,
        pubkey: pubkey ?? undefined,
      });
    },
    {
      ch: channelName,
      kind: options?.kind,
      mentionPubkeys: options?.mentionPubkeys,
      msg: content,
      parentEventId: options?.parentEventId ?? null,
      pubkey: options?.pubkey ?? TEST_IDENTITIES.alice.pubkey,
    },
  );
  if (!event) {
    throw new Error("Mock message emitter is not installed");
  }
  return event;
}

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
  kind?: number,
) {
  await expect
    .poll(async () => {
      return page.evaluate(
        ({ currentChannelName, kind: expectedKind }) => {
          return (
            (
              window as Window & {
                __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                  channelName: string;
                  kind?: number;
                }) => boolean;
              }
            ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: currentChannelName,
              kind: expectedKind,
            }) ?? false
          );
        },
        { currentChannelName: channelName, kind },
      );
    })
    .toBe(true);
}

// The channel timeline renders off a `useDeferredValue` snapshot that lags the
// latest `messages` by a commit; the list wrapper carries
// `data-render-pending="true"` while that commit is in flight and drops the
// attribute once it settles. Poll for its absence before asserting on
// freshly-sent content so the assertion does not race the deferred commit.
async function waitForTimelineSettled(page: import("@playwright/test").Page) {
  await expect(page.locator("[data-render-pending]")).toHaveCount(0);
}

async function expectOwnedAgentProfileActions(
  profilePopover: import("@playwright/test").Locator,
  pubkey: string,
) {
  await expect(
    profilePopover.getByTestId(`user-profile-popover-message-${pubkey}`),
  ).toBeVisible();
  await expect(
    profilePopover.getByTestId(`user-profile-popover-wave-${pubkey}`),
  ).toHaveCount(0);
  await expect(
    profilePopover.getByTestId(`user-profile-popover-huddle-${pubkey}`),
  ).toBeVisible();
}

async function expectAgentProfileActionsHidden(
  profilePopover: import("@playwright/test").Locator,
  pubkey: string,
) {
  await expect(
    profilePopover.getByTestId(`user-profile-popover-message-${pubkey}`),
  ).toHaveCount(0);
  await expect(
    profilePopover.getByTestId(`user-profile-popover-wave-${pubkey}`),
  ).toHaveCount(0);
  await expect(
    profilePopover.getByTestId(`user-profile-popover-huddle-${pubkey}`),
  ).toHaveCount(0);
}

test("@ trigger prioritizes channel members before runnable personas and other managed agents", async ({
  page,
}) => {
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        name: "charlie",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@");

  const dropdown = autocomplete(page);
  await expect(dropdown).toBeVisible();
  await expect(dropdown.getByText("alice")).toBeVisible();
  await expect(dropdown.getByText("bob")).toBeVisible();
  await expect(dropdown.getByText("Fizz")).toBeVisible();
  await expect(dropdown.getByText("charlie")).toBeVisible();
  await expect(dropdown.getByText("outsider")).toHaveCount(0);
  const charlieRow = dropdown.locator("button", { hasText: "charlie" });
  await expect(charlieRow.getByTestId("mention-agent-icon")).toBeVisible();
  await expect(charlieRow.getByText("not in channel")).toBeVisible();
  await expect(
    dropdown
      .locator("button", { hasText: "alice" })
      .getByText("not in channel"),
  ).not.toBeVisible();

  const suggestions = dropdown.locator("button");
  const suggestionText = await suggestions.allInnerTexts();
  const aliceIndex = suggestionText.findIndex((text) => text.includes("alice"));
  const fizzIndex = suggestionText.findIndex((text) => text.includes("Fizz"));
  const bobIndex = suggestionText.findIndex((text) => text.includes("bob"));
  const charlieIndex = suggestionText.findIndex((text) =>
    text.includes("charlie"),
  );
  const outsiderIndex = suggestionText.findIndex((text) =>
    text.includes("outsider"),
  );
  expect(aliceIndex).toBeGreaterThanOrEqual(0);
  expect(fizzIndex).toBeGreaterThanOrEqual(0);
  expect(bobIndex).toBeGreaterThanOrEqual(0);
  expect(charlieIndex).toBeGreaterThanOrEqual(0);
  expect(outsiderIndex).toEqual(-1);
  expect(aliceIndex).toBeLessThan(fizzIndex);
  expect(bobIndex).toBeLessThan(fizzIndex);
  expect(fizzIndex).toBeLessThan(charlieIndex);
});

test("duplicate owned agents preserve provenance and exact pubkey selection", async ({
  page,
}) => {
  const managedPubkey = IN_CHANNEL_MANAGED_AGENT_PUBKEY;
  const relayPubkey = ALLOWLIST_RELAY_AGENT_PUBKEY;
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: managedPubkey,
        name: "carl",
        status: "running",
        channelNames: ["general"],
        backend: {
          type: "provider",
          id: "mock",
          config: {},
        },
      },
    ],
    relayAgents: [
      {
        pubkey: relayPubkey,
        ownerPubkey: MOCK_VIEWER_PUBKEY,
        name: "carl",
        respondTo: "owner-only",
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.evaluate(
    async ({ channelId, pubkey }) => {
      const invoke = window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      if (!invoke) throw new Error("Mock bridge is not installed.");
      await invoke("add_channel_members", {
        channelId,
        pubkeys: [pubkey],
        role: "bot",
      });
      await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
        queryKey: ["channels"],
      });
    },
    { channelId: GENERAL_CHANNEL_ID, pubkey: relayPubkey },
  );

  const input = page.getByTestId("message-input");
  await input.fill("@carl");
  const dropdown = autocomplete(page);
  const managedRow = dropdown.getByTestId(
    `mention-suggestion-${managedPubkey}`,
  );
  const relayRow = dropdown.getByTestId(`mention-suggestion-${relayPubkey}`);
  await expect(managedRow).toContainText("agent");
  await expect(managedRow.getByTestId("mention-agent-provenance")).toHaveCount(
    0,
  );
  await expect(relayRow).toContainText("agent");
  const relayProvenanceMarker = relayRow.getByTestId(
    "mention-agent-provenance",
  );
  await expect(relayProvenanceMarker).toHaveAttribute(
    "aria-label",
    "From another Buzz setup",
  );
  await expect(relayProvenanceMarker).toHaveAttribute(
    "title",
    "From another Buzz setup",
  );
  await expect(relayProvenanceMarker).toBeVisible();
  await expect(relayProvenanceMarker).toHaveText("");
  await expect(relayProvenanceMarker.locator("svg")).toBeVisible();
  await expect(managedRow).not.toContainText("managed by you");
  await expect(relayRow).not.toContainText("managed by you");

  await page.setViewportSize({ width: 760, height: 640 });
  await expect(relayProvenanceMarker).toBeVisible();
  const rowBox = await relayRow.boundingBox();
  const dropdownBox = await dropdown.boundingBox();
  expect(rowBox).not.toBeNull();
  expect(dropdownBox).not.toBeNull();
  expect((rowBox?.x ?? 0) + (rowBox?.width ?? 0)).toBeLessThanOrEqual(
    (dropdownBox?.x ?? 0) + (dropdownBox?.width ?? 0) + 1,
  );

  const collisionKeys = dropdown.getByTestId("mention-collision-npub");
  await expect(collisionKeys).toHaveCount(2);
  const fullNpubs = await collisionKeys.evaluateAll((nodes) =>
    nodes.map((node) => node.getAttribute("title")),
  );
  expect(fullNpubs).toHaveLength(2);
  expect(new Set(fullNpubs).size).toBe(2);

  await managedRow
    .getByRole("button", { name: "Automatically mention carl", exact: true })
    .click();
  await expect(
    page.getByTestId(`composer-address-lock-${managedPubkey}`),
  ).toBeVisible();
  await expect(
    page.getByTestId(`composer-address-lock-${relayPubkey}`),
  ).toHaveCount(0);
  await input.fill("local");
  await page.getByTestId("send-message").click();
  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "local"))
    .toEqual([managedPubkey]);
  await expect(input).toHaveText("@carl ");

  await page.getByTestId(`composer-address-lock-${managedPubkey}`).click();
  await input.fill("@carl");
  const reopenedDropdown = autocomplete(page);
  await expect(reopenedDropdown).toBeVisible();
  const reopenedRelayRow = reopenedDropdown.getByTestId(
    `mention-suggestion-${relayPubkey}`,
  );
  await reopenedRelayRow
    .getByRole("button", { name: "Automatically mention carl", exact: true })
    .click();
  await expect(
    page.getByTestId(`composer-address-lock-${relayPubkey}`),
  ).toBeVisible();
  await expect(
    page.getByTestId(`composer-address-lock-${managedPubkey}`),
  ).toHaveCount(0);
  await input.fill("remote");
  await page.getByTestId("send-message").click();
  const sendWithoutInviting = page.getByRole("button", { name: "Do nothing" });
  try {
    await sendWithoutInviting.waitFor({ state: "visible", timeout: 2_000 });
    await sendWithoutInviting.click();
  } catch {
    // In-channel selections send immediately without opening the prompt.
  }
  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "remote"))
    .toEqual([relayPubkey]);

  await page.getByTestId("channel-members-trigger").click();
  const remoteSidebarMarker = page.getByTestId(
    `sidebar-member-agent-provenance-${relayPubkey}`,
  );
  await expect(
    page.getByTestId(`sidebar-member-agent-provenance-${managedPubkey}`),
  ).toHaveCount(0);
  await expect(remoteSidebarMarker).toHaveAttribute(
    "aria-label",
    "From another Buzz setup",
  );
  await expect(remoteSidebarMarker).toHaveText("");
  await expect(remoteSidebarMarker.locator("svg")).toBeVisible();
  const remoteSidebarRow = page.getByTestId(`sidebar-member-${relayPubkey}`);
  const localSidebarMarker = page.getByTestId(
    `sidebar-member-agent-provenance-${managedPubkey}`,
  );
  const markerRemainsVisible = () =>
    remoteSidebarMarker.evaluate((marker) => {
      let element: Element | null = marker;
      while (element) {
        const style = window.getComputedStyle(element);
        if (style.display === "none" || style.visibility === "hidden") {
          return false;
        }
        if (Number(style.opacity) === 0) return false;
        element = element.parentElement;
      }
      return true;
    });
  await expect.poll(markerRemainsVisible).toBe(true);
  await expect(localSidebarMarker).toHaveCount(0);
  const sidebarMarkerBox = await remoteSidebarMarker.boundingBox();
  const sidebarBox = await page.getByTestId("members-sidebar").boundingBox();
  expect(sidebarMarkerBox).not.toBeNull();
  expect(sidebarBox).not.toBeNull();
  expect(
    (sidebarMarkerBox?.x ?? 0) + (sidebarMarkerBox?.width ?? 0),
  ).toBeLessThanOrEqual((sidebarBox?.x ?? 0) + (sidebarBox?.width ?? 0) + 1);

  await remoteSidebarRow.hover();
  await page.waitForTimeout(200);
  await expect.poll(markerRemainsVisible).toBe(true);
  await expect(localSidebarMarker).toHaveCount(0);

  await page.getByTestId(`sidebar-member-open-profile-${relayPubkey}`).focus();
  await page.waitForTimeout(200);
  await expect.poll(markerRemainsVisible).toBe(true);
  await expect(localSidebarMarker).toHaveCount(0);
});

test("relay-only shared agents emit an outbound mention tag when selected", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Ask @alice");

  const aliceRow = autocomplete(page).locator("button", { hasText: "alice" });
  await expect(aliceRow).toBeVisible();
  await aliceRow.click();
  await page.keyboard.type("please reply");

  const content = "Ask @alice please reply";
  await expect(input).toHaveText(content);
  await page.getByTestId("send-message").click();

  await expect
    .poll(() => readOutgoingMentionPubkeys(page, content))
    .toContain(TEST_IDENTITIES.alice.pubkey);
});

test("thread autocomplete keeps multiple long names readable in a narrow panel", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey:
          "9999999999999999999999999999999999999999999999999999999999999999",
        name: "Brain With A Very Long Name",
        status: "stopped",
      },
      {
        pubkey:
          "9999999999999999999999999999999999999999999999999999999999999998",
        name: "Brainstorming Assistant With A Long Name",
        status: "stopped",
      },
      {
        pubkey:
          "9999999999999999999999999999999999999999999999999999999999999997",
        name: "Brainy Helper With Another Long Name",
        status: "stopped",
      },
    ],
  });
  await page.setViewportSize({ width: 900, height: 640 });
  await page.addInitScript(() => {
    window.sessionStorage.setItem("buzz.desktop.thread-panel-width", "300");
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.setViewportSize({ width: 760, height: 640 });

  await emitMockMessage(page, "general", "Reply to open the thread", {
    parentEventId: "mock-general-welcome",
  });
  const threadSummary = page.getByTestId("message-thread-summary").first();
  await expect(threadSummary).toBeVisible();
  await threadSummary.click();

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();
  const panelBox = await threadPanel.boundingBox();
  expect(panelBox?.width ?? Number.POSITIVE_INFINITY).toBeLessThanOrEqual(320);

  const input = threadPanel.getByTestId("message-input");
  await input.fill("@Brain");

  const dropdown = threadPanel.getByTestId("mention-autocomplete");
  await expect(dropdown).toBeVisible();

  for (const name of [
    "Brain With A Very Long Name",
    "Brainstorming Assistant With A Long Name",
    "Brainy Helper With Another Long Name",
  ]) {
    const row = dropdown.locator("button", { hasText: name });
    await expect(row).toBeVisible();
    await expect(
      row.getByTestId("mention-suggestion-avatar-fallback"),
    ).toBeVisible();
    await expect(row.getByText("agent")).toBeVisible();
    await expect(row.getByText("managed by you")).toBeVisible();

    await expect(row.getByText(name)).not.toHaveCSS(
      "text-overflow",
      "ellipsis",
    );
  }
});

test("blocks non-participant persona mentions in DM threads", async ({
  page,
}) => {
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
  });
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");
  await waitForMockLiveSubscription(page, "bob-tyler");

  const threadRoot = await emitMockMessage(
    page,
    "bob-tyler",
    "Thread before adding an agent",
  );
  await emitMockMessage(page, "bob-tyler", "Existing thread reply", {
    parentEventId: threadRoot.id,
  });
  const threadSummary = page.getByTestId("message-thread-summary").first();
  await expect(threadSummary).toBeVisible();
  await threadSummary.click();

  const threadPanel = page.getByTestId("message-thread-panel");
  const input = threadPanel.getByTestId("message-input");
  await input.fill("Ask @fi");
  await expect(
    threadPanel
      .getByTestId("mention-autocomplete")
      .locator("button", { hasText: "Fizz" }),
  ).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" in this thread");
  const baselineCommands = await readCommandLog(page);

  await threadPanel.getByTestId("send-message").click();

  await expect(
    page.getByText(
      "Agents must already be in a DM to be mentioned in its threads. Start a new conversation that includes the agent.",
    ),
  ).toBeVisible();
  const commands = await readCommandLog(page);
  expect(commandCount(commands, "create_managed_agent")).toBe(
    commandCount(baselineCommands, "create_managed_agent"),
  );
  expect(commandCount(commands, "add_channel_members")).toBe(
    commandCount(baselineCommands, "add_channel_members"),
  );
  await expect(input).toContainText("Fizz");
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");
});

test("defers agent mentions until DM members finish loading", async ({
  page,
}) => {
  await installMockBridge(page, {
    channelMembersReadDelayMs: 5_000,
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        name: "alice",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");
  await waitForMockLiveSubscription(page, "alice-tyler");

  const threadRoot = await emitMockMessage(
    page,
    "alice-tyler",
    "Thread while members load",
  );
  await emitMockMessage(page, "alice-tyler", "Existing thread reply", {
    parentEventId: threadRoot.id,
  });
  await page.getByTestId("message-thread-summary").first().click();

  const threadPanel = page.getByTestId("message-thread-panel");
  const input = threadPanel.getByTestId("message-input");
  await input.fill("Ask @ali");
  await expect(
    threadPanel
      .getByTestId("mention-autocomplete")
      .locator("button", { hasText: "alice" }),
  ).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" before members resolve");
  const baselineCommands = await readCommandLog(page);
  await threadPanel.getByTestId("send-message").click();

  await expect(
    page.getByText(DM_THREAD_MEMBERS_LOADING_ERROR_TEXT).first(),
  ).toBeVisible();
  await page.mouse.move(0, 0);
  expect(commandCount(await readCommandLog(page), "add_channel_members")).toBe(
    commandCount(baselineCommands, "add_channel_members"),
  );
  await expect(input).toContainText("before members resolve");

  await page.waitForTimeout(5_100);
  await threadPanel.getByTestId("send-message").click();

  await expect(page.getByText(DM_THREAD_AGENT_MENTION_ERROR_TEXT)).toHaveCount(
    0,
  );
  expect(commandCount(await readCommandLog(page), "add_channel_members")).toBe(
    commandCount(baselineCommands, "add_channel_members"),
  );
  await expect(input).toHaveText("@alice ");
  await expect(threadPanel).toContainText("before members resolve");
});

test("autocomplete filters managed-agent suggestions as user types", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        name: "alice",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@ali");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("alice")).toBeVisible();
  await expect(dropdown.getByText("bob")).not.toBeVisible();
});

test("autocomplete searches global non-member people from the first typed character", async ({
  page,
}) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: CASEY_PROFILE_PUBKEY,
        displayName: "tessa",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@t");

  const dropdown = autocomplete(page);
  const tessaRow = dropdown.locator("button", { hasText: "tessa" });
  await expect(tessaRow).toBeVisible();
  await expect(tessaRow.getByText("not in channel")).toBeVisible();
});

test("mention autocomplete caps global people search at 50 results", async ({
  page,
}) => {
  const searchProfiles = Array.from({ length: 55 }, (_, index) => ({
    pubkey: `${(index + 1).toString(16).padStart(64, "0")}`,
    displayName: `Alex ${String(index + 1).padStart(2, "0")}`,
  }));
  await installMockBridge(page, { searchProfiles });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill("@Alex");

  const dropdown = autocomplete(page);
  await expect(dropdown.locator("button")).toHaveCount(50);
  await expect(dropdown.getByText("Alex 50")).toBeVisible();
  await expect(dropdown.getByText("Alex 55")).toHaveCount(0);
  await expect(dropdown.getByText("not in channel").last()).toBeVisible();

  const searchCalls = (await readCommandPayloadLog(page)).filter(
    (entry) => entry.command === "search_users",
  );
  expect(searchCalls).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        payload: expect.objectContaining({ cursor: null, limit: 50 }),
      }),
    ]),
  );
  expect(searchCalls).not.toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        payload: expect.objectContaining({ cursor: "2", limit: 50 }),
      }),
    ]),
  );
});

test("selecting a person mention inserts @Name into input", async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("buzz-theme", "buzz-dark");
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  const dropdown = autocomplete(page);
  for (let attempt = 0; attempt < 5; attempt++) {
    await input.fill("Hey @bo");
    await dropdown.getByText("bob").click();
    await page.keyboard.type("hello");
    await expect(input).toHaveText("Hey @bob hello");
  }
  const mentionChip = input.locator(".human-mention-highlight", {
    hasText: "bob",
  });
  await expect(mentionChip).toBeVisible();
  await expect(mentionChip).toHaveText("bob");
  await expect(mentionChip).not.toHaveClass(/agent-mention-highlight/);
  await expect(mentionChip).toHaveCSS("display", "inline");
  await expect(
    input.locator(".mention-prefix-hidden", { hasText: "@" }),
  ).toHaveCount(1);
  const iconMask = await mentionChip.evaluate((element) =>
    getComputedStyle(element, "::before").getPropertyValue(
      "-webkit-mask-image",
    ),
  );
  expect(iconMask).toContain("data:image/svg+xml");
  await expect(mentionChip).toHaveCSS("line-height", "18px");
  const scrollViewport = page.getByTestId("message-input-scroll");
  const paintedBounds = await mentionChip.evaluate((element) => {
    const chip = element.getBoundingClientRect();
    const viewport = element
      .closest("[data-testid='message-input-scroll']")
      ?.getBoundingClientRect();
    if (!viewport)
      throw new Error("Mention chip is missing its scroll viewport");
    return {
      chipTop: chip.top,
      chipBottom: chip.bottom,
      viewportTop: viewport.top,
      viewportBottom: viewport.bottom,
    };
  });
  expect(paintedBounds.chipTop).toBeGreaterThanOrEqual(
    paintedBounds.viewportTop,
  );
  expect(paintedBounds.chipBottom).toBeLessThanOrEqual(
    paintedBounds.viewportBottom,
  );
  const humanIconTranslateY = await mentionChip.evaluate(
    (element) =>
      new DOMMatrix(getComputedStyle(element, "::before").transform).m42,
  );
  const channelIconTranslateY = await input.evaluate((composer) => {
    const probe = document.createElement("span");
    probe.className =
      "mention-chip inline-chip-with-icon inline-chip-icon-channel";
    composer.append(probe);
    const translateY = new DOMMatrix(
      getComputedStyle(probe, "::before").transform,
    ).m42;
    probe.remove();
    return translateY;
  });
  expect(humanIconTranslateY - channelIconTranslateY).toBeCloseTo(1);

  await input.fill("@bo");
  await dropdown.getByText("bob").click();
  await input.press("Shift+Enter");
  await page.keyboard.type("@bo");
  await dropdown.getByText("bob").click();
  await input.press("Shift+Enter");
  await page.keyboard.type("@bo");
  await dropdown.getByText("bob").click();
  const multilineChips = input.locator(".human-mention-highlight");
  await expect(multilineChips).toHaveCount(3);
  const multilinePaintBounds = await multilineChips.evaluateAll((elements) => {
    const viewport = elements[0]
      ?.closest("[data-testid='message-input-scroll']")
      ?.getBoundingClientRect();
    if (!viewport) {
      throw new Error("Mention chips are missing their scroll viewport");
    }
    return elements.map((element) => {
      const chip = element.getBoundingClientRect();
      return {
        chipTop: chip.top,
        chipBottom: chip.bottom,
        viewportTop: viewport.top,
        viewportBottom: viewport.bottom,
      };
    });
  });
  for (const bounds of multilinePaintBounds) {
    expect(bounds.chipTop).toBeGreaterThanOrEqual(bounds.viewportTop);
    expect(bounds.chipBottom).toBeLessThanOrEqual(bounds.viewportBottom);
  }

  await scrollViewport.evaluate((element) => {
    element.style.width = "8rem";
  });
  await input.fill("A deliberately long prefix that forces @bo");
  await dropdown.getByText("bob").click();
  const wrappedChip = input.locator(".human-mention-highlight", {
    hasText: "bob",
  });
  const wrappedPaintBounds = await wrappedChip.evaluate((element) => {
    const chip = element.getBoundingClientRect();
    const viewport = element
      .closest("[data-testid='message-input-scroll']")
      ?.getBoundingClientRect();
    if (!viewport)
      throw new Error("Mention chip is missing its scroll viewport");
    return {
      chipTop: chip.top,
      chipBottom: chip.bottom,
      viewportTop: viewport.top,
      viewportBottom: viewport.bottom,
    };
  });
  expect(wrappedPaintBounds.chipTop).toBeGreaterThanOrEqual(
    wrappedPaintBounds.viewportTop,
  );
  expect(wrappedPaintBounds.chipBottom).toBeLessThanOrEqual(
    wrappedPaintBounds.viewportBottom,
  );
  await expect(input).toHaveCSS("height", /^(?!20px$)/);
  await scrollViewport.evaluate((element) => {
    element.style.removeProperty("width");
  });

  await waitForAnimations(page);
  await page.getByTestId("message-composer").screenshot({
    path: "test-results/inline-chip-polish/composer-after.png",
  });
});

test("immediate ArrowLeft after a person mention is not bounced past the trailing space", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Hey @bo");
  await autocomplete(page).getByText("bob").click();
  await page.keyboard.press("ArrowLeft");
  await page.keyboard.type("x");
  const text = (await input.innerText()).replace(/\s+$/, "");
  expect(text).toBe("Hey @bobx");
  expect(text).not.toMatch(/@bob x/);
});

test("clicking a person mention chip edge is not treated as after the trailing space", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Hey @bo");
  await autocomplete(page).getByText("bob").click();
  const chip = input.locator(".human-mention-highlight", { hasText: "bob" });
  await expect(chip).toBeVisible();
  const box = await chip.boundingBox();
  expect(box).toBeTruthy();
  await chip.click({
    position: {
      x: Math.max((box?.width ?? 1) - 2, 0),
      y: (box?.height ?? 2) / 2,
    },
  });
  await page.keyboard.type("x");
  const text = (await input.innerText()).replace(/\s+$/, "");
  expect(text).toBe("Hey @bobx");
  expect(text).not.toMatch(/@bob x/);
});

test("typing a mention before existing text does not interleave spaces", async ({
  page,
}) => {
  // Regression (the reported repro): with a draft already written, place the
  // caret earlier in the message, type a partial mention, pick a suggestion,
  // then keep typing. Caret correction used to fire on every document change
  // and walk the caret across the mention's trailing space, so each keystroke
  // pushed a space further into the rest of the draft.
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("hello world");

  // Click between "hello" and " world", then open autocomplete there.
  await input.focus();
  for (let i = 0; i < " world".length; i++) {
    await page.keyboard.press("ArrowLeft");
  }
  await page.keyboard.type(" @bo");
  await autocomplete(page).getByText("bob").click();
  await page.keyboard.type("abc");

  await expect(input).toHaveText("hello @bob abc world");
});

test("typing an unregistered @token before existing text is left alone", async ({
  page,
}) => {
  // The trailing-space scan is purely textual, so it also fired for tokens
  // that were never registered as mentions.
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("hello world");
  await input.focus();
  for (let i = 0; i < " world".length; i++) {
    await page.keyboard.press("ArrowLeft");
  }
  await page.keyboard.type(" @zzq");

  await expect(input).toHaveText("hello @zzq world");
});

test("channel references keep caret movement through the channel name", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("#general");

  const channelChip = input.locator(".inline-chip-icon-channel", {
    hasText: "general",
  });
  await expect(channelChip).toBeVisible();
  await expect(channelChip).toHaveText("general");
  await expect(
    input.locator(".mention-prefix-hidden", { hasText: "#" }),
  ).toHaveCount(1);
  const iconMask = await channelChip.evaluate((element) =>
    getComputedStyle(element, "::before").getPropertyValue(
      "-webkit-mask-image",
    ),
  );
  expect(iconMask).toContain("data:image/svg+xml");

  await input.focus();
  await input.press("ArrowLeft");
  await input.press("ArrowLeft");
  await input.press("ArrowLeft");
  await page.keyboard.type("X");

  await expect(input).toHaveText("#geneXral");
});

test("selecting a managed agent mention inserts @Name into input", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        name: "alice",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  const dropdown = autocomplete(page);
  for (let attempt = 0; attempt < 5; attempt++) {
    await input.fill("Hey @ali");
    await dropdown.getByText("alice").click();
    await page.keyboard.type("hello");
    await expect(input).toHaveText("Hey @alice hello");
  }
  const agentMentionChip = input.locator(".agent-mention-highlight", {
    hasText: "alice",
  });
  await expect(agentMentionChip).toBeVisible();
  await expect(agentMentionChip).toHaveText("alice");
  await expect(agentMentionChip).toHaveCSS("display", "inline");
  await expect(agentMentionChip).toHaveCSS("border-top-width", "0px");
});

test("selecting a persona mention creates a channel agent before sending", async ({
  page,
}) => {
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Ask @fi");

  const dropdown = autocomplete(page);
  const fizzRow = dropdown.locator("button", { hasText: "Fizz" });
  await expect(fizzRow).toBeVisible();
  await expect(fizzRow.getByTestId("mention-agent-icon")).toBeVisible();
  await expect(fizzRow.getByText("agent")).toBeVisible();
  await expect(fizzRow.getByText("not in channel")).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" for a hand");

  const composerChip = input.locator(".agent-mention-highlight", {
    hasText: "Fizz",
  });
  await expect(composerChip).toBeVisible();
  await expect(composerChip).toHaveText("Fizz");

  const baselineCommands = await readCommandLog(page);
  const baselineCreateCount = commandCount(
    baselineCommands,
    "create_managed_agent",
  );
  const baselineAddCount = commandCount(
    baselineCommands,
    "add_channel_members",
  );
  const baselineStartCount = commandCount(
    baselineCommands,
    "start_managed_agent",
  );

  await page.getByTestId("send-message").click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);

  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "create_managed_agent"),
    )
    .toBeGreaterThan(baselineCreateCount);
  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "add_channel_members"),
    )
    .toBeGreaterThan(baselineAddCount);
  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "start_managed_agent"),
    )
    .toBeGreaterThan(baselineStartCount);
  await expect
    .poll(async () => commandCount(await readCommandLog(page), "sign_event"))
    .toBeGreaterThan(commandCount(baselineCommands, "sign_event"));

  const commandsAfterSend = (await readCommandLog(page)).slice(
    baselineCommands.length,
  );
  const startIndex = commandsAfterSend.indexOf("start_managed_agent");
  const sendIndex = commandsAfterSend.indexOf("sign_event");
  expect(startIndex).toBeGreaterThanOrEqual(0);
  expect(sendIndex).toBeGreaterThanOrEqual(0);
  expect(startIndex).toBeLessThan(sendIndex);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention].agent-mention-highlight", { hasText: "Fizz" });
  await expect(mentionChip).toBeVisible();
  await expect(mentionChip).toHaveText("Fizz");
});

test("selecting a persona mention reuses an existing persona agent", async ({
  page,
}) => {
  await installMockBridge(page, {
    activePersonaIds: ["builtin:fizz"],
    managedAgents: [
      {
        pubkey: REUSABLE_PERSONA_AGENT_PUBKEY,
        name: "Fizz",
        personaId: "builtin:fizz",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Ask @fi");

  const dropdown = autocomplete(page);
  const fizzRow = dropdown.locator("button", { hasText: "Fizz" });
  await expect(fizzRow).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" for a hand");

  const baselineCommands = await readCommandLog(page);
  const baselineCreateCount = commandCount(
    baselineCommands,
    "create_managed_agent",
  );
  const baselineAddCount = commandCount(
    baselineCommands,
    "add_channel_members",
  );
  const baselineStartCount = commandCount(
    baselineCommands,
    "start_managed_agent",
  );

  await page.getByTestId("send-message").click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);

  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "add_channel_members"),
    )
    .toBeGreaterThan(baselineAddCount);
  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "start_managed_agent"),
    )
    .toBeGreaterThan(baselineStartCount);
  expect(
    commandCount(await readCommandLog(page), "create_managed_agent"),
  ).toEqual(baselineCreateCount);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention].agent-mention-highlight", { hasText: "Fizz" });
  await expect(mentionChip).toBeVisible();
  await expect(mentionChip).toHaveText("Fizz");
});

test("managed relay-profile agents with member roles can be addressed explicitly", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        name: "charlie",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");

  await openChannelBrowser(page);
  await expect(page.getByTestId("channel-browser-dialog")).toBeVisible();
  await page
    .getByTestId("browse-channel-sales")
    .getByRole("button", { name: "Join" })
    .click();
  await expect(page.getByTestId("chat-title")).toHaveText("sales");

  const input = page.getByTestId("message-input");
  await input.fill("@char");

  const dropdown = autocomplete(page);
  const charlieRow = dropdown.getByTestId(
    `mention-suggestion-${TEST_IDENTITIES.charlie.pubkey}`,
  );
  await expect(charlieRow.getByText("charlie")).toBeVisible();
  await expect(charlieRow.getByText("agent")).toBeVisible();
  await charlieRow
    .getByRole("button", {
      name: "Automatically mention charlie",
      exact: true,
    })
    .click();

  await expect(input).toHaveText("@charlie ");
  await expect(input.locator(".agent-mention-highlight")).toHaveText("charlie");
  await expect(
    page.getByTestId(`composer-address-lock-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toBeVisible();
  await expect(
    page.getByRole("status").filter({
      hasText: "Automatically mentioning charlie",
    }),
  ).toBeVisible();
});

test("other-owned agents without a shared channel are hidden from mentions", async ({
  page,
}) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: PROFILE_ONLY_AGENT_PUBKEY,
        displayName: "mira",
        ownerPubkey: TEST_IDENTITIES.outsider.pubkey,
        isAgent: true,
      },
    ],
    userSearchDelayMs: 1_000,
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@mira");

  const dropdown = autocomplete(page);
  await expect(dropdown).not.toBeVisible();
  await expect(input.locator(".mention-chip")).toHaveCount(0);
});

test("stale channel-member agents absent from managed and relay directories stay hidden", async ({
  page,
}) => {
  await installMockBridge(page, { userSearchDelayMs: 1_000 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@mira");

  await expect(autocomplete(page)).toHaveCount(0);
});

test("managed relay agents are visible in channel mentions regardless of relay policy", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        status: "stopped",
      },
    ],
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: ["deadbeef".repeat(8)],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@quinn");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("quinn")).toBeVisible();
  await expect(dropdown.getByText("agent")).toBeVisible();
});

test("relay-only shared agents stay hidden from DM mentions", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");

  await page.getByTestId("message-input").fill("@alice");

  await expect(autocomplete(page)).toHaveCount(0);
});

test("an existing remote agent in a DM is mentioned without creating a local runtime", async ({
  page,
}) => {
  const remoteAgentPubkey = "7".repeat(64);
  await installMockBridge(page, {
    acpRuntimesCatalog: [],
    personas: [
      {
        id: "pm-persona",
        displayName: "PM Bot",
        systemPrompt: "Own product management.",
        isActive: true,
      },
    ],
    relayAgents: [
      {
        pubkey: remoteAgentPubkey,
        ownerPubkey: MOCK_VIEWER_PUBKEY,
        name: "PM Bot",
        channelNames: ["alice-tyler"],
        respondTo: "owner-only",
      },
    ],
    searchProfiles: [
      {
        pubkey: remoteAgentPubkey,
        displayName: "PM Bot",
        ownerPubkey: MOCK_VIEWER_PUBKEY,
        isAgent: true,
      },
    ],
  });
  await page.goto("/");
  await expect(page.getByTestId("channel-alice-tyler")).toBeVisible();
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");

  const input = page.getByTestId("message-input");
  await input.fill("@PM Bot");
  await autocomplete(page)
    .getByTestId(`mention-suggestion-${remoteAgentPubkey}`)
    .click();
  await input.press("End");
  await input.type("keep going");
  await input.press("Enter");

  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "@PM Bot keep going"))
    .toContain(remoteAgentPubkey);
  expect(await readCommandLog(page)).not.toContain("create_managed_agent");
  await expect(
    page.getByText(/No agent runtime available/, { exact: false }),
  ).toHaveCount(0);
});

test("cached relay-agent suggestions are removed when channel authorization disappears", async ({
  page,
}) => {
  await installMockBridge(page, { userSearchDelayMs: 10_000 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@alice");
  const aliceSuggestion = autocomplete(page).getByTestId(
    `mention-suggestion-${TEST_IDENTITIES.alice.pubkey}`,
  );
  await expect(aliceSuggestion).toBeVisible();
  await expect
    .poll(async () =>
      (await readCommandPayloadLog(page)).some(
        (entry) =>
          entry.command === "search_users" &&
          (entry.payload as { query?: string }).query === "alice",
      ),
    )
    .toBe(true);

  await page.evaluate(async (channelId) => {
    const bridge = window as Window & {
      __BUZZ_E2E_INVALIDATE_CHANNELS__?: () => Promise<void>;
      __BUZZ_E2E_MUTATE_CHANNEL__?: (opts: {
        channelId: string;
        channelType: null;
      }) => void;
    };
    bridge.__BUZZ_E2E_MUTATE_CHANNEL__?.({ channelId, channelType: null });
    await bridge.__BUZZ_E2E_INVALIDATE_CHANNELS__?.();
  }, GENERAL_CHANNEL_ID);

  await expect(aliceSuggestion).toHaveCount(0);
});

test("relay-only shared agents appear in forum mentions", async ({ page }) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [MOCK_VIEWER_PUBKEY],
        channelNames: ["watercooler"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-watercooler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
  await page.getByRole("button", { name: "Start a new post..." }).click();

  await page.getByTestId("message-input").fill("@quinn");

  await expect(
    page.getByTestId("mention-autocomplete").getByText("quinn"),
  ).toBeVisible();
});

test("forum sends revalidate relay-agent authorization before signing", async ({
  page,
}) => {
  await installMockBridge(page, {
    deferredComposerUploads: true,
    uploadDescriptors: [
      {
        url: `https://mock.relay/media/${"f".repeat(64)}.pdf`,
        sha256: "f".repeat(64),
        size: 12345,
        type: "application/pdf",
        uploaded: Math.floor(Date.now() / 1000),
        filename: "forum-race.pdf",
      },
    ],
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [MOCK_VIEWER_PUBKEY],
        channelNames: ["watercooler"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-watercooler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");
  await page.getByRole("button", { name: "Start a new post..." }).click();

  const input = page.getByTestId("message-input");
  await input.fill("@quinn");
  await page.getByTestId("mention-autocomplete").getByText("quinn").click();
  await page.keyboard.type("hello");
  await page.getByRole("button", { name: "Attach file" }).click();
  const removeAttachment = page.getByRole("button", {
    name: "Remove attachment",
  });
  await expect(removeAttachment).toBeVisible();
  await page.evaluate(() => {
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.agentListDelayMs = 1_000;
    window.__BUZZ_E2E__.mock.relayAgentListErrors = Array(100).fill(
      "mock forum directory revoked before send",
    );
  });

  await page.getByTestId("send-message").click();
  await expect(input).toHaveAttribute("contenteditable", "false");
  await expect(removeAttachment).toBeDisabled();
  await removeAttachment.evaluate((button: HTMLButtonElement) =>
    button.click(),
  );
  await expect(removeAttachment).toBeVisible();
  await input.focus();
  await page.keyboard.type(" later edit");
  await expect(input).toContainText("@quinn hello");
  await expect(input).not.toContainText("later edit");

  const outgoingContent = `@quinn hello\n[forum-race.pdf](https://mock.relay/media/${"f".repeat(64)}.pdf)`;
  await expect
    .poll(() => readOutgoingMentionPubkeys(page, outgoingContent))
    .not.toBeNull();
  await expect
    .poll(() => readOutgoingMentionPubkeys(page, outgoingContent))
    .not.toContain(ALLOWLIST_RELAY_AGENT_PUBKEY);
});

test("relay-only allowlisted agents are visible in channel mentions", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [MOCK_VIEWER_PUBKEY],
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@quinn");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("quinn")).toBeVisible();
  await expect(dropdown.getByText("agent")).toBeVisible();
});

test("relay-agent directory errors fail closed and recover after a fresh fetch", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgentListErrors: ["mock directory unavailable", null],
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [MOCK_VIEWER_PUBKEY],
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill("@quinn");
  await expect(autocomplete(page)).toHaveCount(0);

  await page.evaluate(async () => {
    await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
      queryKey: ["relay-agents"],
    });
  });
  await expect(autocomplete(page).getByText("quinn")).toBeVisible();

  await page.evaluate(() => {
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.agentListDelayMs = 1_000;
    void window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
      queryKey: ["relay-agents"],
    });
  });
  await expect(autocomplete(page).getByText("quinn")).toHaveCount(0);
  await expect(autocomplete(page).getByText("quinn")).toBeVisible();
});

test("relay-only allowlisted agents emit a p tag when sent", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [MOCK_VIEWER_PUBKEY],
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.evaluate(
    async ({ channelId, pubkey }) => {
      const invoke = window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      if (!invoke) throw new Error("Mock bridge is not installed.");
      await invoke("add_channel_members", {
        channelId,
        pubkeys: [pubkey],
        role: "bot",
      });
      await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
        queryKey: ["channels"],
      });
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
    },
  );

  const input = page.getByTestId("message-input");
  for (let attempt = 0; attempt < 5; attempt++) {
    await input.fill("@quinn");
    const quinnRow = autocomplete(page).locator("button", { hasText: "quinn" });
    await expect(quinnRow).toBeVisible();
    await quinnRow.click();
    await page.keyboard.type("hello");
    await expect(input).toHaveText("@quinn hello");
  }
  const baselineCommands = await readCommandLog(page);
  await page.getByTestId("send-message").click();

  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "@quinn hello"))
    .toContain(ALLOWLIST_RELAY_AGENT_PUBKEY);

  const commands = await readCommandLog(page);
  expect(commandCount(commands, "revalidate_relay_agents")).toBe(
    commandCount(baselineCommands, "revalidate_relay_agents") + 2,
  );
  expect(commandCount(commands, "list_relay_agents")).toBe(
    commandCount(baselineCommands, "list_relay_agents"),
  );
});

test("managed agents keep their p tag when relay discovery fails before send", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        status: "running",
      },
    ],
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [MOCK_VIEWER_PUBKEY],
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();

  const input = page.getByTestId("message-input");
  await input.fill("@quinn");
  const quinnRow = autocomplete(page).locator("button", { hasText: "quinn" });
  await expect(quinnRow).toBeVisible();
  await quinnRow.click();
  await expect(input).toHaveText("@quinn ");
  await page.keyboard.type("hello");
  await expect(input).toHaveText("@quinn hello");

  await page.evaluate(() => {
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.relayAgentListErrors = Array(5).fill(
      "mock unrelated relay directory failure",
    );
  });
  await page.getByTestId("send-message").click();

  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "@quinn hello"))
    .toContain(ALLOWLIST_RELAY_AGENT_PUBKEY);
});

test("targeted revocation before send causes no agent side effects", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [MOCK_VIEWER_PUBKEY],
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.evaluate(
    async ({ channelId, pubkey }) => {
      const invoke = window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      if (!invoke) throw new Error("Mock bridge is not installed.");
      await invoke("add_channel_members", {
        channelId,
        pubkeys: [pubkey],
        role: "bot",
      });
      await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
        queryKey: ["channels"],
      });
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
    },
  );
  const input = page.getByTestId("message-input");
  await input.fill("@quinn");
  const quinnRow = autocomplete(page).locator("button", { hasText: "quinn" });
  await expect(quinnRow).toBeVisible();
  await quinnRow.click();
  await page.keyboard.type("hello");

  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.relayAgentRevalidationRevokedPubkeys = [pubkey];
  }, ALLOWLIST_RELAY_AGENT_PUBKEY);
  const baselineCommands = await readCommandLog(page);
  await page.getByTestId("send-message").click();

  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "@quinn hello"))
    .not.toBeNull();
  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "@quinn hello"))
    .not.toContain(ALLOWLIST_RELAY_AGENT_PUBKEY);
  const commands = await readCommandLog(page);
  expect(commandCount(commands, "revalidate_relay_agents")).toBe(
    commandCount(baselineCommands, "revalidate_relay_agents") + 2,
  );
  expect(commandCount(commands, "list_relay_agents")).toBe(
    commandCount(baselineCommands, "list_relay_agents"),
  );
  for (const command of [
    "add_channel_members",
    "start_managed_agent",
    "attach_managed_agent",
    "sync_agents_to_active_huddle",
  ]) {
    expect(commandCount(commands, command)).toBe(
      commandCount(baselineCommands, command),
    );
  }
});

test("selected relay agents revoked after the invite prompt cause no side effects", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [MOCK_VIEWER_PUBKEY],
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill("@quinn");
  const quinnRow = autocomplete(page).locator("button", { hasText: "quinn" });
  await expect(quinnRow).toBeVisible();
  await quinnRow.click();
  await page.keyboard.type("hello");
  await page.getByTestId("send-message").click();
  const inviteButton = page.getByRole("button", {
    name: "Invite",
    exact: true,
  });
  await expect(inviteButton).toBeVisible();

  await page.evaluate(() => {
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.relayAgentListErrors = Array(5).fill(
      "mock directory revoked after invite prompt",
    );
  });
  const baselineCommands = await readCommandLog(page);
  await inviteButton.click();

  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "@quinn hello"))
    .not.toBeNull();
  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "@quinn hello"))
    .not.toContain(ALLOWLIST_RELAY_AGENT_PUBKEY);
  const commands = await readCommandLog(page);
  for (const command of [
    "add_channel_members",
    "start_managed_agent",
    "attach_managed_agent",
    "sync_agents_to_active_huddle",
  ]) {
    expect(commandCount(commands, command)).toBe(
      commandCount(baselineCommands, command),
    );
  }
});

test("selected relay agents revoked during send emit no p tag", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [MOCK_VIEWER_PUBKEY],
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill("@quinn");
  const quinnRow = autocomplete(page).locator("button", { hasText: "quinn" });
  await expect(quinnRow).toBeVisible();
  await quinnRow.click();
  await page.keyboard.type("hello");

  await page.evaluate(() => {
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.agentListDelayMs = 300;
  });
  await page.getByTestId("send-message").click();
  await page.getByRole("button", { name: "Invite", exact: true }).click();
  await page.evaluate(() => {
    window.__BUZZ_E2E__.mock ??= {};
    window.__BUZZ_E2E__.mock.relayAgentListErrors = Array(100).fill(
      "mock directory revoked mid-send",
    );
  });

  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "@quinn hello"))
    .not.toBeNull();
  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "@quinn hello"))
    .not.toContain(ALLOWLIST_RELAY_AGENT_PUBKEY);
});

test("owner-only builds admit cross-owner relay agents authorized by allowlist", async ({
  page,
}) => {
  await installMockBridge(page, {
    ownerOnlyAccessBuild: true,
    searchProfiles: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        displayName: "quinn",
        ownerPubkey: TEST_IDENTITIES.outsider.pubkey,
        isAgent: true,
      },
    ],
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        ownerPubkey: TEST_IDENTITIES.outsider.pubkey,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [MOCK_VIEWER_PUBKEY],
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.evaluate(
    async ({ channelId, pubkey }) => {
      const invoke = window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      if (!invoke) throw new Error("Mock bridge is not installed.");
      await invoke("add_channel_members", {
        channelId,
        pubkeys: [pubkey],
        role: "bot",
      });
      await window.__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries({
        queryKey: ["channels"],
      });
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
    },
  );
  const input = page.getByTestId("message-input");
  await input.fill("@quinn");
  const quinnRow = autocomplete(page).locator("button", { hasText: "quinn" });
  await expect(quinnRow).toBeVisible();
  await quinnRow.click();
  await page.keyboard.type("hello");
  await page.getByTestId("send-message").click();

  await expect
    .poll(() => readOutgoingMentionPubkeys(page, "@quinn hello"))
    .toContain(ALLOWLIST_RELAY_AGENT_PUBKEY);
});

test("owner-only builds show verified same-owner relay agents", async ({
  page,
}) => {
  await installMockBridge(page, {
    ownerOnlyAccessBuild: true,
    searchProfiles: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        displayName: "quinn",
        ownerPubkey: MOCK_VIEWER_PUBKEY,
        isAgent: true,
      },
    ],
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        ownerPubkey: MOCK_VIEWER_PUBKEY,
        name: "quinn",
        respondTo: "owner-only",
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.getByTestId("message-input").fill("@quinn");

  await expect(autocomplete(page).getByText("quinn")).toBeVisible();
});

test("relay-only allowlisted agents stay hidden outside their channel", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [MOCK_VIEWER_PUBKEY],
        channelNames: ["agents"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill("@quinn");

  await expect(autocomplete(page)).toHaveCount(0);
});

test("owner-only builds admit cross-owner relay agents authorized for anyone", async ({
  page,
}) => {
  await installMockBridge(page, {
    ownerOnlyAccessBuild: true,
    searchProfiles: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        displayName: "quinn",
        ownerPubkey: TEST_IDENTITIES.outsider.pubkey,
        isAgent: true,
      },
    ],
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        ownerPubkey: TEST_IDENTITIES.outsider.pubkey,
        name: "quinn",
        respondTo: "anyone",
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill("@quinn");

  await expect(autocomplete(page).getByText("quinn")).toBeVisible();
});

test("relay-only excluded agents stay hidden from channel mentions", async ({
  page,
}) => {
  await installMockBridge(page, {
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [TEST_IDENTITIES.outsider.pubkey],
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill("@quinn");

  await expect(autocomplete(page)).toHaveCount(0);
});

test("shared agents wait for initial directory authorization", async ({
  page,
}) => {
  await installMockBridge(page, {
    agentListDelayMs: 1_000,
    relayAgents: [
      {
        pubkey: ALLOWLIST_RELAY_AGENT_PUBKEY,
        name: "quinn",
        respondTo: "allowlist",
        respondToAllowlist: [MOCK_VIEWER_PUBKEY],
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill("@quinn");

  await expect(autocomplete(page)).toHaveCount(0);
  await expect(autocomplete(page).getByText("quinn")).toBeVisible({
    timeout: 3_000,
  });
});

test("mentioning an in-channel stopped managed agent starts it before sending", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: IN_CHANNEL_MANAGED_AGENT_PUBKEY,
        name: "fizz",
        status: "stopped",
        channelNames: ["general"],
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Hey @fizz");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("fizz")).toBeVisible();
  await expect(dropdown.getByText("agent")).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" can you help?");

  const baselineStartCount = commandCount(
    await readCommandLog(page),
    "start_managed_agent",
  );
  await page.getByTestId("send-message").click();

  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "start_managed_agent"),
    )
    .toBeGreaterThan(baselineStartCount);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention].agent-mention-highlight", { hasText: "fizz" });
  await expect(mentionChip).toBeVisible();
});

test("mentioning an in-channel provider managed agent deploys it before sending", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: OUT_OF_CHANNEL_PROVIDER_AGENT_PUBKEY,
        name: "portal",
        status: "not_deployed",
        channelNames: ["general"],
        backend: {
          type: "provider",
          id: "portal",
          config: { region: "test" },
        },
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Hey @portal");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("portal")).toBeVisible();
  await expect(dropdown.getByText("agent")).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" can you help?");

  const baselineStartCount = commandCount(
    await readCommandLog(page),
    "start_managed_agent",
  );
  await page.getByTestId("send-message").click();

  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "start_managed_agent"),
    )
    .toBeGreaterThan(baselineStartCount);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention].agent-mention-highlight", { hasText: "portal" });
  await expect(mentionChip).toBeVisible();
});

test("mentioning a non-member managed agent adds and starts it before sending", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: OUT_OF_CHANNEL_MANAGED_AGENT_PUBKEY,
        name: "fizz",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Loop in @fizz");

  const dropdown = autocomplete(page);
  const fizzRow = dropdown.locator("button", { hasText: "fizz" });
  await expect(fizzRow).toBeVisible();
  await expect(fizzRow.getByText("not in channel")).toBeVisible();
  await input.press("Enter");

  const baselineCommands = await readCommandLog(page);
  const baselineAddCount = commandCount(
    baselineCommands,
    "add_channel_members",
  );
  const baselineStartCount = commandCount(
    baselineCommands,
    "start_managed_agent",
  );

  await page.getByTestId("send-message").click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);

  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "add_channel_members"),
    )
    .toBeGreaterThan(baselineAddCount);
  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "start_managed_agent"),
    )
    .toBeGreaterThan(baselineStartCount);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention].agent-mention-highlight", { hasText: "fizz" });
  await expect(mentionChip).toBeVisible();
});

test("mentioning a non-member provider managed agent deploys it before sending", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: OUT_OF_CHANNEL_PROVIDER_AGENT_PUBKEY,
        name: "portal",
        status: "not_deployed",
        backend: {
          type: "provider",
          id: "portal",
          config: { region: "test" },
        },
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Loop in @portal");

  const dropdown = autocomplete(page);
  const portalRow = dropdown.locator("button", { hasText: "portal" });
  await expect(portalRow).toBeVisible();
  await expect(portalRow.getByText("not in channel")).toBeVisible();
  await input.press("Enter");

  const baselineCommands = await readCommandLog(page);
  const baselineAddCount = commandCount(
    baselineCommands,
    "add_channel_members",
  );
  const baselineStartCount = commandCount(
    baselineCommands,
    "start_managed_agent",
  );

  await page.getByTestId("send-message").click();
  await expect(page.getByRole("alertdialog")).toHaveCount(0);

  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "add_channel_members"),
    )
    .toBeGreaterThan(baselineAddCount);
  await expect
    .poll(async () =>
      commandCount(await readCommandLog(page), "start_managed_agent"),
    )
    .toBeGreaterThan(baselineStartCount);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention].agent-mention-highlight", { hasText: "portal" });
  await expect(mentionChip).toBeVisible();
});

test("system add rows use plain names while remove rows retain agent mention styling", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: OUT_OF_CHANNEL_PROVIDER_AGENT_PUBKEY,
        name: "portal",
        status: "deployed",
        backend: {
          type: "provider",
          id: "portal",
          config: { region: "test" },
        },
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general", SYSTEM_MESSAGE_KIND);

  await page.evaluate(
    ({ actorPubkey, kind, targetPubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: JSON.stringify({
          type: "member_joined",
          actor: actorPubkey,
          target: targetPubkey,
        }),
        kind,
      });
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: JSON.stringify({
          type: "member_removed",
          actor: actorPubkey,
          target: targetPubkey,
        }),
        kind,
      });
    },
    {
      actorPubkey: TEST_IDENTITIES.tyler.pubkey,
      kind: SYSTEM_MESSAGE_KIND,
      targetPubkey: OUT_OF_CHANNEL_PROVIDER_AGENT_PUBKEY,
    },
  );

  const addedRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "portal" })
    .filter({ hasText: "added by" });
  const removedRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "removed portal from the channel" });

  const addedName = addedRow.getByText("portal", { exact: true });
  await expect(addedName).toBeVisible();
  await expect(addedName).not.toHaveAttribute("data-mention");
  await expect(
    removedRow.locator("[data-mention].agent-mention-highlight", {
      hasText: "portal",
    }),
  ).toHaveText("portal");
});

test("groups contiguous arrival activity with hidden names in the standard tooltip", async ({
  page,
}) => {
  const actor = {
    pubkey: "10".repeat(32),
    displayName: "Alice Chen",
  };
  const targets = [
    { pubkey: "11".repeat(32), displayName: "Erica Chapman" },
    { pubkey: "12".repeat(32), displayName: "Peter Griffin" },
    { pubkey: "13".repeat(32), displayName: "Marcia Thomas" },
    { pubkey: "14".repeat(32), displayName: "Jordan Lee" },
    { pubkey: "15".repeat(32), displayName: "Olivia Park" },
    { pubkey: "16".repeat(32), displayName: "Sam Rivera" },
  ];
  await installMockBridge(page, {
    searchProfiles: [actor, ...targets],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general", SYSTEM_MESSAGE_KIND);

  await page.evaluate(
    ({ actorPubkey, addedTargets, kind }) => {
      const createdAt = Math.floor(Date.now() / 1_000);
      for (const [index, target] of addedTargets.entries()) {
        window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
          channelName: "general",
          content: JSON.stringify({
            type: "member_joined",
            actor: actorPubkey,
            target: target.pubkey,
          }),
          createdAt: createdAt + index,
          kind,
        });
      }
    },
    {
      actorPubkey: actor.pubkey,
      addedTargets: targets,
      kind: SYSTEM_MESSAGE_KIND,
    },
  );
  await waitForTimelineSettled(page);

  const groupedRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "added by Alice Chen, along with" });
  for (const visibleName of [
    "Erica Chapman",
    "Peter Griffin",
    "Marcia Thomas",
  ]) {
    await expect(groupedRow).toContainText(visibleName);
  }
  await expect(
    groupedRow.locator("p").filter({ hasText: "added by Alice Chen" }),
  ).toContainText(
    "Erica Chapman added by Alice Chen, along with Peter Griffin, Marcia Thomas, Jordan Lee, and 2 others",
  );
  const avatarStack = groupedRow.getByTestId("system-message-avatar-stack");
  await expect(avatarStack).toHaveCount(1);
  await expect(avatarStack.getByTestId("system-message-avatar")).toHaveCount(5);
  await expect(
    groupedRow.locator("p").filter({ hasText: "added by Alice Chen" }),
  ).toHaveCSS("text-align", "left");
  await expect(groupedRow.locator("[data-mention]")).toHaveCount(0);

  const visibleName = groupedRow.getByText("Peter Griffin", { exact: true });
  await expect(visibleName).toHaveCSS("text-decoration-line", "none");
  await visibleName.hover();
  await expect(visibleName).toHaveCSS("text-decoration-line", "underline");

  const othersTrigger = groupedRow.getByRole("button", { name: "2 others" });
  // Park the pointer off-target first: the previous hover leaves the mouse at a
  // fixed viewport point, and any later reflow (new rows, scroll-to-bottom, a
  // different text wrap) can slide this button under it. Without this the
  // assertion measures where the mouse happens to be, not the resting style.
  await page.mouse.move(0, 0);
  await expect(othersTrigger).toHaveCSS("text-decoration-line", "none");
  await othersTrigger.hover();
  await expect(othersTrigger).toHaveCSS("text-decoration-line", "underline");

  const tooltip = page.getByRole("tooltip");
  await expect(tooltip).toContainText("Olivia Park");
  await expect(tooltip).toContainText("Sam Rivera");

  await expect(avatarStack.locator("..")).toHaveCSS("align-items", "center");
});

test("system agent profile exposes owned agent actions", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general", SYSTEM_MESSAGE_KIND);

  await page.evaluate(
    ({ actorPubkey, kind, targetPubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: JSON.stringify({
          type: "member_joined",
          actor: actorPubkey,
          target: targetPubkey,
        }),
        kind,
      });
    },
    {
      actorPubkey: TEST_IDENTITIES.tyler.pubkey,
      kind: SYSTEM_MESSAGE_KIND,
      targetPubkey: PROFILE_ONLY_AGENT_PUBKEY,
    },
  );
  await waitForTimelineSettled(page);

  const joinedRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "mira" })
    .filter({ hasText: "added by" });
  const agentName = joinedRow.getByText("mira", { exact: true });
  await expect(agentName).toHaveText("mira");
  await expect(agentName).not.toHaveAttribute("data-mention");
  await agentName.hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expectOwnedAgentProfileActions(
    profilePopover,
    PROFILE_ONLY_AGENT_PUBKEY,
  );
});

test("system agent activity avatar stack is decorative", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await waitForMockLiveSubscription(page, "random", SYSTEM_MESSAGE_KIND);

  await page.evaluate(
    ({ kind, targetPubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: JSON.stringify({
          type: "member_joined",
          actor: targetPubkey,
          target: targetPubkey,
        }),
        kind,
      });
    },
    {
      kind: SYSTEM_MESSAGE_KIND,
      targetPubkey: PROFILE_ONLY_AGENT_PUBKEY,
    },
  );
  await waitForTimelineSettled(page);

  const joinedRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "mira" })
    .filter({ hasText: "joined the channel" });
  const avatarStack = joinedRow.getByTestId("system-message-avatar-stack");
  await expect(avatarStack.getByTestId("system-message-avatar")).toHaveCount(1);
  await expect(avatarStack.locator("button")).toHaveCount(0);
});

test("membership activity folds a member joining then leaving", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await waitForMockLiveSubscription(page, "random", SYSTEM_MESSAGE_KIND);

  await page.evaluate(
    ({ alicePubkey, kind }) => {
      const createdAt = Math.floor(Date.now() / 1_000);
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: JSON.stringify({
          type: "member_joined",
          actor: alicePubkey,
          target: alicePubkey,
        }),
        createdAt,
        kind,
      });
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: JSON.stringify({ type: "member_left", actor: alicePubkey }),
        createdAt: createdAt + 1,
        kind,
      });
    },
    { alicePubkey: TEST_IDENTITIES.alice.pubkey, kind: SYSTEM_MESSAGE_KIND },
  );
  await waitForTimelineSettled(page);
  const lifecycleRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "alice" })
    .filter({ hasText: "joined, then left the channel" });
  await expect(lifecycleRow).toBeVisible();
  await expect(lifecycleRow.getByTestId("system-message-avatar")).toHaveCount(
    1,
  );
});

test("profile-only agent author hides actions without agent access", async ({
  page,
}) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: PROFILE_ONLY_AGENT_PUBKEY,
        displayName: "mira",
        isAgent: true,
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Mira status update.", {
    pubkey: PROFILE_ONLY_AGENT_PUBKEY,
  });
  await waitForTimelineSettled(page);

  const messageRow = page
    .getByTestId("message-row")
    .filter({ hasText: "Mira status update." })
    .first();
  await messageRow.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expectAgentProfileActionsHidden(
    profilePopover,
    PROFILE_ONLY_AGENT_PUBKEY,
  );
});

test("system member-joined rows render the joined person as a plain profile name", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general", SYSTEM_MESSAGE_KIND);

  await page.evaluate(
    ({ kind, pubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: JSON.stringify({
          type: "member_joined",
          actor: pubkey,
          target: pubkey,
        }),
        kind,
      });
    },
    { kind: SYSTEM_MESSAGE_KIND, pubkey: TEST_IDENTITIES.bob.pubkey },
  );
  await waitForTimelineSettled(page);

  const joinedRow = page
    .getByTestId("system-message-row")
    .filter({ hasText: "bob" })
    .filter({ hasText: "joined the channel" });
  const joinedPersonName = joinedRow.getByText("bob", { exact: true });

  await expect(joinedPersonName).toBeVisible();
  await expect(joinedPersonName).not.toHaveAttribute("data-mention");
});

test("a managed non-member agent from a DM can be addressed explicitly", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        name: "charlie",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");

  const input = page.getByTestId("message-input");
  await input.fill("@char");

  const dropdown = autocomplete(page);
  const charlieRow = dropdown.getByTestId(
    `mention-suggestion-${TEST_IDENTITIES.charlie.pubkey}`,
  );
  await expect(charlieRow.getByText("charlie")).toBeVisible();
  await expect(autocomplete(page)).toHaveCount(1);
  await expect(input.locator(".mention-chip")).toHaveCount(0);
  await charlieRow
    .getByRole("button", {
      name: "Automatically mention charlie",
      exact: true,
    })
    .click();

  await expect(input).toHaveText("@charlie ");
  await expect(input.locator(".agent-mention-highlight")).toHaveText("charlie");
  await expect(
    page.getByTestId(`composer-address-lock-${TEST_IDENTITIES.charlie.pubkey}`),
  ).toBeVisible();
});

test("global non-member people can be selected from channel mentions", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Loop in @out");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("outsider")).toBeVisible();
  await expect(dropdown.getByText("not in channel")).toBeVisible();
});

test("duplicate global people with the same visible identity collapse in channel mentions", async ({
  page,
}) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: CASEY_PROFILE_PUBKEY,
        displayName: "Pip",
      },
      {
        pubkey:
          "2222222222222222222222222222222222222222222222222222222222222222",
        displayName: "Pip",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@pip");

  const dropdown = autocomplete(page);
  await expect(dropdown.locator("button", { hasText: "Pip" })).toHaveCount(1);
});

test("sent non-member person mention uses the normal mention style", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");

  const input = page.getByTestId("message-input");
  await input.fill("Loop in @out");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("outsider")).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" please");
  await page.getByTestId("send-message").click();

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention]", { hasText: "outsider" });
  await expect(mentionChip).toBeVisible();
  await expect(mentionChip).toHaveClass(/inline-chip-icon-human/);
});

test("sent managed non-member agent mention uses the agent mention style", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        name: "charlie",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");

  const input = page.getByTestId("message-input");
  await input.fill("Loop in @char");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("charlie")).toBeVisible();
  await input.press("Enter");
  await page.keyboard.type(" too");
  await page.getByTestId("send-message").click();

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention]", { hasText: "charlie" });
  await expect(mentionChip).toBeVisible();
  await expect(mentionChip).toHaveText("charlie");
  await expect(mentionChip).toHaveClass(/agent-mention-highlight/);
});

test("mention button opens autocomplete and inserts a selected member", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Hey ");
  await page.getByTestId("message-insert-mention").click();

  const dropdown = autocomplete(page);
  await expect(dropdown).toBeVisible();
  await dropdown.getByText("bob").click();

  await expect(input).toHaveText("Hey @bob ");
});

test("inserting a mention preserves Shift+Enter newlines (regression: bug #2)", async ({
  page,
}) => {
  // Before PR #618, mention insertion round-tripped through
  // `setContent(markdown)`, which collapsed every Shift+Enter hard
  // break to a single space. After the fix, autocomplete uses a
  // native ProseMirror `tr.insertText` transaction and the line
  // breaks survive.
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.click();
  await page.keyboard.type("line one");
  await page.keyboard.press("Shift+Enter");
  await page.keyboard.type("line two @bo");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("bob")).toBeVisible();
  await dropdown.getByText("bob").click();

  // Both lines must still be present, separated by a real line break
  // (rendered as a `<br>` by Tiptap; the projection sees `\n`).
  await expect(input).toHaveText(/line one[\s\S]*line two @bob/);
  await expect(input.locator("br")).toHaveCount(1);
});

test("keyboard navigation selects mention with Enter", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@bo");

  const dropdown = autocomplete(page);
  await expect(dropdown.getByText("bob")).toBeVisible();

  // Press Enter to select the first (and only) suggestion
  await input.press("Enter");

  // Should insert @bob and NOT send the message
  await expect(input).toHaveText("@bob ");
});

test("Escape dismisses autocomplete dropdown", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("@");

  const dropdown = autocomplete(page);
  await expect(dropdown).toBeVisible();

  await input.press("Escape");

  await expect(dropdown).not.toBeVisible();
});

test("mention text is highlighted in sent messages", async ({ page }) => {
  const suffix = ` check this out ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Hey @bo");
  await autocomplete(page).getByText("bob").click();
  await expect(input).toHaveText("Hey @bob ");
  await page.keyboard.type(suffix);
  await page.getByTestId("send-message").click();

  await waitForTimelineSettled(page);

  const mentionChip = page
    .getByTestId("message-row")
    .last()
    .locator("[data-mention].mention-chip", { hasText: "bob" });
  await expect(mentionChip).toBeVisible();
  await expect(mentionChip).toHaveText("bob");
  await expect(mentionChip).toHaveClass(/inline-chip-icon-human/);
});

test("clicking author name opens user profile panel", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // The seed message in general is from the mock identity (npub1mock...)
  const firstMessage = page.getByTestId("message-row").first();
  const authorButton = firstMessage.locator("button", {
    hasText: "npub1mock...",
  });
  await authorButton.click();

  // Click now opens the full profile panel instead of the popover
  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("deadbeef");
});

test("hovering avatar opens popover, clicking opens profile panel", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const firstMessage = page.getByTestId("message-row").first();
  const avatarButton = firstMessage.locator("button").first();

  // Hover should open the popover
  await avatarButton.hover();
  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();

  // Click should close the popover and open the profile panel
  await avatarButton.click();
  await expect(profilePopover).toHaveCount(0);
  await expect(page.getByTestId("user-profile-panel")).toBeVisible();
});

test("clicking a mention chip in the timeline opens the profile panel", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Ping @bob about the launch", {
    mentionPubkeys: [TEST_IDENTITIES.bob.pubkey],
  });
  await waitForTimelineSettled(page);

  const mentionChip = page
    .getByTestId("message-row")
    .filter({ hasText: "Ping bob about the launch" })
    .locator("[data-mention]", { hasText: "bob" });
  await expect(mentionChip).toBeVisible();
  await mentionChip.click();

  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("bob");
});

test("mention text matching the kind-0 name alias resolves and opens the profile panel", async ({
  page,
}) => {
  // bob's mock profile has display_name "bob" and kind-0 name "bobby". A
  // message that says "@bobby" (how agents/CLI resolve mentions at send time)
  // must still render a clickable chip bound to bob's pubkey.
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Ask @bobby to review the doc", {
    mentionPubkeys: [TEST_IDENTITIES.bob.pubkey],
  });
  await waitForTimelineSettled(page);

  const mentionChip = page
    .getByTestId("message-row")
    .filter({ hasText: "Ask bobby to review the doc" })
    .locator("[data-mention]", { hasText: "bobby" });
  await expect(mentionChip).toBeVisible();
  await mentionChip.click();

  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("bob");
});

test("clicking a mention chip in a forum post opens the profile panel", async ({
  page,
}) => {
  await page.goto("/");
  // Seed the forum post before entering the channel — forum views load from
  // the mock store on fetch, so no live subscription is needed.
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await emitMockMessage(page, "watercooler", "Welcome aboard @bob!", {
    kind: 45001,
    mentionPubkeys: [TEST_IDENTITIES.bob.pubkey],
  });

  await page.getByTestId("channel-watercooler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");

  const mentionChip = page.locator("[data-mention]", { hasText: "bob" });
  await expect(mentionChip).toBeVisible();
  await mentionChip.click();

  const panel = page.getByTestId("user-profile-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toContainText("bob");
  // The chip click must not bubble into the card and open the thread view.
  await expect(page.getByRole("button", { name: "Back to posts" })).toHaveCount(
    0,
  );
});

test("agent profile popover shows its owner", async ({ page }) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: OWNED_AGENT_PROFILE_PUBKEY,
        displayName: "Pollen",
        ownerPubkey: TEST_IDENTITIES.bob.pubkey,
        isAgent: true,
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Pollen checking in.", {
    pubkey: OWNED_AGENT_PROFILE_PUBKEY,
  });
  await waitForTimelineSettled(page);

  const pollenMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Pollen checking in." })
    .first();
  await pollenMessage.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-owner-${OWNED_AGENT_PROFILE_PUBKEY}`,
    ),
  ).toHaveText("managed by bob");
});

test("agent profile popover labels an agent owned by the viewer as you", async ({
  page,
}) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: OWNED_AGENT_PROFILE_PUBKEY,
        displayName: "Pollen",
        ownerPubkey: MOCK_VIEWER_PUBKEY,
        isAgent: true,
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Pollen checking in.", {
    pubkey: OWNED_AGENT_PROFILE_PUBKEY,
  });
  await waitForTimelineSettled(page);

  const pollenMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Pollen checking in." })
    .first();
  await pollenMessage.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-owner-${OWNED_AGENT_PROFILE_PUBKEY}`,
    ),
  ).toHaveText("managed by you");
});

test("agent profile popover falls back to the owner's pubkey", async ({
  page,
}) => {
  await installMockBridge(page, {
    searchProfiles: [
      {
        pubkey: OWNED_AGENT_PROFILE_PUBKEY,
        displayName: "Pollen",
        ownerPubkey: CASEY_PROFILE_PUBKEY,
        isAgent: true,
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Pollen checking in.", {
    pubkey: OWNED_AGENT_PROFILE_PUBKEY,
  });
  await waitForTimelineSettled(page);

  const pollenMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Pollen checking in." })
    .first();
  await pollenMessage.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-owner-${OWNED_AGENT_PROFILE_PUBKEY}`,
    ),
  ).toHaveText("managed by 11111111…1111");
});

test("human profile popover does not show an owner", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Bob checking in.", {
    pubkey: TEST_IDENTITIES.bob.pubkey,
  });
  await waitForTimelineSettled(page);

  const bobMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Bob checking in." })
    .first();
  await bobMessage.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expect(
    profilePopover.locator('[data-testid^="user-profile-popover-owner-"]'),
  ).toHaveCount(0);
});

test("owned bot profile exposes message and huddle actions", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        name: "charlie",
        status: "online",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  const charlieMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Indexing the channel catalog now." })
    .first();
  await charlieMessage.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expectOwnedAgentProfileActions(
    profilePopover,
    TEST_IDENTITIES.charlie.pubkey,
  );

  await profilePopover
    .getByTestId(
      `user-profile-popover-huddle-${TEST_IDENTITIES.charlie.pubkey}`,
    )
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).find(
            (entry) => entry.command === "start_huddle",
          )?.payload,
      ),
    )
    .toMatchObject({ memberPubkeys: [TEST_IDENTITIES.charlie.pubkey] });
});

test("owned agent mention profile exposes message and huddle actions", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.charlie.pubkey,
        name: "charlie",
        status: "online",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Can @charlie take this?", {
    mentionPubkeys: [TEST_IDENTITIES.charlie.pubkey],
  });
  await waitForTimelineSettled(page);

  const mentionChip = page
    .getByTestId("message-row")
    .filter({ hasText: "Can charlie take this?" })
    .locator("[data-mention].agent-mention-highlight", { hasText: "charlie" });
  await expect(mentionChip).toBeVisible();
  await mentionChip.hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expectOwnedAgentProfileActions(
    profilePopover,
    TEST_IDENTITIES.charlie.pubkey,
  );
});

test("profile popover wave sends a direct message for a human profile", async ({
  page,
}) => {
  await installMockBridge(page, { sendMessageDelayMs: 2_500 });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Bob says hello.", {
    pubkey: TEST_IDENTITIES.bob.pubkey,
  });
  await waitForTimelineSettled(page);

  const bobMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Bob says hello." })
    .first();
  await bobMessage.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-message-${TEST_IDENTITIES.bob.pubkey}`,
    ),
  ).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-huddle-${TEST_IDENTITIES.bob.pubkey}`,
    ),
  ).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-wave-${TEST_IDENTITIES.bob.pubkey}`,
    ),
  ).toBeVisible();
  await profilePopover
    .getByTestId(`user-profile-popover-wave-${TEST_IDENTITIES.bob.pubkey}`)
    .click();

  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");
  const waveAttachment = page.getByTestId("message-wave-attachment");
  await expect(waveAttachment).toBeVisible({ timeout: 1_500 });
  await expect(page.getByText("Sending")).toHaveCount(0, { timeout: 4_000 });
  await waitForTimelineSettled(page);
  await expect(waveAttachment).toContainText("👋");
  await expect(waveAttachment).toContainText("npub1mock... waved at you.");
  await expect(waveAttachment).toContainText("Start a huddle to talk to them.");
  await expect(
    waveAttachment.getByRole("button", { name: "Start huddle" }),
  ).toBeVisible();

  const commandLog = await readCommandPayloadLog(page);
  expect(commandLog).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        command: "send_channel_message",
        payload: expect.objectContaining({
          content: expect.stringContaining("npub1mock... waved at you."),
        }),
      }),
    ]),
  );
  expect(commandLog).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        command: "send_channel_message",
        payload: expect.objectContaining({
          content: expect.stringContaining("<!-- buzz:wave:v1 -->"),
        }),
      }),
    ]),
  );
});

test("delayed inaccessible agent profile keeps all actions hidden", async ({
  page,
}) => {
  await installMockBridge(page, {
    agentListDelayMs: 5_000,
    relayAgents: [
      {
        pubkey: DELAYED_RELAY_AGENT_PUBKEY,
        name: "orbit",
        channelNames: ["general"],
      },
    ],
    searchProfiles: [
      {
        pubkey: DELAYED_RELAY_AGENT_PUBKEY,
        displayName: "orbit",
      },
    ],
  });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");

  await emitMockMessage(page, "general", "Orbit checking in.", {
    pubkey: DELAYED_RELAY_AGENT_PUBKEY,
  });
  await waitForTimelineSettled(page);

  const orbitMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Orbit checking in." })
    .first();
  await orbitMessage.locator("button").first().hover();

  const profilePopover = page.locator(
    '[data-testid="user-profile-popover"][data-state="open"]',
  );
  await expect(profilePopover).toBeVisible();
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-message-${DELAYED_RELAY_AGENT_PUBKEY}`,
    ),
  ).toHaveCount(0);
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-wave-${DELAYED_RELAY_AGENT_PUBKEY}`,
    ),
  ).toHaveCount(0);
  await expect(
    profilePopover.getByTestId(
      `user-profile-popover-huddle-${DELAYED_RELAY_AGENT_PUBKEY}`,
    ),
  ).toHaveCount(0);
});
