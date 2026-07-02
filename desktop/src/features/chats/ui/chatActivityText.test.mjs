import assert from "node:assert/strict";
import test from "node:test";

import {
  cleanAssistantMessageText,
  isHumanFacingAssistantText,
  toolLabel,
} from "./chatActivityText.ts";

const baseTimestamp = "2026-07-02T09:30:00.000Z";

function makeTool(overrides = {}) {
  return {
    id: "tool:1",
    type: "tool",
    title: "Tool call",
    toolName: "shell",
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "",
    isError: false,
    timestamp: baseTimestamp,
    startedAt: baseTimestamp,
    completedAt: "2026-07-02T09:30:01.000Z",
    ...overrides,
  };
}

test("cleanAssistantMessageText strips chat-mode emojis", () => {
  assert.equal(
    cleanAssistantMessageText("Let's do it! 🚀 What are we trying out?"),
    "Let's do it! What are we trying out?",
  );
});

test("cleanAssistantMessageText preserves markdown line breaks", () => {
  assert.equal(
    cleanAssistantMessageText(
      "Here's the full system:\n\n---\n\n## Desktop\n\n| Utility | Value |\n| --- | --- |\n| rounded-lg | 10px |",
    ),
    "Here's the full system:\n\n---\n\n## Desktop\n\n| Utility | Value |\n| --- | --- |\n| rounded-lg | 10px |",
  );
});

test("isHumanFacingAssistantText hides agent tool summary replies", () => {
  assert.equal(
    isHumanFacingAssistantText(
      "Replied to Kenny's message, ready to help with whatever he wants to try out.",
    ),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText(
      "Replied to Kenny confirming I'm online and ready to help.",
    ),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText(
      "Now I have all the button details across all three platforms. Let me send the reply.",
    ),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText(
      "Now I have a comprehensive picture. Let me also check the Tailwind config for any custom radius values:",
    ),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText(
      "Now let me create a new worktree from the latest main in the REPOS directory for this workspace:",
    ),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText(
      "Now let me find the agent popover menu component in the desktop app:",
    ),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText(
      "Let me look at the ProfilePopover.tsx and UserProfilePopover.tsx; these are likely the agent popover components:",
    ),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText(
      "That's the recent change. Let me look at the full diff for that commit to understand what was changed:",
    ),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText(
      "Now I have everything. Let me send the comprehensive corner radius breakdown.",
    ),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText(
      "I sent a detailed breakdown of the Buzz app's button system across all three platforms.",
    ),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText(
      "Done! I sent a comprehensive breakdown of the corner radius system covering: Desktop and mobile.",
    ),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText(
      "The message 'okay, you can do it again' is top-level, not in a thread, so it's a bit ambitious.",
    ),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText("I asked Kenny for clarification."),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText("They could mean repeat the previous request."),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText("I should ask Kenny for clarification."),
    false,
  );
  assert.equal(
    isHumanFacingAssistantText("Still here. What should we try?"),
    true,
  );
  assert.equal(
    isHumanFacingAssistantText(
      "Here's the full breakdown of buttons across all Buzz platforms: Desktop, web, and mobile each use slightly different button primitives.",
    ),
    true,
  );
});

test("toolLabel keeps command details only while shell commands run", () => {
  assert.equal(
    toolLabel(
      makeTool({
        status: "executing",
        args: { command: 'find . -name "*.md"' },
      }),
    ),
    'Running find . -name "*.md"',
  );
  assert.equal(
    toolLabel(makeTool({ args: { command: 'find . -name "*.md"' } })),
    "Ran command",
  );
});

test("toolLabel keeps search details only while searches run", () => {
  assert.equal(
    toolLabel(
      makeTool({
        toolName: "search",
        buzzToolName: "search",
        status: "executing",
        args: { query: "chat mode" },
      }),
    ),
    "Searching chat mode",
  );
  assert.equal(
    toolLabel(
      makeTool({
        toolName: "search",
        buzzToolName: "search",
        args: { query: "chat mode" },
      }),
    ),
    "Searched",
  );
});
