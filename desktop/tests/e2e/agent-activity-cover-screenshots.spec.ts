import { expect, test, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/agent-activity-cover";

const AGENT_PUBKEY = TEST_IDENTITIES.tyler.pubkey;
const HUMAN_PUBKEY = TEST_IDENTITIES.bob.pubkey;
const CHANNEL_ID = "94a444a4-c0a3-5966-ab05-530c6ddc2301"; // #agents
const SESSION_ID = "session-cover-001";
const TURN_ID = "turn-cover-001";

/**
 * The width the drawer is designed for: two panes fit, so activity covers, and
 * the drawer gets the channel content area less the sliver. Anything narrower
 * is a different presentation with its own spec
 * (`agent-activity-cover.spec.ts`), so the reference shot is taken here.
 */
const DRAWER_VIEWPORT = { width: 1440, height: 900 };

const MANAGED_AGENTS = [
  {
    pubkey: AGENT_PUBKEY,
    name: "Observer Agent",
    status: "running" as const,
    channelNames: ["agents"],
  },
];

/**
 * Anchor the turn just before "now" so the header's relative recency label
 * ("Last updated 1m ago") reads like a live session rather than an archive.
 * Absolute timestamps inside the transcript are not asserted, so a moving
 * anchor costs nothing and keeps the reference shot honest.
 */
const TURN_START_MS = Date.now() - 90_000;
const at = (offsetSeconds: number) =>
  new Date(TURN_START_MS + offsetSeconds * 1_000).toISOString();

type ObserverEventSeed = {
  seq: number;
  timestamp: string;
  kind: string;
  agentIndex: number | null;
  channelId: string | null;
  sessionId: string | null;
  turnId: string | null;
  payload: unknown;
};

let seq = 0;

function sessionUpdate(
  offsetSeconds: number,
  update: unknown,
): ObserverEventSeed {
  seq += 1;
  return {
    seq,
    timestamp: at(offsetSeconds),
    kind: "acp_read",
    agentIndex: 0,
    channelId: CHANNEL_ID,
    sessionId: SESSION_ID,
    turnId: TURN_ID,
    payload: {
      jsonrpc: "2.0",
      method: "session/update",
      params: { sessionId: SESSION_ID, update },
    },
  };
}

/**
 * A tool call as the harness actually reports it: an `in_progress` announcement
 * followed by a terminal update carrying the output. Seeding only the terminal
 * update would skip the correlation path the transcript uses to pair them.
 */
function toolCall(
  offsetSeconds: number,
  input: {
    args: Record<string, unknown>;
    failed?: boolean;
    id: string;
    output: string;
    title: string;
    toolName: string;
  },
): ObserverEventSeed[] {
  return [
    sessionUpdate(offsetSeconds, {
      sessionUpdate: "tool_call",
      rawInput: input.args,
      status: "in_progress",
      title: input.title,
      toolCallId: input.id,
      toolName: input.toolName,
    }),
    sessionUpdate(offsetSeconds + 1, {
      content: [
        { type: "content", content: { type: "text", text: input.output } },
      ],
      rawInput: input.args,
      sessionUpdate: "tool_call_update",
      status: input.failed ? "failed" : "completed",
      title: input.title,
      toolCallId: input.id,
      toolName: input.toolName,
    }),
  ];
}

/**
 * One finished turn with the shape a real investigation has: a mention that
 * starts it, thinking, file reads, a shell command, a relay post, a step that
 * failed, a plan, and an answer containing code.
 *
 * The prompt is framed the way the harness frames it — a `[Buzz event: ...]`
 * section with `From:`/`Content:` lines — because `parsePromptText` reads the
 * author pubkey and the user-visible text out of exactly that shape. A plain
 * text prompt would render as an unattributed bubble and would not exercise the
 * header the drawer is meant to make readable.
 */
function buildTurnEvents(): ObserverEventSeed[] {
  seq = 0;
  const events: ObserverEventSeed[] = [];

  seq += 1;
  events.push({
    seq,
    timestamp: at(0),
    kind: "acp_write",
    agentIndex: 0,
    channelId: CHANNEL_ID,
    sessionId: SESSION_ID,
    turnId: TURN_ID,
    payload: {
      jsonrpc: "2.0",
      id: 1,
      method: "session/prompt",
      params: {
        prompt: [
          {
            type: "text",
            text: [
              "[Buzz event: @mention]",
              "Event ID: 4f1c8e6d2b7a90c3e5148af6b0d29c73518ea4d6c09b7f2318ad45e6019cb372",
              "Channel: agents (#94a444a4-c0a3-5966-ab05-530c6ddc2301)",
              "Kind: 9",
              `From: bob (npub: npub1hv32jnyjyr9dwmlagvvsejul4j4ushx2vph2ghde5ktxxu6hlxcqzt5qsn, hex: ${HUMAN_PUBKEY})`,
              "Time: 2026-08-24T18:04:11+00:00",
              "Content: @Observer Agent the mention badge lands on the wrong channel row after a reconnect. Trace where the feed category is set and confirm whether the singular/plural mismatch is the cause. Post what you find here.",
            ].join("\n"),
          },
          {
            type: "text",
            text: "[Thread context]\nThis is the thread history with 3 prior messages.",
          },
        ],
      },
    },
  });

  events.push(
    sessionUpdate(4, {
      sessionUpdate: "agent_thought_chunk",
      messageId: "thought-1",
      content: {
        type: "text",
        text: "The badge is driven by the feed category on the alert event, so a mismatch would show up where that string is built. Start at the emit site, then follow the value into the sidebar row selector.",
      },
    }),
  );

  events.push(
    ...toolCall(12, {
      args: { path: "desktop/src/features/feed/lib/feedCategory.ts" },
      id: "call-read-1",
      output: 'export type FeedCategory = "mention" | "reply" | "reaction";',
      title: "read_file",
      toolName: "buzz_dev_mcp__read_file",
    }),
    ...toolCall(15, {
      args: { path: "desktop/src/features/feed/lib/alertRouting.ts" },
      id: "call-read-2",
      output: 'if (category === "mentions") { routeToChannel(channelId); }',
      title: "read_file",
      toolName: "buzz_dev_mcp__read_file",
    }),
    ...toolCall(18, {
      args: { path: "desktop/src/features/channels/ui/ChannelRowBadge.tsx" },
      id: "call-read-3",
      output: 'const hasMention = categories.includes("mention");',
      title: "read_file",
      toolName: "buzz_dev_mcp__read_file",
    }),
  );

  events.push(
    ...toolCall(24, {
      args: {
        command: "rg -n 'mentions\"' desktop/src --glob '*.ts' --glob '*.tsx'",
      },
      id: "call-shell-1",
      output: [
        'desktop/src/features/feed/lib/alertRouting.ts:41:  if (category === "mentions") {',
        'desktop/src/features/feed/emitFeedAlert.ts:88:    category: "mentions",',
        'desktop/src/features/feed/emitFeedAlert.test.mjs:12:  category: "mentions",',
        "",
        "3 matches across 3 files",
      ].join("\n"),
      title: "shell",
      toolName: "buzz_dev_mcp__shell",
    }),
  );

  events.push(
    ...toolCall(31, {
      args: {
        command:
          "buzz messages send --channel 94a444a4-c0a3-5966-ab05-530c6ddc2301 --content 'Confirmed the plural/singular mismatch at the emit site.'",
      },
      id: "call-relay-1",
      output: '{\n  "accepted": true,\n  "event_id": "a41c9e2f…"\n}',
      title: "shell",
      toolName: "buzz_dev_mcp__shell",
    }),
  );

  events.push(
    ...toolCall(37, {
      args: { command: "pnpm vitest run desktop/src/features/feed" },
      failed: true,
      id: "call-fail-1",
      output:
        "Error: ENOENT: no such file or directory, scandir 'desktop/src/features/feed/__tests__'\n    at readdirSync (node:fs:1478:26)\nexit code 1",
      title: "shell",
      toolName: "buzz_dev_mcp__shell",
    }),
  );

  events.push(
    sessionUpdate(44, {
      sessionUpdate: "plan",
      entries: [
        { content: "Trace the feed category emit site", status: "completed" },
        {
          content: "Confirm the sidebar row reads the singular form",
          status: "completed",
        },
        {
          content: "Add a regression test for the routing branch",
          status: "pending",
        },
        { content: "Open the fix PR", status: "pending" },
      ],
    }),
  );

  events.push(
    sessionUpdate(52, {
      sessionUpdate: "agent_message_chunk",
      messageId: "reply-1",
      content: {
        type: "text",
        text: [
          'Confirmed — it is the singular/plural mismatch, and it is on the emit side. `emitFeedAlert` writes `category: "mentions"` while every reader compares against the singular `"mention"`, so the alert never matches the mention branch and falls through to the generic channel-activity path. That is why the badge lands on whichever row was last touched rather than the mentioning channel.',
          "",
          "```ts",
          "// desktop/src/features/feed/emitFeedAlert.ts",
          '-  category: "mentions",',
          '+  category: "mention",',
          "```",
          "",
          "The reader side needs no change. No fix pushed yet — the feed test directory the suite expects does not exist, so the regression test needs a home first.",
        ].join("\n"),
      },
    }),
  );

  return events;
}

async function seedTurn(page: Page) {
  await expect
    .poll(() =>
      page.evaluate(
        () => typeof window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__ === "function",
      ),
    )
    .toBe(true);
  await page.evaluate(
    ({ evts, pubkey }) => {
      window.__BUZZ_E2E_SEED_OBSERVER_EVENTS__?.({
        agentPubkey: pubkey,
        events: evts,
      });
    },
    { evts: buildTurnEvents(), pubkey: AGENT_PUBKEY },
  );
}

/**
 * Composer activity bar → the agent's row.
 *
 * This ingress has no prior pane, so the drawer opens with its own close
 * affordance and no back arrow — the presentation this reference shot is of.
 */
async function openActivityFromComposer(page: Page) {
  await page.getByTestId("channel-agents").click();
  await expect(page.getByTestId("chat-title")).toHaveText("agents");

  await page.evaluate((pubkey) => {
    window.__BUZZ_E2E_EMIT_MOCK_TYPING__?.({
      channelName: "agents",
      pubkey,
    });
  }, AGENT_PUBKEY);

  const trigger = page.getByTestId("bot-activity-composer-trigger");
  await expect(trigger).toBeVisible();
  await trigger.click();
  const item = page.getByTestId(`bot-activity-composer-item-${AGENT_PUBKEY}`);
  await expect(item).toBeVisible();
  await item.click({ force: true });
}

/**
 * Scroll the transcript to the head of the turn and confirm it stayed there.
 *
 * Finds the scrolling ancestor by computed overflow rather than by class name:
 * the transcript's styling belongs to the transcript variants, which are being
 * restyled in parallel, and this spec must not break when they change.
 *
 * The panel is tail-anchored, and growing its content (by expanding folds) pins
 * it back to the bottom — so a scroll issued while that is still settling gets
 * undone a frame later and the shot silently becomes a second tail frame.
 * Re-issues the scroll and then re-reads it on a later task, so the assertion
 * only passes once the position actually survives a frame.
 */
async function scrollTranscriptToTop(page: Page) {
  const panel = page.getByTestId("agent-session-thread-panel");
  const scrollToTopAndSettle = () =>
    panel.evaluate((element) => {
      let node = element.querySelector('[role="log"]')?.parentElement ?? null;
      while (node) {
        const overflowY = window.getComputedStyle(node).overflowY;
        if (
          (overflowY === "auto" || overflowY === "scroll") &&
          node.scrollHeight > node.clientHeight
        ) {
          const scroller = node;
          scroller.scrollTop = 0;
          // Read back after two frames: a re-pin from the tail anchor lands in
          // an effect or rAF, so an immediate read would report the write rather
          // than the outcome.
          return new Promise<number>((resolve) => {
            requestAnimationFrame(() =>
              requestAnimationFrame(() => resolve(scroller.scrollTop)),
            );
          });
        }
        node = node.parentElement;
      }
      return Promise.resolve(-1);
    });

  await expect.poll(scrollToTopAndSettle, { timeout: 10_000 }).toBe(0);
}

/**
 * Scroll the transcript to the tail of the turn.
 *
 * Same scroller discovery as {@link scrollTranscriptToTop}; no re-pin race to
 * fight here because the tail is where the anchor wants to be anyway.
 */
async function scrollTranscriptToBottom(page: Page) {
  await page.getByTestId("agent-session-thread-panel").evaluate((element) => {
    let node = element.querySelector('[role="log"]')?.parentElement ?? null;
    while (node) {
      const overflowY = window.getComputedStyle(node).overflowY;
      if (
        (overflowY === "auto" || overflowY === "scroll") &&
        node.scrollHeight > node.clientHeight
      ) {
        node.scrollTop = node.scrollHeight;
        return;
      }
      node = node.parentElement;
    }
  });
}

/**
 * Open every folded work block in the transcript.
 *
 * The transcript opens with finished work folded, so the default frame is a
 * stack of one-line summaries—true to the product, but it shows none of the
 * turn's actual shape. Expanding gives the second shot the content the drawer's
 * width exists for: command output, a failed step, plan items.
 *
 * Drive the product's summary buttons rather than reaching through the old
 * `<details>` implementation. This keeps the reference flow exercising the same
 * disclosure state a reader uses.
 */
async function expandTranscriptRows(page: Page) {
  const summaries = page.getByTestId("transcript-work-block-summary");
  const count = await summaries.count();
  expect(count).toBeGreaterThan(0);
  for (let index = 0; index < count; index += 1) {
    const summary = summaries.nth(index);
    if ((await summary.getAttribute("aria-expanded")) !== "true") {
      await summary.click();
    }
  }
}

/**
 * Reference screenshots of a realistic agent turn in the cover drawer.
 *
 * The PNGs are the deliverable — they are what design and review look at, and
 * regenerating them is the point of keeping this spec. So the assertions are
 * deliberately limited to what Slice A owns: the drawer covers, the panel is
 * mounted inside it, and there is no split resize handle. Transcript structure,
 * grouping, and styling belong to the transcript variants and are asserted by
 * their own specs; asserting them here would make the reference shots fail for
 * reasons that have nothing to do with the drawer.
 */
test.describe("agent activity cover drawer screenshots", () => {
  test.use({ viewport: DRAWER_VIEWPORT });

  test("realistic turn in the cover drawer", async ({ page }) => {
    await installMockBridge(page, { managedAgents: MANAGED_AGENTS });
    await page.goto("/", { waitUntil: "domcontentloaded" });

    // Seed before opening so the panel has content on its first paint, and
    // again after: the panel subscribes on mount, and re-seeding is how the
    // observer store notifies an already-mounted subscriber.
    await seedTurn(page);
    await openActivityFromComposer(page);
    await seedTurn(page);

    const drawer = page.getByTestId("agent-activity-drawer");
    const panel = page.getByTestId("agent-session-thread-panel");
    await expect(drawer).toBeVisible();
    await expect(
      drawer.getByTestId("agent-session-thread-panel"),
    ).toBeVisible();
    await expect(
      page.getByTestId("right-auxiliary-pane-resize-handle"),
    ).toHaveCount(0);
    await expect(page.getByTestId("channel-drop-zone")).toHaveAttribute(
      "inert",
      "",
    );

    // The turn actually rendered — without this the shots could be of an empty
    // drawer and still pass every structural assertion above.
    await expect(
      page.getByTestId("transcript-user-message").first(),
    ).toBeVisible({ timeout: 10_000 });

    // The reading view is pinned by presentation rather than inferred from panel
    // width: the drawer is the reading surface, so its transcript renders the
    // conversation variant. Asserted here rather than in
    // `agent-activity-cover.spec.ts` because the marker only exists once the
    // transcript has content, and this is the spec that seeds a turn. Asserted
    // on the DOM marker rather than the prop so it proves the value survives the
    // whole path from the presentation resolver through the panel.
    await expect(panel.locator("[data-transcript-variant]")).toHaveAttribute(
      "data-transcript-variant",
      "conversation",
    );

    await waitForAnimations(page);
    // Full window: the drawer against the sliver and the scrimmed channel,
    // which is the part of this presentation a panel-only shot cannot show.
    // Folds are left as the product leaves them — collapsed on open.
    await page.screenshot({ path: `${SHOTS}/01-turn-in-drawer.png` });

    // Second shot expanded, at the head: this is the frame that shows what the
    // drawer's width buys — the prompt, thinking, and the reads and shell output
    // that are invisible while the runs are folded. Scrolled back to the head
    // because expanding overflows the panel and it is anchored to the tail.
    await expandTranscriptRows(page);
    await scrollTranscriptToTop(page);
    await waitForAnimations(page);
    await panel.screenshot({ path: `${SHOTS}/02-turn-expanded-head.png` });

    // Third shot, the tail of the same expanded turn: the failed step with its
    // error, the plan, and the answer with code. Expanded, the turn is taller
    // than the drawer, so no single frame holds both ends of it.
    await scrollTranscriptToBottom(page);
    await waitForAnimations(page);
    await panel.screenshot({ path: `${SHOTS}/03-turn-expanded-tail.png` });
  });
});
