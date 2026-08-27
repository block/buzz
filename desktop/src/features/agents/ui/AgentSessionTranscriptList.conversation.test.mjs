/**
 * Rendering contract for the `conversation` transcript variant (focus mode):
 * layout, prompt authorship, plans, lifecycle chrome, and the byte-for-byte
 * guarantee for the other variants.
 *
 * Mounts the shipping AgentSessionTranscriptList so the variant plumbing
 * (variant context + derived turn meta) is exercised end to end rather than
 * asserting against re-implemented render classes.
 *
 * Reasoning is no longer a per-thought disclosure on this path — thoughts,
 * tool steps and interim notes fold into one work block, whose contract lives in
 * `AgentSessionWorkBlock.test.mjs`. The identity row and code-block chrome live
 * in `AgentSessionTranscriptList.conversationChrome.test.mjs`; shared jsdom
 * setup, ambient-formatting pins, and render helpers live in the harness they
 * all import.
 *
 * The byte-for-byte tests at the bottom are the important ones: `conversation`
 * is purely additive, so `default` and `compactPreview` markup for the same
 * transcript must be byte-identical to the markup captured before the variant
 * existed. See the harness for how that fixture was produced.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  act,
  AUTHOR,
  AUTHOR_PROFILES,
  AUTHOR_TRUNCATED,
  BASELINE_MARKUP,
  baselineItems,
  cleanup,
  FIXTURE_LOCALE,
  items,
  renderRerenderableTranscript,
  renderTranscript,
} from "./AgentSessionTranscriptList.conversationHarness.mjs";

test("conversation marks the transcript container and centers a reading column", async () => {
  const { container } = await renderTranscript("conversation");
  const log = container.querySelector("[data-transcript-variant]");
  assert.ok(log, "conversation should tag its container for variant styling");
  assert.equal(log.getAttribute("data-transcript-variant"), "conversation");
  assert.match(log.className, /max-w-3xl/);
  assert.match(log.className, /gap-8/);
});

test("conversation renders the prompt as a filled right-aligned bubble with an author label", async () => {
  const { container } = await renderTranscript("conversation", {
    profiles: AUTHOR_PROFILES,
  });
  const author = container.querySelector(
    '[data-testid="transcript-user-message-author"]',
  );
  assert.ok(author, "conversation should label the prompt author");
  assert.equal(author.textContent, "Ada Lovelace");

  const row = container.querySelector(
    '[data-testid="transcript-user-message"]',
  );
  assert.match(row.className, /justify-end/);
  // Keep Buzz's native 16px chat-bubble radius while adopting the focus view's
  // soft tint, borderless surface, and `px-4 py-2` spacing.
  const bubble = row.querySelector(".rounded-2xl");
  assert.ok(bubble, "the prompt should match Buzz's chat-bubble radius");
  assert.match(bubble.className, /relative/);
  assert.match(bubble.className, /bg-muted\/60/);
  const openInChatCue = bubble.querySelector('[aria-hidden="true"]');
  assert.ok(openInChatCue, "linked prompts should retain the hover cue");
  assert.match(
    openInChatCue.className,
    /top-1\.5/,
    "the sender hover cue uses its native inset once the bubble is its containing block",
  );
  assert.doesNotMatch(
    openInChatCue.className,
    /top-3/,
    "the cue should not be pushed into the prompt text to compensate for the old containing-block bug",
  );
  assert.doesNotMatch(
    bubble.className,
    /(?<!\/)bg-muted(?!\/)/,
    "sender prompts use the lighter muted tint rather than the agent-send surface",
  );
  assert.match(bubble.className, /px-4/);
  assert.match(bubble.className, /py-2(?!\.)/);
  assert.match(
    bubble.className,
    /border-0/,
    "berd never draws a border on the user turn",
  );
  // Focus mode shows the whole prompt rather than clamping it.
  assert.doesNotMatch(bubble.className, /max-h-36/);
});

test("conversation caps the prompt bubble at a fixed measure, not a percentage", async () => {
  // berd caps the user turn with `--chat-user-message-max-width: 640px`. A
  // percentage cap re-wraps the prompt every time the cover view is resized;
  // a fixed measure holds one stable line length, which is the point of the
  // recipe. Guards against a silent revert to `max-w-[85%]`.
  const { container } = await renderTranscript("conversation");
  const column = container.querySelector(
    '[data-testid="transcript-user-message-author"]',
  ).parentElement;
  assert.match(column.className, /max-w-prompt-bubble/);
  assert.doesNotMatch(column.className, /max-w-\[\d+%\]/);
});

test("conversation never shows the trigger title as the prompt author when the sender is unresolved", async () => {
  // Regression guard. The label chain's last fallback used to be the prompt
  // item's `title`, which is a description of the trigger ("@Mention",
  // "Prompt", "Buzz event") rather than an identity. On the other variants that
  // string only seeds avatar initials, so it was harmless; the conversation
  // author row displays it, so an unresolved sender was rendered as "@Mention"
  // — a trigger presented as a person's name. With no profile for the author,
  // the row must fall back to the truncated pubkey instead.
  const { container } = await renderTranscript("conversation");
  const author = container.querySelector(
    '[data-testid="transcript-user-message-author"]',
  );
  assert.ok(author, "the author row should still render without a profile");
  assert.equal(author.textContent, AUTHOR_TRUNCATED);
  assert.doesNotMatch(
    author.textContent,
    /@Mention|Prompt|Buzz event/,
    "the trigger title must never be displayed as an author name",
  );
});

test("conversation prefers the resolved profile over both the trigger title and the pubkey", async () => {
  // The truncated-pubkey fallback must not shadow a real identity: with a
  // profile present the row shows the display name, and with only a NIP-05
  // handle it shows that.
  const { container } = await renderTranscript("conversation", {
    profiles: {
      [AUTHOR]: {
        displayName: null,
        avatarUrl: null,
        nip05Handle: "ada@example.com",
        ownerPubkey: null,
      },
    },
  });
  assert.equal(
    container.querySelector('[data-testid="transcript-user-message-author"]')
      .textContent,
    "ada@example.com",
  );
});

test("conversation renders agent messages as unboxed prose at full fidelity", async () => {
  const { container } = await renderTranscript("conversation");
  const message = container.querySelector(
    '[data-testid="transcript-assistant-message"]',
  );
  assert.ok(message, "assistant message should render");
  assert.doesNotMatch(message.innerHTML, /rounded-2xl/);
  // The shared markdown renderer is reused, so inline code still renders as code.
  assert.ok(message.querySelector("code"), "markdown/code fidelity preserved");
});

test("conversation renders the plan as a checklist card with in-place progress", async () => {
  const { container } = await renderTranscript("conversation");
  const card = container.querySelector('[data-testid="transcript-plan-item"]');
  assert.ok(card, "plan should render");
  assert.equal(card.getAttribute("data-variant"), "conversation-plan-card");
  assert.equal(
    card.querySelector('[data-testid="transcript-plan-progress"]').textContent,
    "1/3 complete",
  );
  const statuses = [...card.querySelectorAll("[data-plan-entry-status]")].map(
    (entry) => entry.getAttribute("data-plan-entry-status"),
  );
  assert.deepEqual(statuses, ["completed", "in_progress", "pending"]);
});

test("conversation updates the same plan card in place as entries advance", async () => {
  // The transcript reducer mutates the plan item in place (same item id), so the
  // card must be re-rendered rather than replaced: progress copy changes while
  // the same DOM node is retained. A new node here would mean the reader's
  // scroll position and any card-level state get thrown away on every update.
  const { container, setOverrides } =
    await renderRerenderableTranscript("conversation");
  const before = container.querySelector(
    '[data-testid="transcript-plan-item"]',
  );
  assert.equal(
    before.querySelector('[data-testid="transcript-plan-progress"]')
      .textContent,
    "1/3 complete",
  );

  const advanced = items().map((item) =>
    item.id === "plan:1"
      ? {
          ...item,
          text: "- [x] read the transcript\n- [x] write the summary\n- [ ] ship it (in progress)",
        }
      : item,
  );
  await setOverrides({ items: advanced });

  const after = container.querySelector('[data-testid="transcript-plan-item"]');
  assert.equal(after, before, "the same plan card node should be reused");
  assert.equal(
    after.querySelector('[data-testid="transcript-plan-progress"]').textContent,
    "2/3 complete",
  );
  assert.deepEqual(
    [...after.querySelectorAll("[data-plan-entry-status]")].map((entry) =>
      entry.getAttribute("data-plan-entry-status"),
    ),
    ["completed", "completed", "in_progress"],
  );
});

test("conversation quiets session boundaries and status rows into centered dividers", async () => {
  const shared = { channelId: "chan-1", sessionId: "sess-2", turnId: "turn-2" };
  const { container } = await renderTranscript("conversation", {
    items: [
      ...items(),
      {
        ...shared,
        id: "msg:user2",
        type: "message",
        renderClass: "message",
        role: "user",
        title: "Ada",
        text: "next task",
        timestamp: "2026-06-14T19:05:01.000Z",
        messageId: "event-2",
        authorPubkey: AUTHOR,
      },
      {
        ...shared,
        id: "life:status",
        type: "lifecycle",
        renderClass: "status",
        title: "Context compacted",
        text: "",
        timestamp: "2026-06-14T19:05:02.000Z",
      },
    ],
  });

  // A second session id draws the boundary rule between the two runs.
  const boundary = container.querySelector(
    '[data-testid="session-boundary-divider"]',
  );
  assert.ok(boundary, "a second session should draw a boundary");
  assert.equal(boundary.getAttribute("data-variant"), "conversation-divider");

  const status = container.querySelector(
    '[data-testid="transcript-lifecycle-item"]',
  );
  assert.ok(status, "status lifecycle row should render");
  assert.equal(status.getAttribute("data-variant"), "conversation-divider");
  assert.match(status.textContent, /Context compacted/);
});

test("conversation keeps errors and permission gates loud", async () => {
  const shared = { channelId: "chan-1", sessionId: "sess-1", turnId: "turn-1" };
  const { container } = await renderTranscript("conversation", {
    items: [
      ...items(),
      {
        ...shared,
        id: "life:permission",
        type: "lifecycle",
        renderClass: "permission",
        title: "Permission requested",
        text: "write src/main.rs\nOptions: Allow, Deny",
        outcome: "Approved (once)",
        timestamp: "2026-06-14T19:00:11.000Z",
      },
      {
        ...shared,
        id: "life:error",
        type: "lifecycle",
        renderClass: "error",
        title: "Turn failed",
        text: "the harness exited",
        timestamp: "2026-06-14T19:00:12.000Z",
      },
    ],
  });

  const permission = container.querySelector(
    '[data-testid="transcript-permission-item"]',
  );
  assert.ok(permission, "permission item should render");
  assert.match(permission.className, /amber/);
  assert.equal(
    permission.getAttribute("data-variant"),
    null,
    "permission gates never route through the quiet divider",
  );
  assert.equal(
    permission
      .querySelector('[data-testid="transcript-permission-outcome"]')
      .textContent.trim(),
    "Approved (once)",
  );

  // The error shares the lifecycle testid, so pick it out by its copy.
  const error = [
    ...container.querySelectorAll('[data-testid="transcript-lifecycle-item"]'),
  ].find((node) => node.textContent.includes("Turn failed"));
  assert.ok(error, "error lifecycle item should still render");
  assert.match(error.className, /destructive/);
  assert.equal(
    error.getAttribute("data-variant"),
    null,
    "errors never route through the quiet divider",
  );
});

test("default and compactPreview markup is unchanged by the conversation variant", async () => {
  for (const variant of ["default", "compactPreview"]) {
    const { container } = await renderTranscript(variant, {
      items: baselineItems(),
    });
    assert.equal(
      container.innerHTML,
      BASELINE_MARKUP[variant],
      `${variant} transcript markup drifted`,
    );
    cleanup();
  }
});

test("the byte-for-byte baseline actually exercises every renderable item kind", async () => {
  // The fixture embeds formatted dates and times, so a zone or locale other than
  // the one it was captured in fails the comparison above for a reason that has
  // nothing to do with markup. Fail loudly and specifically instead.
  assert.equal(
    new Intl.DateTimeFormat().resolvedOptions().timeZone,
    "UTC",
    "the baseline fixture is captured in UTC — see the TZ pin at the top of this file",
  );
  assert.equal(
    new Intl.DateTimeFormat().resolvedOptions().locale,
    FIXTURE_LOCALE,
    `the baseline fixture is captured in ${FIXTURE_LOCALE} — see the locale pin at the top of this file`,
  );
  assert.equal(
    new Date("2026-06-14T19:05:00.000Z").toLocaleString(),
    "6/14/2026, 7:05:00 PM",
    "Date.prototype.toLocaleString must be locale-pinned too — the session-boundary divider uses it",
  );
  // Guards the fixture itself: a baseline that silently stopped covering a kind
  // would keep passing the comparison above while protecting nothing. Asserted
  // against `default`, which renders every kind.
  const { container } = await renderTranscript("default", {
    items: baselineItems(),
  });
  const present = (selector) => container.querySelector(selector) !== null;
  for (const [kind, selector] of [
    ["prompt", '[data-testid="transcript-user-message"]'],
    ["prompt setup", '[data-testid="transcript-turn-setup"]'],
    ["assistant message", '[data-testid="transcript-assistant-message"]'],
    ["thought", '[data-testid="transcript-thought-item"]'],
    ["plan", '[data-testid="transcript-plan-item"]'],
    ["tool", '[data-testid="transcript-tool-item"]'],
    ["permission", '[data-testid="transcript-permission-item"]'],
    ["lifecycle status/error", '[data-testid="transcript-lifecycle-item"]'],
    ["session boundary", '[data-testid="session-boundary-divider"]'],
  ]) {
    assert.ok(present(selector), `baseline must cover ${kind}`);
  }
  // Status and error share the lifecycle testid, so check both are really there.
  const lifecycleText = [
    ...container.querySelectorAll('[data-testid="transcript-lifecycle-item"]'),
  ]
    .map((node) => node.textContent)
    .join("\n");
  assert.match(lifecycleText, /Context compacted/);
  assert.match(lifecycleText, /Turn failed/);
});

/**
 * A reader's expansion survives the work blocks being regrouped underneath it.
 *
 * Work blocks are derived, not stored: `groupConversationWorkBlocks` rebuilds
 * them every render and ids each one after its first step
 * (`work-block:${items[0].id}`). `findFinalAnswerId` exempts only the LAST
 * assistant message from the block, so a second assistant message demotes the
 * first — the earlier answer becomes work, and the runs on either side of it
 * merge into one block:
 *
 *     frame 2  work-block:thought:1[...]  msg:1  work-block:thought:2[...]
 *     frame 3  work-block:thought:1[thought:1,tool:1,msg:1,thought:2,tool:2]  msg:2
 *
 * `work-block:thought:2` stops existing, so React unmounts it. With the fold
 * state held in that component, a reader who had opened the second block to read
 * its steps was folded shut by the agent posting a second message — an event
 * they did not cause and cannot predict. This drives the real list so the actual
 * regrouping runs, rather than asserting against hand-built segments.
 */
test("conversation keeps a reader's expanded work block open when a later answer regroups it", async () => {
  const shared = { channelId: "chan-1", sessionId: "sess-1", turnId: "turn-1" };
  const at = (seconds) =>
    `2026-06-14T19:00:${String(seconds).padStart(2, "0")}.000Z`;
  const thought = (id, seconds) => ({
    ...shared,
    id,
    type: "thought",
    renderClass: "thought",
    title: "Thinking",
    text: `reasoning ${id}`,
    timestamp: at(seconds),
  });
  const tool = (id, seconds) => ({
    ...shared,
    id,
    type: "tool",
    renderClass: "shell",
    title: id,
    toolName: "shell",
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "ok",
    isError: false,
    timestamp: at(seconds),
    startedAt: at(seconds),
    completedAt: at(seconds),
    descriptor: {
      renderClass: "shell",
      label: "Ran command",
      preview: id,
      source: "shell",
      groupKey: "shell:command",
    },
  });
  const answer = (id, seconds) => ({
    ...shared,
    id,
    type: "message",
    renderClass: "message",
    role: "assistant",
    title: "Test Agent",
    text: `answer ${id}`,
    timestamp: at(seconds),
  });

  const firstRun = [thought("thought:1", 1), tool("tool:1", 2)];
  const frame1 = [...firstRun, answer("msg:1", 3)];
  const frame2 = [...frame1, thought("thought:2", 4), tool("tool:2", 5)];
  const frame3 = [...frame2, answer("msg:2", 6)];

  const { container, setOverrides } = await renderRerenderableTranscript(
    "conversation",
    { items: frame1 },
  );
  const blocks = () => [
    ...container.querySelectorAll('[data-testid="transcript-work-block"]'),
  ];
  const summaries = () => [
    ...container.querySelectorAll(
      '[data-testid="transcript-work-block-summary"]',
    ),
  ];
  const openCount = () =>
    summaries().filter((node) => node.getAttribute("aria-expanded") === "true")
      .length;

  await setOverrides({ items: frame2 });
  assert.equal(
    blocks().length,
    2,
    "the demoted-answer frame should show two separate work blocks",
  );
  assert.equal(openCount(), 0, "both blocks start folded once work finished");

  // The reader opens the SECOND block — the one the merge destroys.
  await act(async () => {
    summaries()[1].click();
  });
  assert.equal(openCount(), 1, "the reader's click should open that block");

  await setOverrides({ items: frame3 });
  assert.equal(
    blocks().length,
    1,
    "the second answer should merge the runs into one block",
  );
  assert.equal(
    openCount(),
    1,
    "the merged block must stay open — the reader asked to see those steps, and an agent posting again is not a reason to fold them away",
  );
  // The steps they were reading are actually on screen, not merely a block
  // reporting itself open.
  const stepText = [
    ...container.querySelectorAll('[data-testid="transcript-work-block-step"]'),
  ]
    .map((node) => node.textContent)
    .join("\n");
  assert.match(
    stepText,
    /tool:2/,
    "the reader's steps should still be visible",
  );

  // ...and the merged block is still theirs to fold. Recording the choice
  // against only the block's first step would leave the absorbed block's stale
  // `open` entry behind, and since an open choice wins the read, the merged
  // block could never be folded again — the reader's click would do nothing.
  await act(async () => {
    summaries()[0].click();
  });
  assert.equal(
    openCount(),
    0,
    "a reader who folds the merged block must actually fold it",
  );
});
