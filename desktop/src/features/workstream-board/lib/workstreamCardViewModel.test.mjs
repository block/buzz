import assert from "node:assert/strict";
import test from "node:test";

import { buildWorkstreamCardViewModel } from "./workstreamCardViewModel.ts";

const VALID_CARD_CONTENT = [
  "```buzz-workstream-card",
  JSON.stringify({
    version: 1,
    synopsis: "Shipping the canvas card slice.",
    orchestrator: { pubkey: "loganj-pubkey", name: "Logan" },
    assignees: [{ pubkey: "alice-pubkey", name: "Alice" }],
  }),
  "```",
].join("\n");

test("reports loading while the canvas query is in flight", () => {
  const viewModel = buildWorkstreamCardViewModel({
    canvasContent: undefined,
    isLoading: true,
    isError: false,
  });
  assert.deepEqual(viewModel, { status: "loading" });
});

test("degrades to unavailable when the canvas fetch errors", () => {
  const viewModel = buildWorkstreamCardViewModel({
    canvasContent: undefined,
    isLoading: false,
    isError: true,
  });
  assert.deepEqual(viewModel, { status: "unavailable" });
});

test("degrades to unavailable when the canvas has no content", () => {
  const viewModel = buildWorkstreamCardViewModel({
    canvasContent: null,
    isLoading: false,
    isError: false,
  });
  assert.deepEqual(viewModel, { status: "unavailable" });
});

test("degrades to unavailable when the canvas content fails to parse (card-local failure)", () => {
  const viewModel = buildWorkstreamCardViewModel({
    canvasContent: "```buzz-workstream-card\nnot valid json\n```",
    isLoading: false,
    isError: false,
  });
  assert.deepEqual(viewModel, { status: "unavailable" });
});

test("degrades to unavailable for an unknown-version card without surfacing a global error", () => {
  const viewModel = buildWorkstreamCardViewModel({
    canvasContent: [
      "```buzz-workstream-card",
      JSON.stringify({ version: 2, synopsis: "x", orchestrator: "y" }),
      "```",
    ].join("\n"),
    isLoading: false,
    isError: false,
  });
  assert.deepEqual(viewModel, { status: "unavailable" });
});

test("returns a ready card when the canvas parses successfully", () => {
  const viewModel = buildWorkstreamCardViewModel({
    canvasContent: VALID_CARD_CONTENT,
    isLoading: false,
    isError: false,
  });

  assert.equal(viewModel.status, "ready");
  assert.equal(viewModel.card.synopsis, "Shipping the canvas card slice.");
  assert.deepEqual(viewModel.card.orchestrator, {
    pubkey: "loganj-pubkey",
    name: "Logan",
  });
  assert.deepEqual(viewModel.card.assignees, [
    { pubkey: "alice-pubkey", name: "Alice" },
  ]);
});

test("loading takes priority over content even if content happens to be malformed", () => {
  const viewModel = buildWorkstreamCardViewModel({
    canvasContent: "garbage",
    isLoading: true,
    isError: false,
  });
  assert.deepEqual(viewModel, { status: "loading" });
});
