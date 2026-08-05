import { expect, test } from "@playwright/test";

import {
  KIND_HUDDLE_ENDED,
  KIND_HUDDLE_STARTED,
} from "../../src/shared/constants/kinds";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const HUDDLE_CHANNEL_ID = "11111111-1111-4111-8111-111111111111";
const HUDDLE_PARENT_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const HUDDLE_THREAD_ROOT_ID = "mock-general-welcome";

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
  kind?: number,
) {
  await expect
    .poll(() =>
      page.evaluate(
        ({ kind, name }) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: name,
            kind,
          }) ?? false,
        { kind, name: channelName },
      ),
    )
    .toBe(true);
}

async function setHuddleSnapshot(
  page: import("@playwright/test").Page,
  members: Array<{
    pubkey: string;
    role: "member" | "bot";
  }>,
  transcriptionEnabled: boolean,
) {
  await page.evaluate(
    async ({ nextMembers, nextTranscriptionEnabled }) => {
      const setSnapshot = (
        window as Window & {
          __BUZZ_E2E_SET_MOCK_HUDDLE_SNAPSHOT__?: (input: {
            members: Array<{
              pubkey: string;
              role: "member" | "bot";
            }>;
            transcriptionEnabled: boolean;
          }) => Promise<void>;
        }
      ).__BUZZ_E2E_SET_MOCK_HUDDLE_SNAPSHOT__;
      if (!setSnapshot) {
        throw new Error("Mock huddle snapshot control is not installed.");
      }
      await setSnapshot({
        members: nextMembers,
        transcriptionEnabled: nextTranscriptionEnabled,
      });
    },
    { nextMembers: members, nextTranscriptionEnabled: transcriptionEnabled },
  );
}

async function installFakeHuddleMicrophone(
  page: import("@playwright/test").Page,
) {
  await page.addInitScript(() => {
    const mediaDevices = navigator.mediaDevices ?? ({} as MediaDevices);
    if (!navigator.mediaDevices) {
      Object.defineProperty(navigator, "mediaDevices", {
        configurable: true,
        value: mediaDevices,
      });
    }

    Object.defineProperty(mediaDevices, "enumerateDevices", {
      configurable: true,
      value: async () => [
        {
          deviceId: "mock-huddle-microphone",
          groupId: "mock-huddle-audio",
          kind: "audioinput",
          label: "Mock Huddle Microphone",
          toJSON() {
            return this;
          },
        } as MediaDeviceInfo,
      ],
    });
    Object.defineProperty(mediaDevices, "getUserMedia", {
      configurable: true,
      value: async () => {
        const context = new AudioContext({ sampleRate: 48_000 });
        const oscillator = context.createOscillator();
        const gain = context.createGain();
        const destination = context.createMediaStreamDestination();
        gain.gain.value = 0;
        oscillator.connect(gain);
        gain.connect(destination);
        oscillator.start();
        return destination.stream;
      },
    });
  });
}

test("keeps the drawer open until the huddle is expanded", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("buzz-theme", "buzz-dark");
  });
  await installMockBridge(page, {
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [
        { pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" },
        { pubkey: TEST_IDENTITIES.alice.pubkey, role: "bot" },
      ],
      transcriptionEnabled: true,
    },
  });

  await page.goto("/");

  const gradientUnderlay = page.locator(".buzz-theme-gradient-underlay");
  const openGradient = await gradientUnderlay.evaluate(
    (element) => getComputedStyle(element).backgroundImage,
  );
  expect(openGradient).not.toBe("none");

  const transcriptButton = page.getByRole("button", {
    name: "Stop transcript",
  });
  await expect(transcriptButton).toBeVisible();
  const huddleShell = page.locator(".buzz-huddle-shell");
  await expect(huddleShell).toHaveAttribute("data-huddle-open", "true");
  const huddleBackdrop = page.locator(".buzz-huddle-drawer-backdrop");
  await expect(huddleBackdrop).toHaveClass(/buzz-huddle-drawer-backdrop-open/);
  const [huddleBackdropColor, huddleDrawerColor] = await Promise.all([
    huddleBackdrop.evaluate(
      (element) => getComputedStyle(element).backgroundColor,
    ),
    page
      .locator(".buzz-huddle-drawer")
      .first()
      .evaluate((element) => getComputedStyle(element).backgroundColor),
  ]);
  expect(huddleBackdropColor).toBe(huddleDrawerColor);
  await expect
    .poll(() =>
      huddleBackdrop.evaluate((element) => {
        const shell = element.parentElement;
        const appSurface = shell?.querySelector(".buzz-huddle-app-surface");
        if (!appSurface) return false;
        const backdropStyle = getComputedStyle(element);
        const appStyle = getComputedStyle(appSurface);
        return Number(backdropStyle.zIndex) < Number(appStyle.zIndex);
      }),
    )
    .toBe(true);
  const drawerAgentMenu = page.getByRole("button", {
    name: "Voice settings for alice",
  });
  await expect(
    drawerAgentMenu.getByTestId("huddle-participant-avatar"),
  ).toBeVisible();
  await drawerAgentMenu.click();
  await expect(
    page.getByText("Agent text-to-speech", { exact: true }),
  ).toBeVisible();
  await expect(page.getByText("Agent voice", { exact: true })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Remove alice from huddle" }),
  ).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("profile-huddle-control")).toHaveCount(0);
  await page
    .getByRole("button", { name: "Open huddle in a new window" })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
            (entry) => entry.command === "open_huddle_window",
          ).length,
      ),
    )
    .toBe(1);
  const huddleControl = page.getByTestId("profile-huddle-control");
  await expect(huddleControl).toBeVisible();
  const mainContentSurface = page.locator(
    "[data-buzz-content-surface]:not([data-buzz-content-unframed])",
  );
  await expect(mainContentSurface).toBeVisible();
  const [mainContentColor, huddleCardColor] = await Promise.all([
    mainContentSurface.evaluate(
      (element) => getComputedStyle(element).backgroundColor,
    ),
    huddleControl.evaluate(
      (element) => getComputedStyle(element).backgroundColor,
    ),
  ]);
  expect(huddleCardColor).toBe(mainContentColor);
  await expect(huddleControl).toHaveCSS("border-top-width", "1px");
  await expect(huddleControl).toContainText("In a huddle");
  await expect(huddleControl).toContainText("#general");
  const compactMicButton = huddleControl.getByRole("button", {
    name: /microphone/i,
  });
  await expect(compactMicButton).toBeVisible();
  await expect(compactMicButton).toHaveClass(/bg-destructive\/15/);
  await expect(compactMicButton).toHaveClass(/text-destructive/);
  await expect(
    huddleControl.getByRole("button", { name: "Audio settings" }),
  ).not.toHaveClass(/bg-destructive\/15/);
  const [micContainerRadius, micHoverRadius] = await Promise.all([
    compactMicButton
      .locator("..")
      .evaluate((element) => getComputedStyle(element).borderTopLeftRadius),
    compactMicButton.evaluate(
      (element) => getComputedStyle(element).borderTopLeftRadius,
    ),
  ]);
  expect(micHoverRadius).toBe(micContainerRadius);
  await expect(huddleControl).toContainText("Leave");
  await expect(
    huddleControl.getByRole("button", { name: /huddle speaker/i }),
  ).toHaveCount(0);
  await expect(transcriptButton).toHaveCount(0);
  await huddleControl
    .getByRole("button", { name: "Open huddle window" })
    .click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
            (entry) => entry.command === "open_huddle_window",
          ).length,
      ),
    )
    .toBe(2);
  await huddleControl.getByRole("button", { name: "Leave huddle" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
            (entry) => entry.command === "leave_huddle",
          ).length,
      ),
    )
    .toBe(1);
  await expect(page.locator(".buzz-huddle-shell")).toHaveAttribute(
    "data-huddle-open",
    "false",
  );
  const closedGradient = await gradientUnderlay.evaluate(
    (element) => getComputedStyle(element).backgroundImage,
  );
  expect(closedGradient).toBe(openGradient);
});

test("shows speaker identity on every huddle chat message", async ({
  page,
}) => {
  await installMockBridge(page, {
    windowLabel: `huddle-${HUDDLE_CHANNEL_ID}`,
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      huddleThreadEventId: HUDDLE_THREAD_ROOT_ID,
      members: [
        { pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" },
        { pubkey: TEST_IDENTITIES.alice.pubkey, role: "bot" },
      ],
      transcriptionEnabled: true,
    },
    mock: { threadRepliesDelayMs: 1_500 },
  });
  await page.goto("/");

  await expect
    .poll(() =>
      page.evaluate(
        () => window.__TAURI_INTERNALS__?.metadata.currentWindow.label,
      ),
    )
    .toBe(`huddle-${HUDDLE_CHANNEL_ID}`);
  await expect(page.getByTestId("huddle-transcript-intro")).toBeVisible();
  await expect(page.getByTestId("message-thread-replies-loading")).toHaveCount(
    0,
  );
  const transcriptTopGap = await page.evaluate(() => {
    const participantStrip = document.querySelector(
      '[data-testid="huddle-participant-strip"]',
    );
    const participantTray = participantStrip?.closest("header");
    const transcriptIntro = document.querySelector(
      '[data-testid="huddle-transcript-intro"]',
    );
    if (!participantTray || !transcriptIntro) return null;
    return Math.round(
      transcriptIntro.getBoundingClientRect().top -
        participantTray.getBoundingClientRect().bottom,
    );
  });
  expect(transcriptTopGap).toBe(16);
  await waitForMockLiveSubscription(page, "huddle");
  await page.evaluate(() => {
    const threadRoot = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "huddle",
      content: "First huddle message",
    });
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "huddle",
      content: "Second huddle message",
    });
    if (!threadRoot) throw new Error("Failed to create Huddle thread root.");
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "huddle",
      content: "Agent response flattened into the huddle chat",
      parentEventId: threadRoot.id,
      pubkey: "deadbeef".repeat(8),
    });
  });

  for (const content of [
    "First huddle message",
    "Second huddle message",
    "Agent response flattened into the huddle chat",
  ]) {
    const row = page.getByTestId("message-row").filter({ hasText: content });
    await expect(row).toBeVisible();
    await expect(row.locator('[data-testid^="message-avatar-"]')).toBeVisible();
    await expect(row.getByTestId("message-author")).toBeVisible();
    await expect(row.getByTestId("message-timestamp")).toBeVisible();
  }
  await expect(page.getByTestId("message-thread-panel")).toHaveCount(0);
});

test("ignores persisted community onboarding in the huddle room", async ({
  page,
}) => {
  const persistedTransaction = {
    id: "main-window-community-transaction",
    source: "deep-link-join",
    stage: "claiming",
    relayUrl: "wss://other.example",
    inviteCode: "main-window-invite",
    communityName: "Other community",
    createdAt: "2026-08-03T20:00:00.000Z",
    updatedAt: "2026-08-03T20:00:00.000Z",
  };
  await page.addInitScript((transaction) => {
    window.localStorage.setItem(
      "buzz-community-onboarding-transaction.v1",
      JSON.stringify(transaction),
    );
  }, persistedTransaction);
  const claimRequests: string[] = [];
  page.on("request", (request) => {
    if (request.url().includes("/api/invites/claim")) {
      claimRequests.push(request.url());
    }
  });
  await installMockBridge(page, {
    windowLabel: `huddle-${HUDDLE_CHANNEL_ID}`,
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
    },
  });

  for (let load = 0; load < 2; load += 1) {
    await page.goto("/");
    await expect(page.getByTestId("huddle-transcript-intro")).toBeVisible();
    await expect(
      page.getByText("Setting up your community", { exact: true }),
    ).toHaveCount(0);
    await expect
      .poll(() =>
        page.evaluate(() => {
          const raw = window.localStorage.getItem(
            "buzz-community-onboarding-transaction.v1",
          );
          return raw ? JSON.parse(raw) : null;
        }),
      )
      .toEqual(persistedTransaction);
    await expect
      .poll(() =>
        page.evaluate(() =>
          JSON.parse(window.localStorage.getItem("buzz-communities") ?? "[]"),
        ),
      )
      .not.toContainEqual(
        expect.objectContaining({ relayUrl: persistedTransaction.relayUrl }),
      );
    expect(claimRequests).toHaveLength(0);
  }
});

test("keeps main-app shortcuts from navigating the huddle room", async ({
  page,
}) => {
  await installMockBridge(page, {
    windowLabel: `huddle-${HUDDLE_CHANNEL_ID}`,
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
    },
    pendingCommunityDeepLinks: [
      {
        id: "companion-must-not-consume",
        kind: "connect",
        relayUrl: "wss://other.example",
      },
    ],
  });
  await page.goto("/");

  await expect(page).toHaveURL(
    new RegExp(`/channels/${HUDDLE_CHANNEL_ID.replaceAll("-", "\\-")}`),
  );
  const huddleUrl = page.url();
  const primaryModifier = process.platform === "darwin" ? "Meta" : "Control";

  for (const shortcut of ["Shift+K", "Shift+A", "Comma"]) {
    await page.keyboard.press(`${primaryModifier}+${shortcut}`);
    await expect.poll(() => page.url()).toBe(huddleUrl);
    await expect(page.getByTestId("settings-view")).toHaveCount(0);
  }

  await page.evaluate(async (channelId) => {
    await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.("deep-link-message", {
      channelId,
      messageId: "mock-general-welcome",
      threadRootId: null,
    });
  }, HUDDLE_PARENT_ID);
  await expect.poll(() => page.url()).toBe(huddleUrl);

  const pendingCommunityLink = {
    code: null,
    id: "companion-must-not-consume",
    kind: "connect" as const,
    name: null,
    relayUrl: "wss://other.example",
  };
  await page.evaluate(async () => {
    await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.("deep-link-connect", null);
  });
  await expect
    .poll(() =>
      page.evaluate(async () =>
        window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
          "take_pending_community_deep_link",
        ),
      ),
    )
    .toEqual(pendingCommunityLink);
});

test("speaks the first eligible agent reply with its participant identity", async ({
  page,
}) => {
  await installMockBridge(page, {
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [
        { pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" },
        { pubkey: TEST_IDENTITIES.alice.pubkey, role: "bot" },
      ],
      ttsEnabled: true,
    },
  });
  await page.goto("/");
  await waitForMockLiveSubscription(page, "huddle");

  await page.evaluate((agentPubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "huddle",
      content: "This first reply should be spoken.",
      pubkey: agentPubkey,
    });
  }, TEST_IDENTITIES.alice.pubkey);

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).find(
            (entry) => entry.command === "speak_agent_message",
          )?.payload,
      ),
    )
    .toMatchObject({
      text: "This first reply should be spoken.",
      speakerPubkey: TEST_IDENTITIES.alice.pubkey,
    });
  await expect(
    page.getByRole("img", {
      name: "Anyone can send instructions to this agent",
    }),
  ).toHaveCount(0);
});

test("animates the responding agent with the shared speaker ring", async ({
  page,
}) => {
  await installMockBridge(page, {
    windowLabel: `huddle-${HUDDLE_CHANNEL_ID}`,
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [
        { pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" },
        { pubkey: TEST_IDENTITIES.alice.pubkey, role: "bot" },
      ],
    },
  });
  await page.goto("/");

  const agentAvatar = page
    .getByTestId("huddle-participant-strip")
    .locator(".buzz-huddle-speaking-avatar")
    .nth(1);
  await expect(agentAvatar).toBeVisible();
  await page.evaluate(async (pubkey) => {
    await window.__BUZZ_E2E_EMIT_MOCK_HUDDLE_TTS_SPEAKER__?.({
      pubkey,
      level: 0.8,
    });
  }, TEST_IDENTITIES.alice.pubkey);
  await expect
    .poll(() =>
      agentAvatar.evaluate((element) =>
        element.style.getPropertyValue("--buzz-huddle-speaker-opacity"),
      ),
    )
    .toBe("0.890");

  await page.evaluate(async () => {
    await window.__BUZZ_E2E_EMIT_MOCK_HUDDLE_TTS_SPEAKER__?.({
      pubkey: null,
      level: 0,
    });
  });
  await expect
    .poll(() =>
      agentAvatar.evaluate((element) =>
        element.style.getPropertyValue("--buzz-huddle-speaker-opacity"),
      ),
    )
    .toBe("0");
});

test("stops the speaking agent from the huddle controls", async ({ page }) => {
  await installMockBridge(page, {
    windowLabel: `huddle-${HUDDLE_CHANNEL_ID}`,
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [
        { pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" },
        { pubkey: TEST_IDENTITIES.alice.pubkey, role: "bot" },
      ],
    },
  });
  await page.goto("/");
  const roomMicButton = page.getByRole("button", {
    name: "Microphone unavailable",
  });
  await expect(roomMicButton).toHaveClass(/bg-destructive\/35/);
  await expect(roomMicButton).toHaveClass(/text-destructive/);
  await expect(
    page.getByRole("button", { name: "Audio settings" }),
  ).not.toHaveClass(/bg-destructive\/35/);
  const roomSpeakerButton = page.getByRole("button", {
    name: "Unmute agent speech",
  });
  await expect(roomSpeakerButton).toHaveClass(/bg-destructive\/35/);
  await expect(roomSpeakerButton).toHaveClass(/text-destructive/);
  await expect(
    page.getByRole("button", { name: "Speaker settings" }),
  ).not.toHaveClass(/bg-destructive\/35/);
  await waitForAnimations(page);
  const agentTile = page
    .getByTestId("huddle-participant-strip")
    .getByTestId("huddle-participant-tile")
    .nth(1);
  const agentAvatar = page
    .getByTestId("huddle-participant-strip")
    .locator(".buzz-huddle-speaking-avatar")
    .nth(1);
  const labelSlots = page.getByTestId("huddle-participant-label-slot");
  await expect(agentAvatar).toBeVisible();
  await waitForAnimations(page);
  const idleTileHeight = await agentTile.evaluate(
    (element) => element.offsetHeight,
  );
  const idleAgentLabelBox = await labelSlots.nth(1).boundingBox();
  const humanLabelBox = await labelSlots.nth(0).boundingBox();
  expect(humanLabelBox?.y).toBe(idleAgentLabelBox?.y);
  expect(humanLabelBox?.height).toBe(idleAgentLabelBox?.height);
  expect(humanLabelBox?.width).toBe(idleAgentLabelBox?.width);
  await page.evaluate(async (pubkey) => {
    await window.__BUZZ_E2E_EMIT_MOCK_HUDDLE_TTS_SPEAKER__?.({
      pubkey,
      level: 0.8,
    });
  }, TEST_IDENTITIES.alice.pubkey);
  await expect
    .poll(() =>
      agentAvatar.evaluate((element) =>
        element.style.getPropertyValue("--buzz-huddle-speaker-opacity"),
      ),
    )
    .toBe("0.890");

  const stopButton = page.getByRole("button", { name: /^Stop .* speaking$/ });
  await expect(stopButton).toBeVisible();
  await expect(stopButton).toHaveClass(/bg-destructive\/15/);
  expect(await labelSlots.nth(1).boundingBox()).toEqual(idleAgentLabelBox);
  expect(await agentTile.evaluate((element) => element.offsetHeight)).toBe(
    idleTileHeight,
  );
  await stopButton.click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).findLast(
            (entry) => entry.command === "interrupt_huddle_speech",
          )?.payload ?? null,
      ),
    )
    .toEqual({ agentPubkey: TEST_IDENTITIES.alice.pubkey });
});

test("assigns distinct agent voices and exposes compact per-agent controls", async ({
  page,
}) => {
  await installMockBridge(page, {
    windowLabel: `huddle-${HUDDLE_CHANNEL_ID}`,
    ttsSettings: {
      version: 1,
      agentTextToSpeech: true,
      voicePreferences: ["pocket:vera"],
    },
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [
        { pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" },
        { pubkey: TEST_IDENTITIES.alice.pubkey, role: "bot" },
        { pubkey: TEST_IDENTITIES.bob.pubkey, role: "bot" },
      ],
      ttsEnabled: true,
    },
  });
  await page.goto("/");

  const voiceMenus = page.getByTestId("huddle-agent-voice-menu-trigger");
  await expect(voiceMenus).toHaveCount(2);
  await expect
    .poll(async () => {
      const state = (await page.evaluate(() =>
        window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("get_huddle_state"),
      )) as {
        agent_voice_settings: Record<
          string,
          { enabled: boolean; voice_key: string }
        >;
      };
      return state.agent_voice_settings;
    })
    .toMatchObject({
      [TEST_IDENTITIES.alice.pubkey]: {
        enabled: true,
        voice_key: "pocket:vera",
      },
    });
  const assignedVoices = await page.evaluate(async () => {
    const state = (await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
      "get_huddle_state",
    )) as {
      agent_voice_settings: Record<string, { voice_key: string }>;
    };
    return Object.values(state.agent_voice_settings).map(
      (settings) => settings.voice_key,
    );
  });
  expect(new Set(assignedVoices).size).toBe(2);

  await page.getByRole("button", { name: "Voice settings for alice" }).click();
  await waitForAnimations(page);
  const voiceMenu = page.locator(
    '[data-testid="huddle-agent-voice-menu-content"][data-state="open"]',
  );
  await expect(voiceMenu).toBeVisible();
  await expect(
    voiceMenu.getByText("Agent text-to-speech", { exact: true }),
  ).toBeVisible();
  const ttsToggle = voiceMenu.getByTestId("huddle-agent-tts-toggle");
  await expect(ttsToggle).toBeChecked();
  await expect(
    voiceMenu.getByTestId("huddle-agent-voice-selector"),
  ).toContainText("Vera");

  await ttsToggle.click();
  await expect(
    voiceMenu.getByTestId("huddle-agent-voice-selector"),
  ).toHaveCount(0);
  await ttsToggle.click();
  await expect(ttsToggle).toBeChecked();
  const voiceSelector = voiceMenu.getByTestId("huddle-agent-voice-selector");
  await expect(voiceSelector).toBeEnabled();
  await voiceSelector.click();
  await page.getByRole("menuitemradio", { name: "Jane" }).click();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const updates = (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
          (entry) => entry.command === "set_huddle_agent_voice",
        );
        return updates.at(-1)?.payload ?? null;
      }),
    )
    .toMatchObject({
      agentPubkey: TEST_IDENTITIES.alice.pubkey,
      voiceKey: "pocket:jane",
    });
});

test("adds channel-mentioned agents to the live huddle roster", async ({
  page,
}) => {
  await installMockBridge(page, {
    windowLabel: `huddle-${HUDDLE_CHANNEL_ID}`,
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [
        { pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" },
        { pubkey: TEST_IDENTITIES.alice.pubkey, role: "bot" },
      ],
    },
  });
  await page.goto("/");

  const participantTiles = page.getByTestId("huddle-participant-tile");
  await expect(participantTiles).toHaveCount(2);
  const result = await page.evaluate(
    async ({ channelId, pubkey }) =>
      window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
        "sync_agents_to_active_huddle",
        {
          channelId,
          agentPubkeys: [pubkey],
        },
      ),
    {
      channelId: HUDDLE_PARENT_ID,
      pubkey: TEST_IDENTITIES.bob.pubkey,
    },
  );

  expect(result).toEqual({
    matched_active_huddle: true,
    added: [TEST_IDENTITIES.bob.pubkey],
  });
  await expect(participantTiles).toHaveCount(3);
  await expect(
    page.getByRole("button", { name: "Voice settings for Bob" }),
  ).toBeVisible();
});

test("does not enroll available agents when sending an ordinary message", async ({
  page,
}) => {
  await installMockBridge(page, {
    windowLabel: `huddle-${HUDDLE_CHANNEL_ID}`,
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
    },
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.alice.pubkey,
        name: "alice",
        status: "running",
      },
      {
        pubkey: TEST_IDENTITIES.bob.pubkey,
        name: "bob",
        status: "running",
      },
    ],
  });
  await page.goto("/");

  const participantTiles = page.getByTestId("huddle-participant-tile");
  await expect(participantTiles).toHaveCount(1);
  await page.getByTestId("message-input").fill("A message for the room");
  await page.getByTestId("send-message").click();

  await expect(
    page
      .getByTestId("message-row")
      .filter({ hasText: "A message for the room" }),
  ).toBeVisible();
  await expect(participantTiles).toHaveCount(1);
  expect(
    await page.evaluate(
      () =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
          (entry) => entry.command === "sync_agents_to_active_huddle",
        ).length,
    ),
  ).toBe(0);
});

test("keeps the colored startup surface while huddle controls connect", async ({
  page,
}) => {
  await installMockBridge(page, {
    windowLabel: `huddle-${HUDDLE_CHANNEL_ID}`,
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      phase: "connected",
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
    },
  });
  await page.goto("/");

  await expect(page.getByTestId("huddle-starting-view")).toBeVisible();
  await expect(
    page.getByRole("status", { name: "Starting huddle" }),
  ).toBeVisible();
  await expect(
    page.getByTestId("huddle-starting-view").locator(".bee-sprite"),
  ).toBeVisible();
  await expect(page.getByTestId("setup-grainient-background")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Leave huddle" }),
  ).toBeVisible();
  await expect(page.getByTestId("message-thread-loading")).toHaveCount(0);
  await expect(page.getByTestId("huddle-participant-strip")).toHaveCount(0);
  await expect(page.getByTestId("huddle-transcript-intro")).toHaveCount(0);
});

test("returns the companion transcript to the same channel in the main app", async ({
  page,
}) => {
  await installMockBridge(page, {
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      huddleThreadEventId: HUDDLE_THREAD_ROOT_ID,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
      transcriptionEnabled: true,
    },
  });
  await page.goto("/");

  await expect(page.locator(".buzz-huddle-shell")).toHaveAttribute(
    "data-huddle-open",
    "true",
  );
  await page.evaluate(() => {
    const threadRoot = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "huddle",
      content: "Transcript written while the companion is open",
    });
    if (!threadRoot) throw new Error("Failed to create Huddle thread root.");
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "huddle",
      content: "Agent reply preserved across the Huddle handoff",
      parentEventId: threadRoot.id,
      pubkey: "deadbeef".repeat(8),
    });
  });

  await page
    .getByRole("button", { name: "Open huddle in a new window" })
    .click();
  await expect(page.locator(".buzz-huddle-shell")).toHaveAttribute(
    "data-huddle-open",
    "false",
  );
  await page.evaluate(async () => {
    await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("close_huddle_companion");
  });

  await expect(page).toHaveURL(
    new RegExp(`/channels/${HUDDLE_CHANNEL_ID.replaceAll("-", "\\-")}`),
  );
  await expect(
    page
      .getByTestId("message-row")
      .filter({ hasText: "Transcript written while the companion is open" }),
  ).toBeVisible();
  await expect(
    page
      .getByTestId("message-row")
      .filter({ hasText: "Agent reply preserved across the Huddle handoff" }),
  ).toBeVisible();
  await expect(page.getByTestId("message-thread-panel")).toHaveCount(0);

  await page
    .getByRole("button", { name: "Open huddle in a new window" })
    .click();
  await expect(page).toHaveURL(
    new RegExp(`/channels/${HUDDLE_PARENT_ID.replaceAll("-", "\\-")}`),
  );
  await expect(
    page.locator(`[data-channel-id="${HUDDLE_CHANNEL_ID}"]`),
  ).toHaveCount(0);
});

test("returns to the parent channel when leaving a huddle channel in view", async ({
  page,
}) => {
  await installMockBridge(page, {
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
    },
  });
  await page.goto("/");

  await expect(page.locator(".buzz-huddle-shell")).toHaveAttribute(
    "data-huddle-open",
    "true",
  );
  await page.evaluate(async () => {
    await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("close_huddle_companion");
  });
  await expect(page).toHaveURL(
    new RegExp(`/channels/${HUDDLE_CHANNEL_ID.replaceAll("-", "\\-")}`),
  );

  await page.getByRole("button", { name: "Leave huddle" }).click();

  await expect(page).toHaveURL(
    new RegExp(`/channels/${HUDDLE_PARENT_ID.replaceAll("-", "\\-")}`),
  );
  await expect(
    page.locator(`[data-channel-id="${HUDDLE_CHANNEL_ID}"]`),
  ).toHaveCount(0);
});

test("keeps the huddle avatar strip compact and exposes the full roster", async ({
  page,
}) => {
  const members = Array.from({ length: 11 }, (_, index) => ({
    pubkey: `${(index + 1).toString(16).padStart(2, "0")}${"a".repeat(62)}`,
    role: index === 10 ? ("bot" as const) : ("member" as const),
  }));

  await installMockBridge(page, {
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members,
    },
  });
  await page.goto("/");

  const participantStrip = page.getByTestId("huddle-participant-strip");
  const participantTrigger = page.getByRole("button", {
    name: "Show all huddle participants (11)",
  });
  await expect(participantTrigger).toBeVisible();
  await expect(
    participantStrip.getByTestId("huddle-participant-avatar"),
  ).toHaveCount(9);
  await expect(participantTrigger).toContainText("+2");
  await expect(page.getByTestId("profile-huddle-control")).toHaveCount(0);

  await participantTrigger.click();
  await expect(
    page.getByRole("heading", { name: "Participants" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Remove Agent .* from huddle/ }),
  ).toHaveCount(1);
});

test("removes an agent from its menu without showing an extra participant control", async ({
  page,
}) => {
  await installMockBridge(page, {
    windowLabel: `huddle-${HUDDLE_CHANNEL_ID}`,
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [
        { pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" },
        { pubkey: TEST_IDENTITIES.alice.pubkey, role: "bot" },
      ],
    },
  });
  await page.goto("/");

  await expect(
    page.getByRole("button", { name: "Manage huddle participants" }),
  ).toHaveCount(0);

  await page.getByTestId("huddle-agent-voice-menu-trigger").click();
  const removeButton = page.getByRole("button", {
    name: "Remove alice from huddle",
  });
  await expect(removeButton).toBeVisible();
  page.once("dialog", (dialog) => dialog.accept());
  await removeButton.click({ force: true });

  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).some(
          (entry) => entry.command === "remove_agent_from_huddle",
        ),
      ),
    )
    .toBe(true);
  await expect(page.getByTestId("huddle-agent-voice-menu-trigger")).toHaveCount(
    0,
  );
});

test("keeps a newer huddle event over a delayed hydration snapshot", async ({
  page,
}) => {
  await installMockBridge(page, {
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [
        { pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" },
        { pubkey: TEST_IDENTITIES.alice.pubkey, role: "bot" },
      ],
      transcriptionEnabled: true,
    },
    huddleStateReadDelayMs: 250,
  });
  await page.goto("/");
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).some(
          (entry) => entry.command === "get_huddle_state",
        ),
      ),
    )
    .toBe(true);

  await setHuddleSnapshot(
    page,
    [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
    false,
  );

  await expect(
    page
      .getByTestId("huddle-participant-strip")
      .getByTestId("huddle-participant-avatar"),
  ).toHaveCount(1);
  await expect(
    page.getByRole("button", { name: /Show all huddle participants/ }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("button", { name: "Start transcript" }),
  ).toHaveAttribute("aria-pressed", "false");
  await page.waitForTimeout(300);
  await expect(
    page
      .getByTestId("huddle-participant-strip")
      .getByTestId("huddle-participant-avatar"),
  ).toHaveCount(1);
});

test("keeps a starting huddle in the drawer after its companion closes", async ({
  page,
}) => {
  await installFakeHuddleMicrophone(page);
  await installMockBridge(page, {
    openHuddleWindowDelayMs: 500,
    startHuddleReturnDelayMs: 1_500,
  });
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await page.getByTestId("channel-start-huddle-trigger").click();

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
            (entry) => entry.command === "open_huddle_window",
          ).length,
      ),
    )
    .toBe(1);
  const ephemeralChannelId = await page.evaluate(async () => {
    const state = (await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
      "get_huddle_state",
    )) as { ephemeral_channel_id: string };
    return state.ephemeral_channel_id;
  });
  await page.evaluate(async () => {
    await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("close_huddle_companion");
  });

  await expect(page.locator(".buzz-huddle-shell")).toHaveAttribute(
    "data-huddle-open",
    "true",
  );
  await expect(page).toHaveURL(
    new RegExp(`/channels/${ephemeralChannelId.replaceAll("-", "\\-")}`),
  );
  await expect
    .poll(() =>
      page.evaluate(async () => {
        const state = (await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
          "get_huddle_state",
        )) as { phase: string };
        return state.phase;
      }),
    )
    .toBe("active");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
            (entry) => entry.command === "open_huddle_window",
          ).length,
      ),
    )
    .toBe(1);
  await expect(page.locator(".buzz-huddle-shell")).toHaveAttribute(
    "data-huddle-open",
    "true",
  );
  await expect(page.getByTestId("profile-huddle-control")).toHaveCount(0);
  await expect(
    page.locator(`[data-channel-id="${ephemeralChannelId}"]`),
  ).toBeVisible();
});

test("starts unmuted with Push to Talk while preserving manual microphone control", async ({
  page,
}) => {
  await installFakeHuddleMicrophone(page);
  await installMockBridge(page, {
    openHuddleWindowDelayMs: 10_000,
    startHuddleReturnDelayMs: 1_500,
  });
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await page.getByTestId("channel-start-huddle-trigger").click();

  const muteButton = page.getByRole("button", { name: "Mute microphone" });
  await expect(muteButton).toBeVisible();
  await expect(muteButton).not.toHaveClass(/bg-destructive\/15/);
  await expect(
    page.getByRole("button", { name: "Audio settings" }),
  ).not.toHaveClass(/bg-destructive\/15/);

  await muteButton.click();
  const unmuteButton = page.getByRole("button", {
    name: "Unmute microphone",
  });
  await expect(unmuteButton).toBeVisible();
  await expect(unmuteButton).toHaveClass(/bg-destructive\/15/);
  await expect(unmuteButton).toHaveClass(/text-destructive/);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).findLast(
            (entry) => entry.command === "set_huddle_manual_mic_unmuted",
          )?.payload,
      ),
    )
    .toEqual({ enabled: false });
  await page.mouse.move(0, 0);
  await unmuteButton.hover();
  await expect(page.getByRole("tooltip")).toHaveAccessibleName(
    /Click to unmute or hold .*Space/,
  );

  await page.evaluate(async () => {
    await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.("ptt-state", true);
  });
  await expect(muteButton).toBeVisible();
  await muteButton.click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).findLast(
            (entry) => entry.command === "set_huddle_manual_mic_unmuted",
          )?.payload,
      ),
    )
    .toEqual({ enabled: false });

  await page.evaluate(async () => {
    await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.("ptt-state", false);
  });
  await expect(unmuteButton).toBeVisible();

  await page.getByRole("button", { name: "Audio settings" }).click();
  await page.getByRole("button", { name: "Turn off Push to Talk" }).click();
  await page.evaluate(async () => {
    await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.("ptt-state", true);
  });
  await expect(unmuteButton).toBeVisible();
  await page.getByRole("button", { name: "Turn on Push to Talk" }).click();
  await expect(unmuteButton).toBeVisible();
  await page.evaluate(async () => {
    await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.("ptt-state", true);
  });
  await expect(muteButton).toBeVisible();
  await page.evaluate(async () => {
    await window.__BUZZ_E2E_EMIT_TAURI_EVENT__?.("ptt-state", false);
  });
  await expect(unmuteButton).toBeVisible();

  await unmuteButton.click();
  await expect(muteButton).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).findLast(
            (entry) => entry.command === "set_huddle_manual_mic_unmuted",
          )?.payload,
      ),
    )
    .toEqual({ enabled: true });
});

test("starts an agent DM huddle and hides its backing channel after it ends", async ({
  page,
}) => {
  await installFakeHuddleMicrophone(page);
  await installMockBridge(page, {
    openHuddleWindowDelayMs: 500,
    startHuddleReturnDelayMs: 1_500,
  });
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");
  await expect
    .poll(() =>
      page.evaluate(
        (pubkey) =>
          window.__BUZZ_E2E_HAS_MOCK_OWNER_KIND_SUBSCRIPTION__?.({
            ownerPubkey: pubkey,
            kind: 44100,
          }) ?? false,
        "deadbeef".repeat(8),
      ),
    )
    .toBe(true);
  const channelReadsBeforeStart = await page.evaluate(
    () =>
      (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
        (entry) => entry.command === "get_channels",
      ).length,
  );

  const startButton = page.getByTestId("channel-start-huddle-trigger");
  await expect(startButton).toBeEnabled();
  await startButton.click();

  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).find(
            (entry) => entry.command === "start_huddle",
          )?.payload,
      ),
    )
    .toMatchObject({
      memberPubkeys: [TEST_IDENTITIES.alice.pubkey],
      parentChannelId: "f48efb06-0c93-5025-aac9-2e646bb6bfa8",
    });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
            (entry) => entry.command === "open_huddle_window",
          ).length,
      ),
    )
    .toBe(1);
  const startingState = await page.evaluate(async () =>
    window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("get_huddle_state"),
  );
  expect(startingState).toMatchObject({ phase: "creating" });
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
            (entry) => entry.command === "get_channels",
          ).length,
      ),
    )
    .toBeGreaterThan(channelReadsBeforeStart);
  const pendingEphemeralChannelId = await page.evaluate(async () => {
    const state = (await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
      "get_huddle_state",
    )) as { ephemeral_channel_id: string };
    return state.ephemeral_channel_id;
  });
  await expect(
    page.locator(`[data-channel-id="${pendingEphemeralChannelId}"]`),
  ).toHaveCount(0);
  await expect(page.locator(".buzz-huddle-shell")).toHaveAttribute(
    "data-huddle-open",
    "false",
  );
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).filter(
            (entry) => entry.command === "open_huddle_window",
          ).length,
      ),
    )
    .toBe(1);
  await expect(page.getByTestId("profile-huddle-control")).toBeVisible();
  await expect(page.locator(".buzz-huddle-shell")).toHaveAttribute(
    "data-huddle-open",
    "false",
  );

  const ephemeralChannelId = await page.evaluate(async () => {
    const state = (await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.(
      "get_huddle_state",
    )) as { ephemeral_channel_id: string };
    return state.ephemeral_channel_id;
  });
  const huddleSidebarChannel = page.locator(
    `[data-channel-id="${ephemeralChannelId}"]`,
  );
  await expect(huddleSidebarChannel).toHaveCount(0);
  await page.evaluate(async () => {
    await window.__BUZZ_E2E_INVOKE_MOCK_COMMAND__?.("close_huddle_companion");
  });
  await expect(page.locator(".buzz-huddle-shell")).toHaveAttribute(
    "data-huddle-open",
    "true",
  );
  await expect(page.getByTestId("profile-huddle-control")).toHaveCount(0);
  await expect(page).toHaveURL(
    new RegExp(`/channels/${ephemeralChannelId.replaceAll("-", "\\-")}`),
  );
  await expect(huddleSidebarChannel).toBeVisible();

  await page
    .getByRole("button", { name: "Open huddle in a new window" })
    .click();
  await expect(huddleSidebarChannel).toHaveCount(0);
  await expect(page.locator(".buzz-huddle-shell")).toHaveAttribute(
    "data-huddle-open",
    "false",
  );
  await expect(page.getByTestId("profile-huddle-control")).toBeVisible();

  await page.getByRole("button", { name: "Leave huddle" }).click();
  await expect(huddleSidebarChannel).toHaveCount(0);
  await expect(page.locator(".buzz-huddle-shell")).toHaveAttribute(
    "data-huddle-open",
    "false",
  );

  await waitForMockLiveSubscription(page, "alice-tyler");
  const lifecycleCreatedAt = Math.floor(Date.now() / 1000);
  await page.evaluate(
    ({ createdAt, ephemeralChannelId, kind }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "alice-tyler",
        content: JSON.stringify({ ephemeral_channel_id: ephemeralChannelId }),
        createdAt,
        id: "3".repeat(64),
        kind,
      });
    },
    {
      createdAt: lifecycleCreatedAt,
      ephemeralChannelId,
      kind: KIND_HUDDLE_STARTED,
    },
  );
  await expect(page.getByTestId("huddle-attachment")).toBeVisible();
  await waitForMockLiveSubscription(page, "alice-tyler", KIND_HUDDLE_ENDED);
  await page.evaluate(
    ({ createdAt, ephemeralChannelId, kind }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "alice-tyler",
        content: JSON.stringify({ ephemeral_channel_id: ephemeralChannelId }),
        createdAt,
        id: "4".repeat(64),
        kind,
      });
    },
    {
      createdAt: lifecycleCreatedAt + 1,
      ephemeralChannelId,
      kind: KIND_HUDDLE_ENDED,
    },
  );
  const endedHuddle = page
    .getByTestId("huddle-attachment")
    .filter({ hasText: "Huddle · Ended" });
  await expect(endedHuddle).toBeVisible();
  await endedHuddle.getByRole("button", { name: "View huddle" }).click();
  await expect(page).toHaveURL(
    new RegExp(`/channels/${ephemeralChannelId.replaceAll("-", "\\-")}`),
  );
  await expect(huddleSidebarChannel).toBeVisible();
  await expect(page.getByTestId("message-thread-panel")).toHaveCount(0);
});

test("closes add-agent dialog when parent membership is already satisfied", async ({
  page,
}) => {
  await installMockBridge(page, {
    addAgentToHuddleResult: {
      ephemeral_added: true,
      parent_added: true,
      parent_error: null,
    },
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
    },
    managedAgents: [
      {
        pubkey: TEST_IDENTITIES.bob.pubkey,
        name: "bob",
        status: "running",
      },
    ],
  });
  await page.goto("/");

  await page.getByRole("button", { name: "Add agent to huddle" }).click();
  const dialog = page.getByRole("dialog", { name: "Add agents" });
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: /bob/i }).click();

  await expect(dialog).toHaveCount(0);
  await expect(
    page.getByText("Added to huddle, but parent channel failed:"),
  ).toHaveCount(0);
});

test("starts an available stopped agent before adding it to the huddle", async ({
  page,
}) => {
  await installMockBridge(page, {
    addAgentToHuddleResult: {
      ephemeral_added: true,
      parent_added: true,
      parent_error: null,
    },
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
    },
    managedAgents: [
      {
        avatarUrl: "https://example.com/bob.png",
        pubkey: TEST_IDENTITIES.bob.pubkey,
        name: "bob",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");

  await page.getByRole("button", { name: "Add agent to huddle" }).click();
  const dialog = page.getByRole("dialog", { name: "Add agents" });
  await expect(dialog).toBeVisible();
  await expect(dialog).toHaveClass(/data-\[state=open\]:animate-in/);
  await expect(dialog).toHaveClass(/data-\[state=open\]:fade-in-0/);
  await expect(dialog).toHaveClass(/data-\[state=open\]:zoom-in-95/);
  await expect(dialog).toHaveClass(/duration-200/);
  await expect(dialog).toHaveClass(/motion-reduce:animate-none/);
  await expect(
    dialog.getByText("Choose an agent to join this huddle.", { exact: true }),
  ).toBeVisible();
  await expect(dialog.getByText("stopped", { exact: true })).toHaveCount(0);
  await dialog.getByRole("button", { name: /bob/i }).click();

  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
          .filter(
            (entry) =>
              entry.command === "start_managed_agent" ||
              entry.command === "add_agent_to_huddle",
          )
          .map((entry) => entry.command),
      ),
    )
    .toEqual(["start_managed_agent", "add_agent_to_huddle"]);
  await expect(dialog).toHaveCount(0);
});

test("stops an agent started solely for a failed huddle add", async ({
  page,
}) => {
  await installMockBridge(page, {
    addAgentToHuddleError: "membership publish rejected",
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
    },
    managedAgents: [
      {
        avatarUrl: "https://example.com/bob.png",
        pubkey: TEST_IDENTITIES.bob.pubkey,
        name: "bob",
        status: "stopped",
      },
    ],
  });
  await page.goto("/");

  await page.getByRole("button", { name: "Add agent to huddle" }).click();
  const dialog = page.getByRole("dialog", { name: "Add agents" });
  await dialog.getByRole("button", { name: /bob/i }).click();

  await expect(dialog.getByText(/Failed to add agent:/)).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
          .filter((entry) =>
            [
              "start_managed_agent",
              "add_agent_to_huddle",
              "stop_managed_agent",
            ].includes(entry.command),
          )
          .map((entry) => entry.command),
      ),
    )
    .toEqual([
      "start_managed_agent",
      "add_agent_to_huddle",
      "stop_managed_agent",
    ]);
});

test("does not deploy a provider agent when its huddle add fails", async ({
  page,
}) => {
  await installMockBridge(page, {
    addAgentToHuddleError: "membership publish rejected",
    huddle: {
      parentChannelId: HUDDLE_PARENT_ID,
      ephemeralChannelId: HUDDLE_CHANNEL_ID,
      members: [{ pubkey: TEST_IDENTITIES.tyler.pubkey, role: "member" }],
    },
    managedAgents: [
      {
        avatarUrl: "https://example.com/bob.png",
        backend: { type: "provider", id: "mock", config: {} },
        pubkey: TEST_IDENTITIES.bob.pubkey,
        name: "bob",
        status: "not_deployed",
      },
    ],
  });
  await page.goto("/");

  await page.getByRole("button", { name: "Add agent to huddle" }).click();
  const dialog = page.getByRole("dialog", { name: "Add agents" });
  await dialog.getByRole("button", { name: /bob/i }).click();

  await expect(dialog.getByText(/Failed to add agent:/)).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate(() =>
        (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
          .filter((entry) =>
            [
              "start_managed_agent",
              "add_agent_to_huddle",
              "stop_managed_agent",
            ].includes(entry.command),
          )
          .map((entry) => entry.command),
      ),
    )
    .toEqual(["add_agent_to_huddle"]);
});
