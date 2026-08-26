import assert from "node:assert/strict";
import test from "node:test";

const { createSearchHighlightNavigation, parseSearchHighlightNavigation } =
  await import("./searchHighlightNavigation.ts");

test("creates trimmed transient state with a unique activation id", () => {
  const first = createSearchHighlightNavigation("message", "  Mentions  ");
  const second = createSearchHighlightNavigation("message", "Mentions");

  assert.deepEqual(
    { messageId: first.messageId, query: first.query },
    { messageId: "message", query: "Mentions" },
  );
  assert.notEqual(first.activationId, second.activationId);
});

test("does not create highlight state for an empty query", () => {
  assert.equal(createSearchHighlightNavigation("message", "  "), undefined);
  assert.equal(
    createSearchHighlightNavigation("message", undefined),
    undefined,
  );
});

test("parses only complete highlight navigation state", () => {
  const state = {
    activationId: "activation",
    messageId: "message",
    query: "mentions",
  };

  assert.deepEqual(parseSearchHighlightNavigation(state), state);
  assert.equal(
    parseSearchHighlightNavigation({ messageId: "message", query: "mentions" }),
    null,
  );
  assert.equal(parseSearchHighlightNavigation(null), null);
});
