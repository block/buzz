/**
 * Screenshot spec for the NIP-AD disposition surface (kind:44300).
 *
 * Captures the three badge states on a marked request, the conflict marker,
 * and both summary-chip states (all-resolved vs. needs-attention with the
 * popover open). See docs/nips/NIP-AD.md.
 *
 * Every shot is scoped with `locator.screenshot()` (or a tight `clip` where an
 * overlay must be included) rather than a full-page capture — an unscoped
 * page shot renders the same pixels for several of these states and produces
 * byte-identical PNGs. Verify with `shasum -a 256` before posting.
 */

import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/nip-ad-screenshots";

// The agent every request is addressed to, and that signs every disposition.
// Binding requires the request to name its disposition's signer as an `agent`
// target, so these must agree.
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const REQUESTER_PUBKEY = "deadbeef".repeat(8);
const AGENT_PUBKEY = "b".repeat(64);

const REQ_COMPLETED = "1".repeat(64);
const REQ_REFUSED = "2".repeat(64);
const REQ_ERRORED = "3".repeat(64);
const REQ_CONFLICT = "4".repeat(64);
const REQ_UNANSWERED = "5".repeat(64);

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(async () =>
      page.evaluate(
        (currentChannelName) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: currentChannelName,
          }) ?? false,
        channelName,
      ),
    )
    .toBe(true);
}

async function openGeneral(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");
}

function emitRequest(
  page: import("@playwright/test").Page,
  id: string,
  content: string,
) {
  return page.evaluate(
    ({ id: eventId, content: body, agent }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: body,
        id: eventId,
        extraTags: [
          ["t", "request"],
          ["agent", agent],
          ["p", agent],
        ],
      });
    },
    { id, content, agent: AGENT_PUBKEY },
  );
}

function emitDisposition(
  page: import("@playwright/test").Page,
  requestId: string,
  disposition: "completed" | "refused" | "errored",
  reason: string,
  createdAt?: number,
) {
  return page.evaluate(
    ({
      requestId: e,
      disposition: d,
      reason: r,
      agent,
      createdAt: at,
      channelId,
      requester,
    }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: JSON.stringify({ disposition: d, reason: r }),
        kind: 44300,
        pubkey: agent,
        createdAt: at,
        extraTags: [
          // No ["h", …] here: the mock bridge adds the channel tag itself,
          // and a second one makes the event structurally invalid.
          ["e", e],
          ["p", requester],
          ["disposition", d],
        ],
      });
    },
    {
      requestId,
      disposition,
      reason,
      agent: AGENT_PUBKEY,
      createdAt,
      channelId: GENERAL_CHANNEL_ID,
      requester: REQUESTER_PUBKEY,
    },
  );
}

function requestRow(page: import("@playwright/test").Page, content: string) {
  return page.getByTestId("message-row").filter({ hasText: content }).last();
}

test.describe("NIP-AD disposition screenshots", () => {
  test.use({ viewport: { width: 1280, height: 800 } });

  test.beforeEach(async ({ page }) => {
    await installMockBridge(page);
  });

  test("01 — completed badge on the request it answers", async ({ page }) => {
    await openGeneral(page);
    await emitRequest(
      page,
      REQ_COMPLETED,
      "@agent summarize yesterday's decisions",
    );
    await emitDisposition(page, REQ_COMPLETED, "completed", "");

    const row = requestRow(page, "@agent summarize yesterday's decisions");
    await expect(row.getByText("Completed")).toBeVisible();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/01-completed-badge.png` });
  });

  test("02 — refused badge, signed reason readable in the tooltip", async ({
    page,
  }) => {
    await openGeneral(page);
    await emitRequest(page, REQ_REFUSED, "@agent delete all of Bob's messages");
    await emitDisposition(
      page,
      REQ_REFUSED,
      "refused",
      "outside my delegation — I have no moderation authority in this channel",
    );

    const badge = requestRow(
      page,
      "@agent delete all of Bob's messages",
    ).getByText("Refused");
    await expect(badge).toBeVisible();
    await waitForAnimations(page);
    await badge.hover();
    const tip = page.getByText("outside my delegation", { exact: false });
    await expect(tip).toBeVisible();
    await waitForAnimations(page);
    // The tooltip is an overlay outside the row's own box, so this needs a
    // page-level clip — derived from the badge's actual position rather than
    // hardcoded, which silently captured the wrong region.
    const box = await badge.boundingBox();
    if (!box) {
      throw new Error("refused badge has no bounding box");
    }
    await page.screenshot({
      path: `${SHOTS}/02-refused-tooltip.png`,
      // The tooltip is ~320px wide and renders left of the badge, far enough
      // that it overhangs the message pane — so the clip starts from the
      // window edge rather than the pane's, or the reason text gets cut.
      clip: {
        x: Math.max(0, box.x - 360),
        y: Math.max(0, box.y - 110),
        width: 820,
        height: 220,
      },
    });
  });

  test("03 — errored badge marks an open, repairable request", async ({
    page,
  }) => {
    await openGeneral(page);
    await emitRequest(
      page,
      REQ_ERRORED,
      "@agent read the deploy log and summarize",
    );
    await emitDisposition(
      page,
      REQ_ERRORED,
      "errored",
      "turn ended with an error",
    );

    const row = requestRow(page, "@agent read the deploy log and summarize");
    await expect(row.getByText("Errored")).toBeVisible();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/03-errored-badge.png` });
  });

  test("04 — contradictory dispositions render an explicit conflict", async ({
    page,
  }) => {
    await openGeneral(page);
    await emitRequest(
      page,
      REQ_CONFLICT,
      "@agent close out the release checklist",
    );
    await emitDisposition(page, REQ_CONFLICT, "completed", "", 1_700_000_100);
    await emitDisposition(
      page,
      REQ_CONFLICT,
      "refused",
      "changed course — this needs a human sign-off",
      1_700_000_200,
    );

    const row = requestRow(page, "@agent close out the release checklist");
    await expect(row.getByText("Disputed")).toBeVisible();
    await waitForAnimations(page);
    await row.screenshot({ path: `${SHOTS}/04-conflict-badge.png` });
  });

  test("05 — summary chip: every request resolved", async ({ page }) => {
    await openGeneral(page);
    await emitRequest(
      page,
      REQ_COMPLETED,
      "@agent summarize yesterday's decisions",
    );
    await emitDisposition(page, REQ_COMPLETED, "completed", "");
    await emitRequest(page, REQ_REFUSED, "@agent delete all of Bob's messages");
    await emitDisposition(
      page,
      REQ_REFUSED,
      "refused",
      "outside my delegation",
    );

    const chip = page.getByText(/2 visible requests · 2 settled/);
    await expect(chip).toBeVisible();
    await waitForAnimations(page);
    await chip.screenshot({ path: `${SHOTS}/05-summary-all-resolved.png` });
  });

  test("06 — summary chip expanded: unanswered and needs-attention", async ({
    page,
  }) => {
    await openGeneral(page);
    await emitRequest(
      page,
      REQ_COMPLETED,
      "@agent summarize yesterday's decisions",
    );
    await emitDisposition(page, REQ_COMPLETED, "completed", "");
    await emitRequest(
      page,
      REQ_ERRORED,
      "@agent read the deploy log and summarize",
    );
    await emitDisposition(
      page,
      REQ_ERRORED,
      "errored",
      "turn ended with an error",
    );
    await emitRequest(
      page,
      REQ_UNANSWERED,
      "@agent open a PR for the changelog",
    );

    const chip = page.getByText(/3 visible requests · 1 settled/);
    await expect(chip).toBeVisible();
    await chip.click();
    await expect(page.getByText("No record")).toBeVisible();
    await waitForAnimations(page);
    // Full-page clip: the popover is an overlay anchored below the chip.
    await page.screenshot({
      path: `${SHOTS}/06-summary-expanded.png`,
      clip: { x: 256, y: 40, width: 700, height: 420 },
    });
  });
});
