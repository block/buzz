import assert from "node:assert/strict";
import test from "node:test";

import {
  mcpAppAttributedMessage,
  MCP_APP_POST_MAX_CHARS,
  mcpAppMessageText,
} from "./mcpAppMessage.ts";

test("extracts text from standard MCP content blocks", () => {
  assert.equal(
    mcpAppMessageText({
      role: "user",
      content: [
        { type: "text", text: "Create the task." },
        { type: "image", data: "ignored" },
        { type: "text", text: "Keep it in this thread." },
      ],
    }),
    "Create the task.\n\nKeep it in this thread.",
  );
});

test("accepts the legacy single text block used by existing apps", () => {
  assert.equal(
    mcpAppMessageText({
      role: "user",
      content: { type: "text", text: "Move this to review." },
    }),
    "Move this to review.",
  );
});

test("rejects messages without text", () => {
  assert.equal(
    mcpAppMessageText({
      role: "user",
      content: [{ type: "image", data: "ignored" }],
    }),
    null,
  );
});

test("collapses excessive blank lines without flattening paragraphs", () => {
  assert.equal(
    mcpAppMessageText({
      role: "user",
      content: "First paragraph.\n\n\n\n\nSecond paragraph.",
    }),
    "First paragraph.\n\nSecond paragraph.",
  );
  assert.equal(MCP_APP_POST_MAX_CHARS, 8_000);
});

test("adds durable visible MCP App attribution and sanitizes the title", () => {
  assert.equal(
    mcpAppAttributedMessage("Project\nBoard", "Moved task to Review."),
    "MCP App · Project Board\n\nMoved task to Review.",
  );
});
