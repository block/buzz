import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey:
          "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        name: "Red Agent",
        status: "stopped",
      },
      {
        avatarUrl:
          "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'%3E%3Crect width='64' height='64' fill='%230ea5e9'/%3E%3Ccircle cx='32' cy='28' r='15' fill='white'/%3E%3C/svg%3E",
        pubkey:
          "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        name: "Blue Agent",
        status: "stopped",
      },
    ],
  });
});

test("runs, pauses, and replays a deterministic two-agent match", async ({
  page,
}) => {
  await page.goto("/#/?lab=artillery");

  const arena = page.getByTestId("artillery-arena");
  await expect(
    page.getByRole("heading", { name: "Buzz Artillery" }),
  ).toBeVisible();
  const liveSetup = page.getByTestId("live-match-setup");
  await expect(liveSetup).toBeVisible();
  const agentSelect = liveSetup.getByRole("combobox", {
    name: "Red agent",
    exact: true,
  });
  await expect(agentSelect).toHaveValue(
    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  );
  await expect(agentSelect.locator("option")).toHaveText([
    "Red Agent · stopped",
    "Blue Agent · stopped",
  ]);
  await expect(
    liveSetup.getByRole("combobox", { name: "Blue agent", exact: true }),
  ).toHaveValue(
    "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
  );
  await expect(
    liveSetup.getByRole("combobox", { name: "Turn timer", exact: true }),
  ).toHaveValue("5");
  await expect(page.getByTestId("start-live-artillery-match")).toBeEnabled();
  await expect(page.getByTestId("artillery-sound-toggle")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await page.getByTestId("artillery-sound-toggle").click();
  await expect(page.getByTestId("artillery-sound-toggle")).toContainText(
    "Sound off",
  );
  await page.getByTestId("artillery-sound-toggle").click();
  await expect(page.getByTestId("publish-artillery-result")).toHaveCount(0);
  await expect(arena.locator("canvas")).toBeVisible({ timeout: 15_000 });
  await expect(arena).toHaveAttribute("data-animation-run", "1");
  await expect(arena).toHaveAttribute("data-match-turn-count", "5");
  await expect(arena).toHaveAttribute("data-animation-phase", "firing");
  await expect(arena).toHaveAttribute("data-last-sound-cue", "launch");
  await expect(arena).toHaveAttribute("data-projectile-whistle", "playing");
  await expect(arena).toHaveAttribute("data-animation-phase", "impact", {
    timeout: 5_000,
  });
  await expect(arena).toHaveAttribute("data-last-sound-cue", "impact");
  await expect(arena).toHaveAttribute("data-projectile-whistle", "stopped");
  const firstBlueFort = Number(
    await arena.getAttribute("data-blue-structure-integrity"),
  );
  expect(firstBlueFort).toBeGreaterThan(0);
  expect(firstBlueFort).toBeLessThan(100);

  await page.getByTestId("artillery-pause").click();
  await expect(arena).toHaveAttribute("data-match-status", "paused");
  await page.getByTestId("artillery-pause").click();
  await expect(arena).toHaveAttribute("data-match-status", "playing");
  await expect(arena).toHaveAttribute("data-match-status", "complete", {
    timeout: 20_000,
  });
  await expect(arena).toHaveAttribute("data-match-turn", "5");
  await expect(arena).toHaveAttribute("data-match-winner", "red");
  await expect(arena).toHaveAttribute("data-last-sound-cue", "victory");
  await expect(arena).toHaveAttribute("data-sound-cue-count", "11");
  await expect(arena).toHaveAttribute("data-blue-structure-integrity", "0");
  await expect(page.getByTestId("artillery-result")).toContainText(
    "Bumble wins!",
  );
  await expect(page.getByTestId("artillery-result")).toContainText("Victory");
  await expect(page.getByTestId("artillery-delete-loser")).toContainText(
    "Delete the loser 💀",
  );
  await expect(page.getByTestId("artillery-delete-loser")).toContainText(
    "(Fizz)",
  );
  await expect(page.getByTestId("artillery-delete-loser")).toBeDisabled();
  await expect(
    page
      .getByTestId("artillery-transcript")
      .locator('[data-resolution="invalid-fallback"]'),
  ).toHaveCount(1);

  await page.getByTestId("artillery-replay").click();
  await expect(page.getByTestId("artillery-result")).toHaveCount(0);
  await expect(arena).toHaveAttribute("data-animation-run", "2");
  await expect(arena).toHaveAttribute("data-animation-phase", "firing");
  await expect(arena).toHaveAttribute("data-blue-structure-integrity", "100");
  await expect(arena).toHaveAttribute("data-match-status", "complete", {
    timeout: 20_000,
  });

  const canvasSize = await arena.locator("canvas").evaluate((canvas) => ({
    height: (canvas as HTMLCanvasElement).height,
    width: (canvas as HTMLCanvasElement).width,
  }));
  expect(canvasSize).toEqual({ height: 540, width: 960 });
});

test("streams two managed agents and preserves the match across navigation", async ({
  page,
}) => {
  await page.goto("/#/?lab=artillery");
  await page.evaluate(() => {
    const e2eWindow = window as typeof window & {
      __BUZZ_ARTILLERY_RESPONDER__?: number;
      __BUZZ_E2E_COMMAND_LOG__?: Array<{
        command: string;
        payload: {
          content?: string;
          mentionPubkeys?: string[];
          parentEventId?: string | null;
        };
      }>;
      __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
        channelName: string;
        content: string;
        kind: number;
        pubkey: string;
      }) => unknown;
    };
    const seen = new Set<string>();
    e2eWindow.__BUZZ_ARTILLERY_RESPONDER__ = window.setInterval(() => {
      for (const entry of e2eWindow.__BUZZ_E2E_COMMAND_LOG__ ?? []) {
        const content = entry.payload.content ?? "";
        if (
          entry.command !== "send_channel_message" ||
          !content.includes("Buzz Artillery turn") ||
          seen.has(content)
        ) {
          continue;
        }
        seen.add(content);
        const requestId = content.match(/request ([^\n]+)/)?.[1];
        const pubkey = entry.payload.mentionPubkeys?.[0];
        if (!requestId || !pubkey) continue;
        window.setTimeout(() => {
          e2eWindow.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
            channelName: "agents",
            content: JSON.stringify({
              requestId,
              angle: 45,
              power: 72,
              weapon: "pulse-shell",
            }),
            kind: 9,
            pubkey,
          });
        }, 220);
      }
    }, 25);
  });

  await page.getByTestId("start-live-artillery-match").click();
  await expect(page.getByTestId("live-turn-wait")).toContainText(
    "waiting for Red Agent",
  );
  const arena = page.getByTestId("artillery-arena");
  await expect(arena).toHaveAttribute("data-match-turn-count", /[1-5]/, {
    timeout: 10_000,
  });
  await expect(arena).toHaveAttribute("data-animation-phase", "firing");

  await page.getByText("agents", { exact: true }).first().click();
  await expect(page.getByRole("heading", { name: "agents" })).toBeVisible();
  await page.evaluate(() => {
    window.location.hash = "/?lab=artillery";
  });

  await expect(page.getByTestId("live-match-setup")).toHaveAttribute(
    "data-live-match-status",
    "complete",
    { timeout: 10_000 },
  );
  await expect(page.getByTestId("publish-artillery-result")).toBeVisible();
  await expect(page.getByTestId("artillery-result")).toContainText(
    "Red Agent wins",
    { timeout: 20_000 },
  );
  await expect(arena).toHaveAttribute("data-match-turn", "5");
  await expect(arena).toHaveAttribute("data-blue-structure-integrity", "0");

  const deleteLoser = page.getByTestId("artillery-delete-loser");
  await expect(deleteLoser).toBeEnabled();
  await expect(deleteLoser).toContainText("Delete the loser 💀");
  await expect(deleteLoser).toContainText("(Blue Agent)");
  await deleteLoser.click();
  await expect(page.getByTestId("artillery-delete-loser-dialog")).toContainText(
    "Delete Blue Agent?",
  );
  await page.getByTestId("artillery-delete-loser-confirm").click();
  await expect(page.getByTestId("artillery-ravine-cinematic")).toBeVisible();
  await expect(arena).toHaveAttribute("data-ravine-cinematic", "playing");
  await expect(page.getByTestId("skip-artillery-ravine")).toBeVisible();
  await page.getByTestId("skip-artillery-ravine").click();
  await expect(page.getByTestId("artillery-ravine-cinematic")).toHaveCount(0);
  await expect(arena).toHaveAttribute("data-ravine-cinematic", "complete");
  await expect(deleteLoser).toContainText("Loser deleted 💀");
  await expect(deleteLoser).toContainText("(Blue Agent)");
  await expect(deleteLoser).toBeDisabled();
  const remainingAgentOptions = page
    .getByTestId("live-match-setup")
    .getByRole("combobox", { name: "Red agent", exact: true })
    .locator("option");
  await expect(remainingAgentOptions).toHaveCount(1);
  await expect(remainingAgentOptions).toContainText("Red Agent");
  await expect(remainingAgentOptions).not.toContainText("Blue Agent");

  const artilleryMessages = await page.evaluate(() => {
    const entries = (
      window as typeof window & {
        __BUZZ_E2E_COMMAND_LOG__?: Array<{
          command: string;
          payload: { content?: string; parentEventId?: string | null };
        }>;
      }
    ).__BUZZ_E2E_COMMAND_LOG__;
    return (entries ?? [])
      .filter(
        (entry) =>
          entry.command === "send_channel_message" &&
          entry.payload.content?.includes("Buzz Artillery"),
      )
      .map((entry) => entry.payload);
  });
  expect(artilleryMessages[0].content).toContain(
    "live · Red Agent vs Blue Agent",
  );
  expect(artilleryMessages[0].content).toContain("5s per turn");
  expect(artilleryMessages.slice(1)).toHaveLength(5);
  expect(
    artilleryMessages.slice(1).every((message) => message.parentEventId),
  ).toBe(true);

  await page.getByText("agents", { exact: true }).first().click();
  await expect(page.getByTestId("artillery-match-attachment")).toBeVisible();
  await page.getByTestId("watch-artillery-match").click();
  await expect(page.getByTestId("durable-match-status")).toHaveAttribute(
    "data-watch-status",
    "complete",
  );
  await expect(page.getByTestId("artillery-arena")).toHaveAttribute(
    "data-match-turn-count",
    "5",
  );

  await page.reload();
  await expect(page.getByTestId("durable-match-status")).toHaveAttribute(
    "data-watch-status",
    "complete",
  );
  await expect(page.getByTestId("artillery-arena")).toHaveAttribute(
    "data-match-turn-count",
    "5",
  );
});

test("takes over an expired referee lease and resumes the interrupted turn", async ({
  page,
}) => {
  await page.addInitScript(() => {
    const testWindow = window as typeof window & {
      __BUZZ_ARTILLERY_FAILOVER_RESPONDER__?: number;
      __BUZZ_E2E_ARTILLERY_LEASE_MS__?: number;
      __BUZZ_E2E_COMMAND_LOG__?: Array<{
        command: string;
        payload: { content?: string; mentionPubkeys?: string[] };
      }>;
      __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
        channelName: string;
        content: string;
        kind: number;
        pubkey: string;
      }) => unknown;
    };
    testWindow.__BUZZ_E2E_ARTILLERY_LEASE_MS__ = 1_000;
    const seen = new Set<string>();
    testWindow.__BUZZ_ARTILLERY_FAILOVER_RESPONDER__ = window.setInterval(
      () => {
        for (const entry of testWindow.__BUZZ_E2E_COMMAND_LOG__ ?? []) {
          const content = entry.payload.content ?? "";
          if (
            entry.command !== "send_channel_message" ||
            !content.includes("Buzz Artillery turn") ||
            seen.has(content)
          ) {
            continue;
          }
          const turn = Number(content.match(/Buzz Artillery turn (\d+)/)?.[1]);
          if (
            turn > 1 &&
            window.sessionStorage.getItem("buzz-artillery-failover-ready") !==
              "yes"
          ) {
            continue;
          }
          seen.add(content);
          const requestId = content.match(/request ([^\n]+)/)?.[1];
          const pubkey = entry.payload.mentionPubkeys?.[0];
          if (!requestId || !pubkey) continue;
          window.setTimeout(() => {
            testWindow.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
              channelName: "agents",
              content: JSON.stringify({
                requestId,
                angle: 45,
                power: 72,
                weapon: "pulse-shell",
              }),
              kind: 9,
              pubkey,
            });
          }, 100);
        }
      },
      25,
    );
  });

  await page.goto("/#/?lab=artillery");
  await page.getByTestId("start-live-artillery-match").click();
  await expect(page.getByTestId("artillery-arena")).toHaveAttribute(
    "data-match-turn-count",
    "1",
    { timeout: 10_000 },
  );

  await page.getByText("agents", { exact: true }).first().click();
  await expect(page.getByTestId("artillery-match-attachment")).toBeVisible();
  await page.getByTestId("watch-artillery-match").click();
  await expect(page.getByTestId("durable-match-status")).toHaveAttribute(
    "data-watch-status",
    "watching",
  );

  await page.evaluate(() => {
    window.sessionStorage.setItem("buzz-artillery-failover-ready", "yes");
  });
  await page.reload();

  await expect(page.getByTestId("durable-match-status")).toHaveAttribute(
    "data-watch-status",
    "complete",
    { timeout: 20_000 },
  );
  await expect(page.getByTestId("artillery-arena")).toHaveAttribute(
    "data-match-turn-count",
    "5",
  );
  const resumedCommands = await page.evaluate(() =>
    (
      window as typeof window & {
        __BUZZ_E2E_COMMAND_LOG__?: Array<{
          command: string;
          payload: { content?: string };
        }>;
      }
    ).__BUZZ_E2E_COMMAND_LOG__?.filter(
      (entry) => entry.command === "send_channel_message",
    ),
  );
  expect(
    resumedCommands?.some((entry) =>
      entry.payload.content?.includes("Referee lease claimed · term 2"),
    ),
  ).toBe(true);
  expect(
    resumedCommands?.some((entry) =>
      entry.payload.content?.includes("Buzz Artillery turn 2"),
    ),
  ).toBe(true);
});

test("honors reduced motion while preserving the final game state", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/#/?lab=artillery");

  const arena = page.getByTestId("artillery-arena");
  await expect(arena.locator("canvas")).toBeVisible({ timeout: 15_000 });
  await expect(arena).toHaveAttribute("data-animation-run", "1");
  await expect(arena).toHaveAttribute("data-match-status", "complete");
  await expect(arena).toHaveAttribute("data-match-turn", "5");
  await expect(arena).toHaveAttribute("data-match-winner", "red");
  await expect(page.getByTestId("artillery-live-status")).toContainText(
    "Bumble wins",
  );
  await expect(arena).toHaveAttribute("data-last-sound-cue", "victory");
  await expect(arena).toHaveAttribute("data-blue-structure-integrity", "0");
});

test("supports an explicit mid-match forfeit", async ({ page }) => {
  await page.goto("/#/?lab=artillery");
  const arena = page.getByTestId("artillery-arena");
  await expect(arena).toHaveAttribute("data-animation-phase", "firing", {
    timeout: 15_000,
  });

  await page.getByTestId("artillery-forfeit").click();
  await expect(arena).toHaveAttribute("data-match-status", "forfeited");
  await expect(page.getByTestId("artillery-result")).toContainText(
    "by forfeit",
  );
  await expect(arena).toHaveAttribute("data-last-sound-cue", "victory");
});
