import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

// NIP-AD disposition surface: badges on request messages and the channel
// accountability summary chip. See docs/nips/NIP-AD.md and
// desktop/src/features/messages/lib/{formatTimelineMessages,
// dispositionSummary}.ts.

const REQUEST_ID_COMPLETED = "1".repeat(64);
const REQUEST_ID_REFUSED = "2".repeat(64);
const REQUEST_ID_UNANSWERED = "3".repeat(64);

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
  kind?: number,
) {
  await expect
    .poll(async () => {
      return page.evaluate(
        ({ currentChannelName, kind: k }) => {
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
              kind: k,
            }) ?? false
          );
        },
        { currentChannelName: channelName, kind },
      );
    })
    .toBe(true);
}

async function openGeneral(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");
}

// The agent every request below is addressed to, and that signs every
// disposition. Binding requires the request to name its disposition's
// signer as an `agent` target — see docs/nips/NIP-AD.md.
// Mock-bridge constants the binding now depends on: a disposition must
// carry the request's own channel and author to bind.
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const REQUESTER_PUBKEY = "deadbeef".repeat(8);
const AGENT_PUBKEY = "b".repeat(64);
// A human CC'd on requests but never an `agent` target.
const HUMAN_PUBKEY = "c".repeat(64);

function emitRequest(
  page: import("@playwright/test").Page,
  id: string,
  content: string,
) {
  return page.evaluate(
    ({ id: eventId, content: body, agent, human }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: body,
        id: eventId,
        extraTags: [
          ["t", "request"],
          ["agent", agent],
          ["p", agent],
          ["p", human],
        ],
      });
    },
    { id, content, agent: AGENT_PUBKEY, human: HUMAN_PUBKEY },
  );
}

function emitDisposition(
  page: import("@playwright/test").Page,
  requestId: string,
  disposition: "completed" | "refused" | "errored",
  reason: string,
  signer: string = AGENT_PUBKEY,
) {
  return page.evaluate(
    ({
      requestId: e,
      disposition: d,
      reason: r,
      signer: pubkey,
      channelId,
      requester,
    }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: JSON.stringify({ disposition: d, reason: r }),
        kind: 44300,
        pubkey,
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
      signer,
      channelId: GENERAL_CHANNEL_ID,
      requester: REQUESTER_PUBKEY,
    },
  );
}

function requestRow(page: import("@playwright/test").Page, content: string) {
  return page.getByTestId("message-row").filter({ hasText: content }).last();
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("a completed disposition shows a completed badge on the request message", async ({
  page,
}) => {
  await openGeneral(page);
  await emitRequest(page, REQUEST_ID_COMPLETED, "@agent summarize yesterday");
  await emitDisposition(page, REQUEST_ID_COMPLETED, "completed", "");

  const row = requestRow(page, "@agent summarize yesterday");
  await expect(row.getByText("Completed")).toBeVisible();
});

test("a refused disposition's reason is readable from the badge without leaving the timeline", async ({
  page,
}) => {
  await openGeneral(page);
  await emitRequest(page, REQUEST_ID_REFUSED, "@agent delete everything");
  await emitDisposition(
    page,
    REQUEST_ID_REFUSED,
    "refused",
    "outside my delegation",
  );

  const row = requestRow(page, "@agent delete everything");
  const badge = row.getByText("Refused");
  await expect(badge).toBeVisible();

  await waitForAnimations(page);
  await badge.hover();
  await expect(page.getByText("outside my delegation")).toBeVisible();
});

test("channel summary chip counts marked requests and flags the unanswered one", async ({
  page,
}) => {
  await openGeneral(page);
  await emitRequest(page, REQUEST_ID_COMPLETED, "@agent summarize yesterday");
  await emitDisposition(page, REQUEST_ID_COMPLETED, "completed", "");
  await emitRequest(page, REQUEST_ID_UNANSWERED, "@agent open a PR for this");

  const chip = page.getByText(/2 visible requests · 1 settled/);
  await expect(chip).toBeVisible();

  await chip.click();
  await expect(page.getByText("No record")).toBeVisible();
  await expect(
    page.getByRole("button", { name: "@agent open a PR for this" }),
  ).toBeVisible();
});

test("an ordinary acknowledgement without an agent mention is never counted as a request", async ({
  page,
}) => {
  await openGeneral(page);
  await page.evaluate(() => {
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: "thanks!",
    });
  });

  await expect(page.getByText(/visible requests? ·/)).not.toBeVisible();
});

test("a disposition signed by a merely-mentioned human does not resolve the request", async ({
  page,
}) => {
  // The end-to-end form of the cross-principal binding check: HUMAN_PUBKEY
  // is `p`-mentioned on the request but is not an `agent` target, so its
  // signed "completed" must leave the request visibly unanswered rather
  // than rendering a Completed badge.
  await openGeneral(page);
  await emitRequest(page, REQUEST_ID_COMPLETED, "@agent summarize yesterday");
  await emitDisposition(
    page,
    REQUEST_ID_COMPLETED,
    "completed",
    "",
    HUMAN_PUBKEY,
  );

  const row = requestRow(page, "@agent summarize yesterday");
  await expect(row.getByText("Completed")).toHaveCount(0);
  await expect(page.getByText(/1 visible request · 0 settled/)).toBeVisible();
});

test("the summary occupies no layout in a channel with no marked requests", async ({
  page,
}) => {
  // The chip returns null for an empty channel, but its layout wrapper was
  // rendered unconditionally, so `pb-1.5` reserved 6px under the header of
  // every channel with no marked requests — nearly all of them — and pushed
  // the timeline's sticky day divider off its designed offset. Upstream's
  // date-divider test caught that only incidentally, by measuring the offset.
  //
  // This asserts the invariant directly: nothing to show, no element. The slot
  // and the chip are gated on one shared predicate (`hasVisibleRequests`)
  // rather than two copies of `total === 0` free to drift apart.
  await openGeneral(page);
  await expect(page.getByTestId("disposition-summary-slot")).toHaveCount(0);

  // ...and it appears once there is something to summarize, so the assertion
  // above cannot pass merely because the slot never renders at all.
  await emitRequest(page, REQUEST_ID_COMPLETED, "@agent summarize yesterday");
  await expect(page.getByTestId("disposition-summary-slot")).toHaveCount(1);
});
