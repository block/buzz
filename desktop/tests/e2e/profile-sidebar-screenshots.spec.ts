import { expect, test, type Page } from "@playwright/test";

import {
  createMockAgentMemoryListing,
  installMockBridge,
} from "../helpers/bridge";

// PR #1200 profile-sidebar screenshot capture. Walks every view the polish
// built — summary, instructions, diagnostics/log, memory, activity, owner row,
// settings cog menu — for both an owner-managed agent profile and a read-only
// human profile. Navigation mirrors the verified "renders agent profile
// ingress subviews from the Playwright mock bridge" test in profile.spec.ts.

const SHOTS = "test-results/profile-sidebar";

const LONG_INSTRUCTION = [
  "Watch the channel and help when asked.",
  "Summarize active decisions, call out risks plainly, and keep the tone concise.",
  "Prefer concrete next steps over broad commentary, and cite the relevant thread context when responding.",
  "Avoid catchphrases, theatrical roleplay, and unsupported guesses.",
  "When uncertainty remains, say exactly what evidence would resolve it.",
].join("\n\n");

// Real avatar imagery for PR-ready shots (tho's directive): seed avatars from
// picsum.photos with a distinct seed string per user so the shots show actual
// portraits instead of initials, and so multiple users are visibly different.
const avatarUrlForSeed = (seed: string) =>
  `https://picsum.photos/seed/${seed}/200`;

// Owner pubkey is the mock viewer ("npub1mock... (you)"); Alice is the seeded
// human/read-only fixture. The managed agent's avatar is seeded at create time
// (see addGenericAgent) since its pubkey is generated at runtime.
const MOCK_OWNER_PUBKEY = "deadbeef".repeat(8);
const ALICE_PUBKEY =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";

const AVATAR_PROFILES = [
  {
    pubkey: MOCK_OWNER_PUBKEY,
    displayName: "npub1mock...",
    avatarUrl: avatarUrlForSeed("boom-owner"),
  },
  {
    pubkey: ALICE_PUBKEY,
    displayName: "alice",
    avatarUrl: avatarUrlForSeed("boom-alice"),
  },
];

async function addGenericAgent(
  page: Page,
  channelName: string,
  agentName: string,
  systemPrompt: string,
  avatarUrl?: string,
): Promise<string> {
  await page.getByTestId(`channel-${channelName}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channelName);
  const channelId = await page
    .getByTestId(`channel-${channelName}`)
    .getAttribute("data-channel-id");
  if (!channelId) {
    throw new Error(`Channel ${channelName} is missing a data-channel-id.`);
  }
  await page.waitForFunction(() => {
    return Boolean(
      (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__,
    );
  });
  return page.evaluate(
    async ({ agentName, avatarUrl, channelId, systemPrompt }) => {
      const invoke = (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
            command: string,
            payload?: Record<string, unknown>,
          ) => Promise<{ agent?: { pubkey: string } }>;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      if (!invoke) {
        throw new Error("Mock bridge is not installed.");
      }
      const created = await invoke("create_managed_agent", {
        input: {
          name: agentName,
          spawnAfterCreate: true,
          systemPrompt,
          avatarUrl,
        },
      });
      const pubkey = created.agent?.pubkey;
      if (!pubkey) {
        throw new Error("create_managed_agent returned no pubkey.");
      }

      await invoke("add_channel_members", {
        channelId,
        pubkeys: [pubkey],
        role: "bot",
      });

      await (
        window as Window & {
          __BUZZ_E2E_QUERY_CLIENT__?: {
            invalidateQueries: () => Promise<void>;
          };
        }
      ).__BUZZ_E2E_QUERY_CLIENT__?.invalidateQueries();

      return pubkey;
    },
    { agentName, avatarUrl, channelId, systemPrompt },
  );
}

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(async () =>
      page.evaluate((channelName) => {
        return (
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName }) ?? false
        );
      }, channelName),
    )
    .toBe(true);
}

async function emitAgentMessage(page: Page, pubkey: string, content: string) {
  await page.evaluate(
    ({ pubkey, content }) => {
      const emit = (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            pubkey: string;
          }) => unknown;
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) {
        throw new Error("Mock message emitter is unavailable.");
      }
      emit({ channelName: "general", content, pubkey });
    },
    { pubkey, content },
  );
}

test.describe("profile sidebar — PR #1200 screenshots", () => {
  test.use({ viewport: { width: 1280, height: 820 } });

  test("agent (owner) profile — every subview", async ({ page }) => {
    await installMockBridge(page, {
      agentMemory: createMockAgentMemoryListing(),
      searchProfiles: AVATAR_PROFILES,
    });
    await page.goto("/");

    const agentPubkey = await addGenericAgent(
      page,
      "general",
      "Memory Bot",
      LONG_INSTRUCTION,
      avatarUrlForSeed("boom"),
    );

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await waitForMockLiveSubscription(page, "general");
    await emitAgentMessage(page, agentPubkey, "Memory bot check-in");

    const messageRow = page
      .getByTestId("message-row")
      .filter({ hasText: "Memory bot check-in" });
    await expect(messageRow).toBeVisible();
    await messageRow.locator("button").first().click();

    const panel = page.getByTestId("user-profile-panel");
    await expect(panel).toBeVisible();

    // Wait for the seeded picsum avatar to actually load before the summary
    // shots — Radix only mounts the <img> once it resolves, so this guards
    // against capturing the initials fallback mid-load.
    await expect(page.getByTestId("user-profile-avatar-image")).toBeVisible();

    // 01 — Summary view (default "Profile"). The Info tab is the default; the
    // owner row, public key, and activity-log ingress all live here inline.
    await expect(page.getByTestId("user-profile-tab-info")).toBeVisible();
    await expect(
      page.getByRole("heading", { level: 2, name: "Profile" }),
    ).toBeVisible();
    await panel.screenshot({ path: `${SHOTS}/01-agent-summary.png` });

    // The Runtime tab only mounts once the spawned agent has settled into a
    // running state (showRuntimeTab depends on managedAgent/runtime fields).
    // Wait for the runtime status to flip before driving the runtime views —
    // mirrors profile.spec.ts L707, which is the proven navigation.
    await expect(
      page.getByTestId("user-profile-runtime-status"),
    ).toHaveAttribute("data-status", "running");

    // 02 — Runtime tab: model + respond-to (owner row lives here)
    await page.getByTestId("user-profile-tab-runtime").click();
    await expect(page.getByTestId("user-profile-model")).toBeVisible();
    await expect(page.getByTestId("user-profile-respond-to")).toBeVisible();
    await panel.screenshot({ path: `${SHOTS}/02-agent-runtime-owner-row.png` });

    // 03 — Settings cog menu (duplicate / export / autostart / delete)
    await page.getByTestId("user-profile-settings-menu-trigger").click();
    await expect(
      page.getByTestId(`user-profile-agent-auto-start-${agentPubkey}`),
    ).toBeVisible();
    await page.screenshot({ path: `${SHOTS}/03-agent-settings-menu.png` });
    await page.keyboard.press("Escape");

    // 04 — Instructions focused view (opened from the runtime tab)
    await expect(
      page.getByTestId("user-profile-agent-instruction"),
    ).toContainText("Watch the channel and help when asked.");
    await page.getByTestId("user-profile-agent-instruction-row").click();
    await expect(
      page.getByRole("heading", { level: 2, name: "Instructions" }),
    ).toBeVisible();
    await expect(
      page.getByTestId("user-profile-agent-instructions-view"),
    ).toContainText("When uncertainty remains");
    await panel.screenshot({ path: `${SHOTS}/04-agent-instructions.png` });
    await page.getByTestId("user-profile-panel-back").click();

    // 05 — Diagnostics / Harness Log (opened from the runtime tab)
    await page.getByTestId("user-profile-tab-runtime").click();
    await page.getByTestId("user-profile-diagnostics-ingress").click();
    await expect(
      page.getByRole("heading", { level: 2, name: "Harness Log" }),
    ).toBeVisible();
    await expect(page.getByTestId("managed-agent-log-content")).toBeVisible();
    await panel.screenshot({ path: `${SHOTS}/05-agent-diagnostics-log.png` });
    await page.getByTestId("user-profile-panel-back").click();

    // 06 — Activity view (agent session thread, opened from the Info tab).
    // Opening the activity ingress swaps the profile panel out for the agent
    // session thread surface, so screenshot that panel directly (mirrors
    // profile.spec.ts L760-763, where user-profile-panel is gone here).
    await page.getByTestId("user-profile-tab-info").click();
    await page.getByTestId(`user-profile-view-activity-${agentPubkey}`).click();
    const sessionThread = page.getByTestId("agent-session-thread-panel");
    await expect(sessionThread).toBeVisible();
    await sessionThread.screenshot({ path: `${SHOTS}/06-agent-activity.png` });
    await page.getByTestId("agent-session-back").click();
    await expect(panel).toBeVisible();

    // 07 — Channels tab
    await page.getByTestId("user-profile-tab-channels").click();
    await expect(page.getByTestId("user-profile-channels-list")).toContainText(
      "#general",
    );
    await panel.screenshot({ path: `${SHOTS}/07-agent-channels.png` });

    // 08 — Memory view
    await page.getByTestId("user-profile-tab-memories").click();
    await expect(page.getByTestId("agent-memory-section")).toBeVisible();
    await expect(page.getByTestId("agent-memory-list")).toContainText(
      "ui-density",
    );
    await panel.screenshot({ path: `${SHOTS}/08-agent-memory.png` });

    // 08b — Memory view expanded (View all)
    const truncated = page.getByTestId("agent-memory-truncated");
    if (await truncated.isVisible().catch(() => false)) {
      await truncated.click();
      await expect(page.getByTestId("agent-memory-list")).toContainText(
        "orphan",
      );
      await panel.screenshot({
        path: `${SHOTS}/08b-agent-memory-expanded.png`,
      });
    }
  });

  test("human (read-only, non-owner) profile", async ({ page }) => {
    await installMockBridge(page, { searchProfiles: AVATAR_PROFILES });
    await page.goto("/");

    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");

    // Alice authored "Hey team — checking in." — a non-self human profile.
    const aliceRow = page
      .getByTestId("message-row")
      .filter({ hasText: "Hey team — checking in." });
    await expect(aliceRow).toBeVisible();
    await aliceRow.locator("button").first().click();

    const panel = page.getByTestId("user-profile-panel");
    await expect(panel).toBeVisible();
    // Wait for Alice's seeded picsum avatar to load before capturing.
    await expect(page.getByTestId("user-profile-avatar-image")).toBeVisible();
    await panel.screenshot({ path: `${SHOTS}/09-human-readonly-summary.png` });
  });
});
