import assert from "node:assert/strict";
import test from "node:test";

const { selectSearchHighlightRouteState } = await import(
  "./searchHighlightRouteState.ts"
);

const searchHighlight = {
  activationId: "activation",
  messageId: "message",
  query: "mentions",
};

test("selects valid transient search highlight state", () => {
  assert.deepEqual(
    selectSearchHighlightRouteState({ state: { searchHighlight } }),
    searchHighlight,
  );
});

test("target cleanup without highlight state preserves the selection", () => {
  assert.equal(selectSearchHighlightRouteState({ state: {} }), undefined);
});

test("ordinary navigation explicitly clears the selection", () => {
  assert.equal(
    selectSearchHighlightRouteState({ state: { searchHighlight: null } }),
    null,
  );
});

test("ignores malformed router state", () => {
  assert.equal(
    selectSearchHighlightRouteState({
      state: { searchHighlight: { messageId: "message", query: "mentions" } },
    }),
    undefined,
  );
});
