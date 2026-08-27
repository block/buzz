/**
 * Work-block behaviour for work that was abandoned mid-flight, the rail's
 * per-kind presentation, and the re-render cost a streaming block must not pay.
 * The live/finished contracts live in `AgentSessionWorkBlock.test.mjs`; the
 * shared rig is `AgentSessionWorkBlockTestRig.mjs`.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  noteStep,
  relayStep,
  renderBlock,
  step,
  thoughtStep,
} from "./AgentSessionWorkBlockTestRig.mjs";

// ── Orphaned work ────────────────────────────────────────────────────────────

/**
 * Reopened history must not present as work in progress.
 *
 * A tool's `executing` status is written when the step starts and never revised
 * if the agent dies first, so an abandoned step keeps it forever. In this block
 * that status is not a per-row detail — one `running` entry makes the whole
 * block active, which suppresses the folded summary line, keeps the rail
 * expanded and pulses a bullet. Scrolling back to a crashed turn therefore
 * showed the reader live work indefinitely (Codex P2 on #6536).
 */
test("a block whose running step has no live session folds like finished work", async () => {
  const view = await renderBlock(
    [step("a"), step("b", { status: "executing", completedAt: null })],
    { liveTurnId: null, streamingItemId: null },
  );

  const summary = view.summary();
  assert.ok(
    summary,
    "history gets its folded summary line — the orphaned status must not suppress it",
  );
  assert.match(
    summary.textContent,
    /2 steps$/,
    "an abandoned step is not known to have failed, so the count stays neutral",
  );
  assert.equal(await view.settleToStepCount(0), 0, "the rail folds away");
  // Then OPEN it and look. Asserting "nothing pulses" on the folded block would
  // be vacuous: the fold unmounts every row, so no glyph exists to carry a
  // pulse class and the assertion would hold against a build where abandoned
  // steps pulse forever — the exact Codex finding. The reader's complaint is
  // about what they see when they scroll back and expand, so that is where the
  // assertion belongs.
  await view.expand();
  assert.equal(view.stepCount(), 2, "the expanded rail has rows to inspect");
  assert.deepEqual(
    view.glyphStates(),
    ["settled", "settled"],
    "the abandoned step reads as settled, not running",
  );
  assert.deepEqual(
    view.pulseStates(),
    [],
    "nothing pulses when nothing is running",
  );
});

test("the same items still hold the block open while the turn is live", async () => {
  // The paired half: this is what makes the gate meaningful rather than a
  // blanket "never trust executing". Identical items, live turn.
  const view = await renderBlock(
    [step("a"), step("b", { status: "executing", completedAt: null })],
    { liveTurnId: "turn-1", streamingItemId: null },
  );

  assert.equal(view.summary(), null, "a live block has no folded line");
  assert.equal(view.stepCount(), 2, "the rail stays open");
  assert.deepEqual(view.glyphStates(), ["settled", "running"]);
  assert.deepEqual(view.pulseStates(), ["running"], "live work pulses");
});

test("an agent live on a later turn does not resurrect an earlier turn's abandoned step", async () => {
  // The reason the signal is a turn id and not a boolean: a restarted agent is
  // live, but not on this turn, and a global flag would keep this step spinning.
  const view = await renderBlock(
    [step("a"), step("b", { status: "executing", completedAt: null })],
    { liveTurnId: "turn-2", streamingItemId: null },
  );

  assert.ok(
    view.summary(),
    "this turn is history even though the agent is busy",
  );
  assert.equal(await view.settleToStepCount(0), 0);
  // Expanded, for the same reason as above: a folded rail has no glyph to pulse,
  // so the negative has to be taken on rows that exist.
  await view.expand();
  assert.deepEqual(view.glyphStates(), ["settled", "settled"]);
  assert.deepEqual(view.pulseStates(), []);
});

test("a block live when the session ends folds instead of spinning forever", async () => {
  const { act } = await import("@testing-library/react");
  // The bug as the reader meets it live: the agent dies mid-step, so the item
  // never reaches a terminal status and the only thing that changes is that no
  // turn is live any more.
  const live = [
    step("a"),
    step("b", { status: "executing", completedAt: null }),
  ];
  const view = await renderBlock(live, {
    liveTurnId: "turn-1",
    streamingItemId: "b",
  });
  assert.equal(view.stepCount(), 2, "live: the rail is open");
  assert.equal(view.summary(), null);

  // Session gone. The items are BYTE-IDENTICAL — only liveness changed.
  await act(async () => {
    view.stream(live, null, null);
  });

  assert.ok(view.summary(), "the block settles when its session goes away");
  assert.match(view.summary().textContent, /2 steps$/);
  // Taken HERE, before the fold finishes: the rail is still mounted for the
  // settle frame, so the glyph that was pulsing a moment ago still exists and
  // can be asked whether it stopped. Once `settleToStepCount(0)` has run there
  // are no rows left and the same assertion proves nothing.
  assert.equal(view.stepCount(), 2, "the rail is still mounted to inspect");
  assert.deepEqual(view.glyphStates(), ["settled", "settled"]);
  assert.deepEqual(
    view.pulseStates(),
    [],
    "the pulse stops the moment the session goes away, not when the fold finishes",
  );
  assert.equal(await view.settleToStepCount(0), 0);
});

// ── The gap between two turns ─────────────────────────────────────────────────

/**
 * A finished block must stay folded through the gap before the next turn shows
 * anything.
 *
 * The rendered half of the `buildConversationTurnMeta` gap contract (see
 * `agentSessionConversationMeta.test.mjs`). The meta test proves the hints are
 * right; this proves the reader sees the consequence, because the symptom was
 * never a wrong id — it was a settled 6-step block re-opening, dropping to its
 * last three steps behind a "previous steps" disclosure, and then folding back,
 * on every single turn.
 *
 * Six steps rather than two on purpose: the live window only applies above
 * three, so a smaller block would hide the loudest part of the regression.
 */
test("a finished block stays folded while the next turn has started but shown nothing", async () => {
  const { act } = await import("@testing-library/react");
  const items = ["a", "b", "c", "d", "e", "f"].map((id) => step(id));

  // Finished: nothing live, so the block is folded to its summary line.
  const view = await renderBlock(items, {
    liveTurnId: null,
    streamingItemId: null,
  });
  assert.match(view.summary().textContent, /6 steps$/);
  assert.equal(await view.settleToStepCount(0), 0, "it starts folded");

  // The next turn starts. It owns liveness (turn-2) and, having emitted nothing
  // renderable, contributes no streaming item — which is exactly what the fixed
  // `latestTurnId`/`streamingIdForTail` pair reports for this frame.
  await act(async () => {
    view.stream(items, null, "turn-2");
  });

  assert.ok(
    view.summary(),
    "the folded summary line survives the next turn starting",
  );
  assert.match(view.summary().textContent, /6 steps$/);
  assert.equal(view.stepCount(), 0, "the rail does not re-open");
  assert.equal(
    view.previousSteps(),
    null,
    "and the live window's previous-steps disclosure never appears",
  );
  assert.deepEqual(view.pulseStates(), []);
});

test("an orphaned step keeps its own row detail rather than gaining an interrupted marker", async () => {
  // Deliberate: we do not know what happened to the step, so it renders as the
  // neutral step it is with whatever it recorded. A visible "interrupted"
  // treatment would be a design addition, not part of this fix.
  const view = await renderBlock(
    [step("a"), step("b", { status: "executing", completedAt: null })],
    { liveTurnId: null, streamingItemId: null },
  );
  await view.expand();
  await view.settleToStepCount(2);

  assert.deepEqual(
    view.glyphStates(),
    ["settled", "settled"],
    "no third state was invented for an abandoned step",
  );
  assert.equal(
    view.qa('[data-testid="transcript-tool-item"]').length,
    2,
    "both steps still render through the normal tool presenter",
  );
});

test("the rail bullet masks the spine with the drawer surface colour", async () => {
  // The bullet has to mask the spine passing behind it, and the mask must match
  // the surface the transcript is drawn on. A mask in any other colour shows as
  // a disc of the wrong shade around every bullet (berd's BOT-1599).
  const view = await renderBlock([step("a"), step("b")]);
  await view.expand();
  const bullet = view.q("[data-step-state]");
  assert.match(bullet.className, /\bbg-background\b/);
  assert.match(bullet.className, /\bring-background\b/);
  assert.match(bullet.className, /\brounded-full\b/);
});

test("the spine is drawn for every step except the last", async () => {
  const view = await renderBlock([step("a"), step("b"), step("c")]);
  await view.expand();
  const spines = view.qa(".w-px");
  assert.equal(
    spines.length,
    2,
    "three steps means two connecting segments; a trailing spine would dangle",
  );
});

test("thinking renders as a rail row with its own glyph, not a nested disclosure", async () => {
  const view = await renderBlock([thoughtStep("thought:1"), step("a")]);
  await view.expand();
  const thought = view.q('[data-testid="transcript-work-block-thought"]');
  assert.ok(thought, "reasoning renders on the rail");
  assert.match(thought.textContent, /weighing the options/);
  assert.equal(
    view.q('[data-testid="transcript-thought-disclosure"]'),
    null,
    "the block is already one disclosure — a thought must not add a second",
  );
});

/**
 * An interim note is progress, not a second reply.
 *
 * Conversation mode renders the turn's answer as standalone prose. A rail note
 * is the same item type, so routing it through that presenter would render an
 * authored agent turn nested inside a muted step row—the reader would see
 * the agent apparently reply twice, once inside the work it was doing. berd
 * draws the same line: its `progress` entry is a plain rail row.
 *
 * The suppression is done on this side (a dedicated prose body) rather than by
 * reaching into the message presenter, so #6720 keeps one rule for what a
 * message looks like.
 */
test("an interim note renders as rail prose with no identity row", async () => {
  const view = await renderBlock([
    step("a"),
    noteStep("msg:interim", "checked the three call sites"),
  ]);
  await view.expand();

  const note = view.q('[data-testid="transcript-work-block-note"]');
  assert.ok(note, "the note renders on the rail");
  assert.match(note.textContent, /checked the three call sites/);

  assert.ok(
    view.q('[data-testid="transcript-assistant-identity"]') === null,
    "an avatar + name row inside a muted step reads as a second reply",
  );
  assert.ok(
    view.q('[data-testid="transcript-assistant-message"]') === null,
    "the note must not go through the message presenter at all",
  );
});

/**
 * A relay post is a step, not a reply — the same rule as an interim note,
 * reached by a different route.
 *
 * A note is an assistant *message* the block re-presents as prose. A relay post
 * is a *tool call* that classifies as `renderClass: "message"`, so it renders
 * through `CompactMessageSummary`: 28px avatar, bordered speech bubble,
 * timestamp, delivery-receipt button. That is right in the activity feed, where
 * a posted message is a destination to open; on the rail it makes the agent
 * appear to reply in the middle of its own work — and it did, in the seeded
 * browser preview, which is where this was caught.
 *
 * Suppressing it needs the presentation signal rather than the transcript
 * variant: the same relay step OUTSIDE a block in this variant keeps its
 * bubble, which the next test pins.
 */
test("a relay post on the rail is a plain step, with no bubble or avatar", async () => {
  const view = await renderBlock([step("a"), relayStep("relay:1")]);
  await view.expand();

  assert.equal(
    view.stepCount(),
    2,
    "the relay post takes its own rail row, like any other step",
  );
  assert.equal(
    view.qa('[data-work-block-entry="tool"]').length,
    2,
    "a relay post is a tool step — it is something the agent did",
  );
  assert.equal(
    view.q('[data-testid="transcript-tool-message-preview"]'),
    null,
    "a speech bubble inside a muted step reads as the agent replying mid-work",
  );
  assert.equal(
    view.q('[data-testid="transcript-agent-sent-avatar"]'),
    null,
    "no identity avatar on the rail",
  );
  assert.equal(
    view.q('[data-testid="transcript-sent-message-context-button"]'),
    null,
    "no delivery receipt on the rail",
  );

  // It is still a real, expandable tool row carrying its command.
  const rows = view.qa('[data-testid="transcript-tool-item"]');
  assert.equal(rows.length, 2, "both steps render as tool rows");
  assert.ok(
    rows[1].querySelector("details"),
    "the relay step keeps the ordinary step disclosure so its args stay reachable",
  );
  assert.match(
    rows[1].textContent,
    /Sent|posted the findings/,
    "the row still says what the step was",
  );
});

/**
 * The other half of the branch: outside a block the bubble is correct and must
 * survive. Without this, suppressing the bubble everywhere in the conversation
 * variant would pass the test above.
 */
test("the same relay post outside a work block keeps its message bubble", async () => {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { QueryClient, QueryClientProvider } = await import(
    "@tanstack/react-query"
  );
  const { createMemoryHistory, createRootRoute, createRouter, RouterProvider } =
    await import("@tanstack/react-router");
  const { AgentSessionTranscriptVariantProvider } = await import(
    "./agentSessionTranscriptContext.ts"
  );
  const { TranscriptActivityItem } = await import(
    "./activityRenderClasses/TranscriptActivityItem.tsx"
  );

  // `gcTime: 0`: React Query's default is 300000ms, and this is the one test
  // that actually drives the bubble presenter's `useQuery`, so its query arms a
  // five-minute gc timer at teardown. node:test waits that timer out before
  // exiting — this file's tests sum to ~2s but the wall was ~303s, all passing,
  // with no failing assertion to point at the cause.
  const queryClient = new QueryClient({
    defaultOptions: { queries: { gcTime: 0, retry: false } },
  });
  const rootRoute = createRootRoute({
    component: () =>
      createElement(
        QueryClientProvider,
        { client: queryClient },
        createElement(
          AgentSessionTranscriptVariantProvider,
          { value: "conversation" },
          createElement(TranscriptActivityItem, {
            agentAvatarUrl: null,
            agentName: "Agent",
            agentPubkey: "pk",
            item: relayStep("relay:1"),
          }),
        ),
      ),
  });
  const router = createRouter({
    history: createMemoryHistory({ initialEntries: ["/"] }),
    routeTree: rootRoute,
  });
  await router.load();
  const view = render(createElement(RouterProvider, { router }));

  const bubble = view.container.querySelector(
    '[data-testid="transcript-tool-message-preview"]',
  );
  assert.ok(
    bubble,
    "outside a block a posted message is a destination the reader can open — the bubble stays",
  );
  assert.match(bubble.className, /relative/);
  assert.match(
    bubble.className,
    /pr-4/,
    "focus-mode agent sends keep a full 16px inset on the right",
  );
  assert.doesNotMatch(
    bubble.className,
    /max-h-36/,
    "focus mode shows the full sent message rather than clamping it",
  );
  assert.match(
    bubble.parentElement.className,
    /pr-9/,
    "the left-side bubble keeps exterior space mirroring the sender avatar gutter",
  );
  assert.match(
    bubble.className,
    /(?<!\/)bg-muted(?!\/)/,
    "agent sends use the stronger muted surface to distinguish them from sender prompts",
  );
});

test("the rail glyph is chosen by kind, and prose kinds share the speech bubble", async () => {
  // The exhaustive switch is the point: a note that fell through to the tool
  // branch would wear a wrench and read as something the agent ran.
  const view = await renderBlock([
    thoughtStep("thought:1"),
    noteStep("msg:interim"),
    step("a"),
    step("b", { isError: true, status: "failed" }),
  ]);
  await view.expand();

  const glyphClass = (kind, index = 0) => {
    const rows = view.qa(`[data-work-block-entry="${kind}"]`);
    const icon = rows[index].querySelector("svg");
    return icon.getAttribute("class") ?? "";
  };

  // lucide stamps each icon with a `lucide-<kebab-name>` class, so the glyph
  // identity is readable from the DOM without reaching into the icon modules.
  assert.match(glyphClass("thought"), /lucide-message-circle/);
  assert.match(
    glyphClass("note"),
    /lucide-message-circle/,
    "prose is the agent talking, whether it is reasoning or a note",
  );
  assert.match(glyphClass("tool", 0), /lucide-wrench/);
  assert.match(
    glyphClass("tool", 1),
    /lucide-circle/,
    "a failed step is a filled dot, not a wrench",
  );
});

test("the rail bullet is never red, whatever the step's outcome", async () => {
  // A failure is carried by glyph shape and by the folded line's count. Tinting
  // the bullet would make one bad step read as an alarm across the whole run.
  //
  // The running step keeps this block live, so the rail is already open — which
  // is also the only state in which a running bullet can be observed at all.
  const view = await renderBlock(
    [
      step("a"),
      step("b", { isError: true, status: "failed" }),
      step("c", { status: "executing", completedAt: null }),
    ],
    { streamingItemId: "c" },
  );

  assert.deepEqual(
    view.glyphStates(),
    ["settled", "failed", "running"],
    "all three outcomes are on screen",
  );
  for (const bullet of view.qa("[data-step-state]")) {
    assert.ok(
      !/\b(text|bg|ring)-(destructive|red)/.test(bullet.className),
      `rail bullet for ${bullet.getAttribute("data-step-state")} must stay muted`,
    );
    assert.match(bullet.className, /\btext-muted-foreground\b/);
  }
});

/**
 * berd brightens rail prose with `usePrimaryText={open}`. Here the brightening
 * is unconditional, and this test records why that is not a divergence: a closed
 * block unmounts its rows rather than dimming them, so there is no state in
 * which rail prose is on screen and NOT in an open block. A `primaryText` prop
 * would have an unreachable false branch.
 */
test("rail prose is primary text, and a closed block has no prose on screen at all", async () => {
  const { act } = await import("@testing-library/react");
  const live = [
    thoughtStep("thought:1"),
    noteStep("msg:interim"),
    step("b", { status: "executing", completedAt: null }),
  ];
  const view = await renderBlock(live, { streamingItemId: "b" });

  const prose = () =>
    view.qa(
      '[data-testid="transcript-work-block-thought"],[data-testid="transcript-work-block-note"]',
    );

  assert.equal(prose().length, 2, "both prose rows are on the live rail");
  for (const node of prose()) {
    assert.match(
      node.className,
      /\btext-foreground\b/,
      "prose the reader can see is primary, not muted",
    );
    assert.ok(
      !/\btext-muted-foreground\b/.test(node.className),
      "the row must not carry both colours",
    );
  }

  // Finish the turn: the block folds and takes its prose with it.
  await act(async () => {
    view.stream([thoughtStep("thought:1"), noteStep("msg:interim"), step("b")]);
  });
  assert.equal(await view.settleToStepCount(0), 0, "it folded");
  assert.equal(
    prose().length,
    0,
    "a folded block renders no prose, so there is no dimmed state to test",
  );

  // And the reader reopening it brings the same primary prose back.
  await act(async () => {
    view.summary().click();
  });
  assert.equal(await view.settleToStepCount(3), 3, "the reader reopened it");
  assert.equal(prose().length, 2);
  for (const node of prose()) {
    assert.match(node.className, /\btext-foreground\b/);
  }
});

// ── Streaming cost ───────────────────────────────────────────────────────────

/**
 * A block re-renders on every append while work streams. Unchanged steps must
 * not re-render with it: each step's presenter rebuilds compact tool summaries,
 * parses diffs and renders markdown/images, so an unmemoized step row makes a
 * long block cost O(n) of that work per appended step.
 *
 * Counted at the presenter boundary — `TranscriptActivityItem` looks its
 * presenter up in `ACTIVITY_RENDER_CLASS_PRESENTERS` on every render, so
 * swapping in a counting presenter observes exactly the work a step row
 * triggers, without reaching into React internals.
 */
async function countStepRenders(initialItems, nextItems) {
  const { createElement } = await import("react");
  const { render } = await import("@testing-library/react");
  const { ACTIVITY_RENDER_CLASS_PRESENTERS } = await import(
    "./activityRenderClasses/TranscriptActivityItem.tsx"
  );
  const { AgentSessionTranscriptTurnMetaProvider } = await import(
    "./agentSessionTranscriptContext.ts"
  );
  const { AgentSessionWorkBlockSegment } = await import(
    "./AgentSessionWorkBlock.tsx"
  );

  const renders = [];
  const original = ACTIVITY_RENDER_CLASS_PRESENTERS.shell;
  ACTIVITY_RENDER_CLASS_PRESENTERS.shell = function CountingPresenter(props) {
    renders.push(props.item.id);
    return createElement("div", null, props.item.id);
  };

  try {
    const element = (items) =>
      createElement(
        AgentSessionTranscriptTurnMetaProvider,
        {
          value: {
            liveTurnId: items[items.length - 1].turnId,
            streamingItemId: items[items.length - 1].id,
          },
        },
        createElement(AgentSessionWorkBlockSegment, {
          agentAvatarUrl: null,
          agentName: "Agent",
          agentPubkey: "pk",
          block: {
            id: "work-block:a",
            items,
            timestamp: items[0].timestamp,
          },
        }),
      );

    const view = render(element(initialItems));
    renders.length = 0;
    view.rerender(element(nextItems));
    return renders;
  } finally {
    ACTIVITY_RENDER_CLASS_PRESENTERS.shell = original;
  }
}

test("appending a step does not re-render the steps already on the rail", async () => {
  // The block is expanded (a live block with ≤3 steps shows them all), and the
  // prior steps are the SAME objects across both renders, as the transcript
  // store replaces items rather than mutating them.
  const settled = [step("a"), step("b")];
  const appended = [
    ...settled,
    step("c", { status: "executing", completedAt: null }),
  ];

  const rendered = await countStepRenders(settled, appended);

  assert.deepEqual(rendered, ["c"]);
});

test("a step that actually changed does re-render", async () => {
  // Guards the memo from being too aggressive: an executing step settling is a
  // new object for that id, and it must re-render to drop its running glyph.
  const a = step("a");
  const executing = step("b", { status: "executing", completedAt: null });
  const settled = step("b");

  const rendered = await countStepRenders([a, executing], [a, settled]);

  assert.deepEqual(rendered, ["b"]);
});
