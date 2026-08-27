/**
 * Presentation contract for the `conversation` transcript variant's *chrome*:
 * the single-agent answer treatment and the focus code-block recipe.
 *
 * Split out of `AgentSessionTranscriptList.conversation.test.mjs` to stay under
 * the repo's hard 1000-line/file ceiling (AGENTS.md). The shared jsdom setup,
 * ambient-formatting pins, and render helpers live in the harness so the two
 * suites cannot drift apart.
 */

import assert from "node:assert/strict";
import { test } from "node:test";

import {
  cleanup,
  fencedCodeItems,
  fencedCodePromptItems,
  renderTranscript,
  renderTranscriptWithCodeChrome,
} from "./AgentSessionTranscriptList.conversationHarness.mjs";

test("conversation keeps assistant prose unboxed without repeating the pane identity", async () => {
  // The activity pane is scoped to one agent and its sticky header already
  // carries that identity. Repeating avatar + name above every answer adds no
  // speaker information and visually collides with message-send tool bubbles.
  const { container } = await renderTranscript("conversation");
  assert.ok(
    container.querySelector('[data-testid="transcript-assistant-identity"]') ===
      null,
    "the single-agent pane should not repeat its header identity",
  );
  const message = container.querySelector(
    '[data-testid="transcript-assistant-message"]',
  );
  assert.ok(message, "the assistant answer should render");
  assert.doesNotMatch(message.innerHTML, /rounded-2xl/);
});

test("conversation frames fenced code with berd's header row", async () => {
  // berd puts the language in a real header row above the frame, with the copy
  // action opposite it (`code-block.tsx` CodeBlockHeader:388-402), and the code
  // itself in a 10px-radius, page-background, borderless-shadow frame
  // (:528-529). Buzz's `rounded-lg` (`--radius: 0.625rem`) is exactly berd's
  // `rounded-[0.625rem]`.
  const { container } = await renderTranscriptWithCodeChrome("conversation", {
    items: fencedCodeItems(),
  });
  const header = container.querySelector(
    '[data-testid="markdown-code-block-header"]',
  );
  assert.ok(header, "focus mode should render a code-block header row");
  // Language sits in the header, not inside the frame.
  assert.match(header.textContent, /^ts/);
  assert.match(header.className, /justify-between/);
  assert.match(header.className, /items-end/);
  assert.match(header.className, /min-h-7/);
  assert.ok(
    header.querySelector('[aria-label="Copy code block"]'),
    "the copy action is a flow sibling of the language label",
  );

  const frame = container.querySelector("pre");
  assert.ok(frame, "the code frame should render");
  assert.match(frame.className, /rounded-lg/);
  assert.match(frame.className, /bg-background/);
  assert.match(frame.className, /border-border\/80/);
  assert.doesNotMatch(
    frame.className,
    /shadow/,
    "berd's code frame carries no shadow",
  );
  // Guards against the default recipe leaking in: it uses a 16px radius, a
  // muted fill, `pr-12` to clear an absolutely-positioned copy button, and an
  // inline `borderRadius` style.
  assert.doesNotMatch(frame.className, /rounded-2xl/);
  assert.doesNotMatch(frame.className, /bg-muted/);
  assert.doesNotMatch(frame.className, /pr-12/);
  assert.equal(frame.style.borderRadius, "");
  // Line numbers come from `.code-block-lines [data-line]` in markdown.css, so
  // the frame only has to keep emitting per-line elements under that class.
  const code = frame.querySelector("code.code-block-lines");
  assert.ok(code, "the code element keeps the line-number class");
  assert.equal(code.querySelectorAll("[data-line]").length, 2);
});

test("conversation applies the code recipe to a fenced human prompt too", async () => {
  // Regression guard for a real bug quality caught. The provider was first
  // mounted inside `MessageActivity`, which only handles assistant items — the
  // user bubble returns before it, so a fence inside a prompt kept the legacy
  // 16px muted frame nested inside the new 12px bubble. The recipe is a
  // property of the *surface*, not of a role, so the provider now sits at the
  // transcript boundary and both roles inherit it.
  const { container } = await renderTranscriptWithCodeChrome("conversation", {
    items: fencedCodePromptItems(),
  });
  const bubble = container.querySelector(
    '[data-testid="transcript-user-message"]',
  );
  assert.ok(bubble, "the prompt should render");
  assert.ok(
    bubble.querySelector('[data-testid="markdown-code-block-header"]'),
    "a fence inside the prompt gets berd's header row",
  );
  const frame = bubble.querySelector("pre");
  assert.match(frame.className, /rounded-lg/);
  assert.doesNotMatch(
    frame.className,
    /rounded-2xl/,
    "the legacy 16px frame must not nest inside the 12px bubble",
  );
  assert.doesNotMatch(frame.className, /pr-12/);
});

test("the default transcript variant keeps the legacy code chrome", async () => {
  // The markdown renderer is shared with channel messages, so `focusProse` is
  // opt-in per surface. Rendering the same fenced block through the `default`
  // transcript variant must still produce the original chrome: no header row,
  // 16px radius, muted fill, and the absolutely-positioned copy button.
  //
  // This proves the *variant gate*, not the channel-message row itself — those
  // rows are covered by the markdown tests in `shared/ui/markdown`.
  const { container } = await renderTranscriptWithCodeChrome("default", {
    items: fencedCodeItems(),
  });
  // `assert.ok(x === null)` rather than `assert.equal(x, null)`: on failure the
  // latter serializes the whole matched jsdom element (and its ancestors) to
  // build a diff, which exhausts memory instead of printing the message.
  assert.ok(
    container.querySelector('[data-testid="markdown-code-block-header"]') ===
      null,
    "the default recipe has no header row",
  );
  const frame = container.querySelector("pre");
  assert.match(frame.className, /rounded-2xl/);
  assert.match(frame.className, /bg-muted\/60/);
  assert.match(frame.className, /pr-12/);
  assert.match(frame.className, /shadow-xs/);
  const copy = container.querySelector('[aria-label="Copy code block"]');
  assert.ok(copy, "the default copy button still renders");
  assert.match(copy.className, /absolute/);
});

test("no transcript variant adds a per-answer identity row", async () => {
  for (const variant of ["conversation", "default", "compactPreview"]) {
    const { container } = await renderTranscript(variant);
    assert.ok(
      container.querySelector(
        '[data-testid="transcript-assistant-identity"]',
      ) === null,
      `${variant} must not render a per-answer identity row`,
    );
    cleanup();
  }
});
