import { createHash } from "node:crypto";

import { expect, type Locator, type Page, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

/**
 * Thread recap ("Catch me up") journeys against the mock bridge. The native
 * commands are doubled in `src/testing/e2eBridgeThreadCatchUp.ts`: generation
 * is deterministic over the thread's own events and records a SHA-256 per
 * source the way native's canonical projection does, so every cited id is a
 * real message and the share payload can be checked byte for byte.
 */

const SELF = "deadbeef".repeat(8);
const ALICE = TEST_IDENTITIES.alice.pubkey;
const TYLER = TEST_IDENTITIES.tyler.pubkey;
const REPO = `30617:${SELF}:buzz`;
const SHOTS = "test-results/thread-catch-up";
const READ_STATE_KEY = `buzz.channel-read-state.v2:${SELF}`;
const CATCH_UP_KEY = "buzz-e2e-thread-catch-up.v1";

const hex = (pair: string) => pair.repeat(32);
const RELEASE = {
  root: hex("c1"),
  decision: hex("c2"),
  tasks: hex("c3"),
  question: hex("c4"),
  chatter: hex("c5"),
};
const DESIGN = {
  root: hex("d1"),
  decision: hex("d2"),
  question: hex("d3"),
  notes: hex("d4"),
};
const ISSUES = {
  mine: hex("e1"),
  someoneElse: hex("e2"),
  pTagOnly: hex("e3"),
  closed: hex("e4"),
};

type Seed = {
  id: string;
  content: string;
  pubkey: string;
  createdAt: number;
  parent?: string;
};

const issueLink = (id: string) => `buzz://issue?id=${id}&owner=${SELF}&d=buzz`;

function releaseThread(now: number): Seed[] {
  const parent = RELEASE.root;
  return [
    {
      id: RELEASE.root,
      content: "Release planning for 2.4",
      pubkey: ALICE,
      createdAt: now - 10_800,
    },
    {
      id: RELEASE.decision,
      content: "We decided to ship 2.4 on Friday.",
      pubkey: ALICE,
      createdAt: now - 3_600,
      parent,
    },
    {
      id: RELEASE.tasks,
      content: `Tracking: ${Object.values(ISSUES).map(issueLink).join(" ")}`,
      pubkey: TYLER,
      createdAt: now - 3_000,
      parent,
    },
    {
      id: RELEASE.question,
      content: "Who owns the release notes?",
      pubkey: TYLER,
      createdAt: now - 1_800,
      parent,
    },
    {
      id: RELEASE.chatter,
      content: "Sounds good.",
      pubkey: ALICE,
      createdAt: now - 600,
      parent,
    },
  ];
}

function designThread(now: number): Seed[] {
  const parent = DESIGN.root;
  return [
    {
      id: DESIGN.root,
      content: "Design review: thread side panes",
      pubkey: ALICE,
      createdAt: now - 20_000,
    },
    {
      id: DESIGN.decision,
      content: "We decided to keep the compact header.",
      pubkey: TYLER,
      createdAt: now - 5_000,
      parent,
    },
    {
      id: DESIGN.question,
      content: "Should the pane remember its size?",
      pubkey: ALICE,
      createdAt: now - 4_000,
      parent,
    },
    {
      id: DESIGN.notes,
      content: "Notes are attached to the review doc.",
      pubkey: TYLER,
      createdAt: now - 3_000,
      parent,
    },
  ];
}

/**
 * Issues in the reader's `buzz` repository. Assignment is reduced from signed
 * operations only: the `p` tag on `pTagOnly` must not count, and `closed` is
 * assigned to the reader but no longer open.
 */
function projectEvents(now: number) {
  const issue = (id: string, subject: string, extraTags: string[][] = []) => ({
    id,
    kind: 1621,
    pubkey: ALICE,
    created_at: now - 50_000,
    content: subject,
    tags: [["a", REPO], ["subject", subject], ...extraTags],
  });
  const assign = (id: string, issueId: string, assignee: string) => ({
    id,
    kind: 1,
    pubkey: ALICE,
    created_at: now - 40_000,
    content: "Assigned this issue",
    tags: [
      ["e", issueId, "", "root"],
      ["a", REPO],
      ["p", assignee],
      ["t", "assignment"],
    ],
  });
  return [
    issue(ISSUES.mine, "Write the 2.4 release notes"),
    issue(ISSUES.someoneElse, "Cut the 2.4 branch"),
    issue(ISSUES.pTagOnly, "Mentioned but unassigned task", [["p", SELF]]),
    issue(ISSUES.closed, "Already finished task"),
    assign(hex("f1"), ISSUES.mine, SELF),
    assign(hex("f2"), ISSUES.someoneElse, TYLER),
    assign(hex("f3"), ISSUES.closed, SELF),
    {
      id: hex("f4"),
      kind: 1632,
      pubkey: ALICE,
      created_at: now - 30_000,
      content: "",
      tags: [
        ["e", ISSUES.closed, "", "root"],
        ["a", REPO],
      ],
    },
  ];
}

async function seedMessages(page: Page, messages: Seed[]) {
  await expect
    .poll(() =>
      page.evaluate(
        () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
      ),
    )
    .toBe(true);
  return page.evaluate((list) => {
    return list.map((message) => {
      const event = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: message.content,
        pubkey: message.pubkey,
        createdAt: message.createdAt,
        id: message.id,
        parentEventId: message.parent ?? null,
      });
      if (!event) throw new Error("message bridge unavailable");
      return event;
    });
  }, messages);
}

async function waitForReadState(page: Page) {
  await expect
    .poll(
      () =>
        page.evaluate(
          () =>
            window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: "general",
              kind: 30078,
            }) ?? false,
        ),
      { timeout: 15_000 },
    )
    .toBe(true);
}

async function openThread(page: Page, rootId: string) {
  await page
    .locator(
      `[data-testid="message-thread-summary"][data-thread-head-id="${rootId}"]`,
    )
    .click();
  const panel = page.getByTestId("message-thread-panel");
  await expect(panel).toBeVisible();
  await expect(
    panel.locator(`[data-message-id="${rootId}"]`).first(),
  ).toBeVisible();
  return panel;
}

function setControls(page: Page, controls: Record<string, unknown>) {
  return page.evaluate((next) => {
    window.__BUZZ_E2E_THREAD_CATCH_UP__ = {
      ...window.__BUZZ_E2E_THREAD_CATCH_UP__,
      ...next,
    };
  }, controls);
}

function getControls(page: Page) {
  return page.evaluate(() => ({ ...window.__BUZZ_E2E_THREAD_CATCH_UP__ }));
}

function storedReadState(page: Page) {
  return page.evaluate(
    (key) => localStorage.getItem(key) ?? "",
    READ_STATE_KEY,
  );
}

/** Wait out the visit's own mark-read writes so later diffs are ours. */
async function settledReadState(page: Page) {
  let previous = await storedReadState(page);
  await expect
    .poll(
      async () => {
        const current = await storedReadState(page);
        const stable = current === previous;
        previous = current;
        return stable;
      },
      { intervals: [500, 500, 500] },
    )
    .toBe(true);
  return previous;
}

function sha256(content: string) {
  return createHash("sha256").update(content).digest("hex");
}

async function expectInsideViewport(page: Page, locator: Locator) {
  const box = await locator.boundingBox();
  const viewport = page.viewportSize();
  if (!box || !viewport) throw new Error("missing layout");
  expect(box.x).toBeGreaterThanOrEqual(0);
  expect(box.x + box.width).toBeLessThanOrEqual(viewport.width + 0.5);
  expect(
    await locator.evaluate(
      (element) => element.scrollWidth <= element.clientWidth + 1,
    ),
  ).toBe(true);
}

test("catch up on a thread: setup, frozen baseline, recap, sources, assignments, share, and corrections", async ({
  page,
}) => {
  const now = Math.floor(Date.now() / 1_000);
  await page.setViewportSize({ width: 1440, height: 960 });
  await page.addInitScript(
    ({ events, readStateKey, threadMarker, root }) => {
      window.__BUZZ_E2E_EXTRA_PROJECT_EVENTS__ = events;
      localStorage.setItem("buzz-theme", "buzz");
      // The previous visit read this thread two hours ago. Seed once so the
      // reload below starts from what this visit wrote.
      if (!sessionStorage.getItem("catch-up-seeded")) {
        sessionStorage.setItem("catch-up-seeded", "1");
        localStorage.setItem(
          readStateKey,
          JSON.stringify({
            [`thread:${root}`]: new Date(threadMarker * 1_000).toISOString(),
          }),
        );
      }
    },
    {
      events: projectEvents(now),
      readStateKey: READ_STATE_KEY,
      threadMarker: now - 7_200,
      root: RELEASE.root,
    },
  );
  await installMockBridge(page);
  await page.goto("/");
  await waitForReadState(page);
  const seeded = await seedMessages(page, releaseThread(now));
  const latest = seeded.find((event) => event.id === RELEASE.chatter);
  if (!latest) throw new Error("missing inbox reply");
  await expect(page.getByTestId("home-inbox")).toHaveAttribute(
    "data-read-state-ready",
    "true",
  );
  await page.evaluate(
    ({ event, self }) => {
      window.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__?.({
        id: event.id,
        kind: event.kind,
        pubkey: event.pubkey,
        content: event.content,
        created_at: event.created_at,
        channel_id: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
        channel_name: "general",
        tags: [...event.tags, ["p", self]],
        category: "mention",
      });
    },
    { event: latest, self: SELF },
  );
  // Select through the real Inbox handler, which advances the thread marker
  // before navigating. Catch up must retain the position from before that.
  await page.getByTestId(`home-inbox-item-${RELEASE.chatter}`).click();
  await expect
    .poll(async () => {
      const markers = JSON.parse(await storedReadState(page));
      return Date.parse(markers[`thread:${RELEASE.root}`]) / 1_000;
    })
    .toBeGreaterThanOrEqual(now - 600);
  await page.getByTestId("home-inbox-open-context").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const panel = page.getByTestId("message-thread-panel");
  await expect(panel).toBeVisible();
  await expect(
    panel.locator(`[data-message-id="${RELEASE.root}"]`).first(),
  ).toBeVisible();
  const toggle = panel.getByTestId("thread-catch-up-toggle");
  await expect(toggle).toHaveAttribute("aria-expanded", "false");

  // Keyboard open: the pane takes focus and the toggle reports it.
  await toggle.focus();
  await page.keyboard.press("Enter");
  const pane = page.getByTestId("thread-catch-up-pane");
  await expect(pane).toBeVisible();
  await expect(toggle).toHaveAttribute("aria-expanded", "true");
  await expect(pane).toBeFocused();

  // Local-only setup: no API key field, hosted or `localhost` endpoints refused.
  const setup = pane.getByTestId("thread-catch-up-setup");
  await expect(setup).toBeVisible();
  await expect(pane.getByLabel(/api key/i)).toHaveCount(0);
  await expect(
    setup.getByTestId("thread-catch-up-setup-requirements"),
  ).toContainText("at least 32,768 tokens");
  await waitForAnimations(page);
  await panel.screenshot({ path: `${SHOTS}/00-setup-light.png` });
  await setup.getByLabel("Local endpoint").fill("http://localhost:11434/v1");
  await setup.getByLabel("Model", { exact: true }).fill("qwen3:4b");
  await setup
    .getByRole("checkbox", { name: /runs the model on this computer/ })
    .check();
  await setup.getByTestId("thread-catch-up-setup-save").click();
  await expect(pane.getByTestId("thread-catch-up-setup-error")).toBeVisible();
  await setup.getByLabel("Local endpoint").fill("http://127.0.0.1:11434/v1");
  await setup.getByTestId("thread-catch-up-setup-save").click();
  await expect(setup).toHaveCount(0);

  // Opening the channel advanced its read marker past every reply, yet the
  // baseline is the position frozen before this visit: four new replies.
  const baseline = pane.getByTestId("thread-catch-up-baseline");
  await expect(baseline).toHaveAttribute("data-baseline", "marker");
  await expect(baseline).toContainText("4 new replies");
  const readStateAfterVisit = await settledReadState(page);
  const visitMarkers = Object.values(
    JSON.parse(readStateAfterVisit) as Record<string, string>,
  ).map((iso) => Date.parse(iso) / 1_000);
  expect(Math.max(...visitMarkers)).toBeGreaterThanOrEqual(now - 600);

  // Cancel a slow generation; native receives the cancellation.
  await setControls(page, { generationDelayMs: 10_000 });
  await pane.getByTestId("thread-catch-up-generate").click();
  await expect(pane.getByTestId("thread-catch-up-generating")).toBeVisible();
  await pane.getByTestId("thread-catch-up-cancel").click();
  await expect(baseline).toBeVisible();
  expect((await getControls(page)).cancelled).toHaveLength(1);

  // A failed generation keeps the chosen period for Retry.
  await setControls(page, {
    generationDelayMs: 0,
    failNextGeneration: "The local model stopped responding.",
  });
  await pane.getByTestId("thread-catch-up-generate").click();
  await expect(pane.getByTestId("thread-catch-up-error")).toContainText(
    "The local model stopped responding.",
  );
  await pane.getByTestId("thread-catch-up-retry").click();

  const recap = pane.getByTestId("thread-catch-up-recap");
  await expect(recap).toBeVisible();
  const generated = (await getControls(page)).generated ?? [];
  expect(generated.at(-1)?.since).toBe(now - 3_600);
  await expect(recap).toContainText("Since your previous read position");
  await expect(pane.getByTestId("thread-catch-up-coverage")).toHaveText(
    "Read 4 retrieved messages",
  );
  await expect(
    pane.getByTestId("thread-catch-up-section-decision"),
  ).toContainText("Decision: We decided to ship 2.4 on Friday.");
  await expect(
    pane.getByTestId("thread-catch-up-section-question"),
  ).toContainText("Open question: Who owns the release notes?");

  // Deterministic assignments: only the open task with a signed assignment to
  // the reader, never the `p`-tagged or someone else's or closed one.
  const tasks = pane.getByTestId("thread-catch-up-task");
  await expect(tasks).toHaveCount(1);
  await expect(tasks).toContainText("Write the 2.4 release notes");

  await waitForAnimations(page);
  await panel.screenshot({ path: `${SHOTS}/01-recap-light.png` });

  // Generating and reading the recap changed nothing about read state.
  expect(await storedReadState(page)).toBe(readStateAfterVisit);

  // Source jump: navigates to the exact message in this thread (the route
  // consumes `messageId` once it lands), keeping the thread and recap open.
  const jumped = page.waitForURL(
    new RegExp(
      `messageId=${RELEASE.question}.*threadRootId=${RELEASE.root}|threadRootId=${RELEASE.root}.*messageId=${RELEASE.question}`,
    ),
  );
  await recap
    .locator(
      `[data-testid="thread-catch-up-source"][data-source-id="${RELEASE.question}"]`,
    )
    .click();
  await jumped;
  await expect(
    panel.locator(`[data-message-id="${RELEASE.question}"]`).first(),
  ).toBeInViewport();
  await expect(recap).toBeVisible();

  // Share: native refuses because a cited source changed; the recap stays and
  // both recoveries are offered. Regenerate, then share the fresh recap.
  await setControls(page, { shareSourceChanged: true });
  await recap.getByTestId("thread-catch-up-share").click();
  await recap.getByTestId("thread-catch-up-share-confirm-button").click();
  const sourceChanged = recap.getByTestId(
    "thread-catch-up-share-source-changed",
  );
  await expect(sourceChanged).toBeVisible();
  await expect(recap.getByTestId("thread-catch-up-point")).toHaveCount(2);
  await expect(
    sourceChanged.getByTestId("thread-catch-up-share-retry"),
  ).toBeEnabled();
  await sourceChanged.getByTestId("thread-catch-up-regenerate").click();
  await expect(sourceChanged).toHaveCount(0);
  await expect(recap.getByTestId("thread-catch-up-point")).toHaveCount(2);
  expect((await getControls(page)).generated).toHaveLength(
    generated.length + 1,
  );
  await recap.getByTestId("thread-catch-up-share").click();
  await recap.getByTestId("thread-catch-up-share-confirm-button").click();
  await expect(
    recap.getByTestId("thread-catch-up-shared-notice"),
  ).toBeVisible();

  // Share sent native's source revisions unchanged.
  const stored = await page.evaluate(
    (key) => JSON.parse(localStorage.getItem(key) ?? "{}"),
    CATCH_UP_KEY,
  );
  const thread = releaseThread(now).filter(
    (message) =>
      message.id === RELEASE.decision || message.id === RELEASE.question,
  );
  expect(stored.shared).toHaveLength(1);
  expect(stored.shared[0].recap.sourceRevisions).toEqual(
    Object.fromEntries(
      thread.map((message) => [message.id, sha256(message.content)]),
    ),
  );

  const shared = pane.getByTestId("thread-catch-up-shared-recap");
  await expect(shared).toHaveCount(1);
  await expect(shared.getByTestId("thread-catch-up-attribution")).toContainText(
    "Shared by",
  );

  // Per-point correction with history.
  const decision = shared
    .getByTestId("thread-catch-up-shared-point")
    .filter({ hasText: "Decisions" });
  await decision.getByTestId("thread-catch-up-correct").click();
  await decision
    .getByTestId("thread-catch-up-correction-input")
    .fill("Decision: ship 2.4 on Friday after QA signs off.");
  await decision.getByTestId("thread-catch-up-correction-save").click();
  await expect(
    decision.getByTestId("thread-catch-up-shared-point-text"),
  ).toHaveText("Decision: ship 2.4 on Friday after QA signs off.");
  await expect(decision).toContainText("Corrected by");
  const history = decision.getByTestId("thread-catch-up-history");
  await history.locator("summary").click();
  await expect(history).toContainText("History (2)");
  await expect(history).toContainText(
    "Decision: We decided to ship 2.4 on Friday.",
  );

  // A concurrent correction wins the race: the draft is kept, the latest
  // version loads, and saving again applies on top of it.
  const question = shared
    .getByTestId("thread-catch-up-shared-point")
    .filter({ hasText: "Still open" });
  await setControls(page, {
    conflictNextCorrection: true,
    conflictAuthor: ALICE,
  });
  await question.getByTestId("thread-catch-up-correct").click();
  const draft = "Open question: Tyler to confirm who owns the release notes.";
  const input = question.getByTestId("thread-catch-up-correction-input");
  await input.fill(draft);
  await question.getByTestId("thread-catch-up-correction-save").click();
  await expect(
    question.getByTestId("thread-catch-up-correction-error"),
  ).toHaveAttribute("data-conflict", "true");
  await expect(input).toHaveValue(draft);
  await expect(
    question.getByTestId("thread-catch-up-shared-point-text"),
  ).toHaveText("Another member corrected this first.");
  await question.getByTestId("thread-catch-up-correction-save").click();
  await expect(
    question.getByTestId("thread-catch-up-shared-point-text"),
  ).toHaveText(draft);
  await expect(input).toHaveCount(0);

  await waitForAnimations(page);
  await panel.screenshot({ path: `${SHOTS}/02-shared-light.png` });

  // Sharing and correcting never moved this thread's read marker.
  expect(
    JSON.parse(await storedReadState(page))[`thread:${RELEASE.root}`],
  ).toBe(JSON.parse(readStateAfterVisit)[`thread:${RELEASE.root}`]);

  // Closing returns focus to the toggle.
  await pane.getByTestId("thread-catch-up-close").click();
  await expect(pane).toBeHidden();
  await expect(toggle).toBeFocused();
});

test("catch up in a narrow dark window: explicit period, thread switch, partial coverage, incomplete list", async ({
  page,
}) => {
  const now = Math.floor(Date.now() / 1_000);
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.addInitScript(() => {
    localStorage.setItem("buzz-theme", "buzz-dark");
    window.__BUZZ_E2E_THREAD_CATCH_UP__ = {
      unreadableCount: 2,
      complete: false,
      historyComplete: false,
    };
  });
  await installMockBridge(page);
  await page.goto("/");
  await waitForReadState(page);
  await seedMessages(page, [...designThread(now), ...releaseThread(now)]);
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  const channelId = new URL(page.url()).hash.match(
    /channels\/([0-9a-f-]{36})/,
  )?.[1];
  if (!channelId) throw new Error("general channel id missing from route");
  // The window narrows once the channel is open.
  await page.setViewportSize({ width: 760, height: 900 });

  // Already set up, and a recap shared earlier whose first revision is no
  // longer retrievable, so the list can't say who shared it.
  const earlier = {
    artifactId: "6f1c0a52-0000-5000-8000-000000000001",
    eventId: hex("aa"),
    authorPubkey: TYLER,
    createdAt: now - 4_500,
    recap: {
      version: 1,
      channelId,
      rootEventId: DESIGN.root,
      since: now - 6_000,
      until: now - 4_500,
      points: [
        {
          id: "p1",
          category: "decision",
          text: "Decision: keep the compact header.",
          sourceIds: [DESIGN.decision],
        },
      ],
      sourceIds: [DESIGN.decision],
      coverage: {
        messageCount: 1,
        inputMessageCount: 1,
        truncated: false,
        warnings: [],
      },
    },
    corrections: [],
    historyComplete: true,
  };
  await page.evaluate(
    ({ key, shared }) => {
      localStorage.setItem(
        key,
        JSON.stringify({
          settings: {
            baseUrl: "http://127.0.0.1:11434/v1",
            model: "qwen3:4b",
            confirmedLocal: true,
          },
          shared: [shared],
        }),
      );
    },
    { key: CATCH_UP_KEY, shared: earlier },
  );

  const panel = await openThread(page, DESIGN.root);
  const toggle = panel.getByTestId("thread-catch-up-toggle");
  // Narrow panels may drop the label; the accessible name stays.
  await expect(toggle).toHaveAccessibleName(/Catch me up/);
  await toggle.click();
  const pane = page.getByTestId("thread-catch-up-pane");
  await expect(pane).toBeVisible();

  // No previous read position for this thread: there is no "since you last
  // read" option, so the reader picks the period explicitly.
  const baseline = pane.getByTestId("thread-catch-up-baseline");
  await expect(baseline).toHaveAttribute(
    "data-baseline",
    /^(absent|unresolved|expired)$/,
  );
  await expect(pane.getByTestId("thread-catch-up-generate")).toHaveCount(0);

  // Shared list incompleteness is surfaced, and incomplete history makes no
  // claim about who shared the recap.
  const attribution = pane.getByTestId("thread-catch-up-attribution");
  await expect(attribution).toContainText("Sharer unknown");
  await expect(attribution).not.toContainText("Shared by");
  await expect(pane.getByTestId("thread-catch-up-unreadable")).toContainText(
    "2 shared recaps couldn't be verified",
  );
  await expect(pane.getByTestId("thread-catch-up-truncated")).toBeVisible();

  // Native's input budget covers only part of the period.
  await setControls(page, { inputLimit: 2 });
  await pane.getByTestId("thread-catch-up-lookback-week").click();
  const recap = pane.getByTestId("thread-catch-up-recap");
  await expect(recap).toContainText("Last 7 days");
  await expect(pane.getByTestId("thread-catch-up-coverage")).toHaveText(
    "Partial: read 2 of 4 retrieved messages",
  );
  await expect(pane.getByTestId("thread-catch-up-warnings")).toContainText(
    "Only the newest 2",
  );
  await expect(recap.getByTestId("thread-catch-up-point")).toHaveText([
    "Open question: Should the pane remember its size?",
  ]);
  const week = (await getControls(page)).generated?.at(-1);
  expect((week?.until ?? 0) - (week?.since ?? 0)).toBe(7 * 24 * 60 * 60);

  await expectInsideViewport(page, pane);
  await waitForAnimations(page);
  await panel.screenshot({ path: `${SHOTS}/03-partial-dark-narrow.png` });

  // Switching threads mid-generation cancels it and resets the pane.
  await pane.getByTestId("thread-catch-up-new").click();
  await setControls(page, { generationDelayMs: 10_000, inputLimit: undefined });
  await pane.getByTestId("thread-catch-up-lookback-day").click();
  await expect(pane.getByTestId("thread-catch-up-generating")).toBeVisible();
  const cancelledBefore = (await getControls(page)).cancelled?.length ?? 0;
  const releasePanel = await openThread(page, RELEASE.root);
  await expect
    .poll(async () => (await getControls(page)).cancelled?.length ?? 0)
    .toBe(cancelledBefore + 1);
  const releaseToggle = releasePanel.getByTestId("thread-catch-up-toggle");
  await expect(releaseToggle).toHaveAttribute("aria-expanded", "false");
  await releaseToggle.click();
  await expect(pane.getByTestId("thread-catch-up-generating")).toHaveCount(0);
  await expect(pane.getByTestId("thread-catch-up-recap")).toHaveCount(0);
  await expect(pane.getByTestId("thread-catch-up-baseline")).toBeVisible();
  await expect(pane.getByTestId("thread-catch-up-shared-recap")).toHaveCount(0);
  await expect(pane).not.toContainText(
    "No recaps have been shared with this thread.",
  );

  await expectInsideViewport(page, pane);
  await waitForAnimations(page);
  await releasePanel.screenshot({
    path: `${SHOTS}/04-switched-dark-narrow.png`,
  });
});
