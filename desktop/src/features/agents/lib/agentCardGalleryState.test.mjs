import assert from "node:assert/strict";
import { describe, it } from "node:test";

// Pins the gallery's query-state boundary: a rejected `list_agent_cards`
// MUST surface as an error state — never as "No cards yet". `data` falls
// back to `[]` at the call site, so an emptiness-first branch would render
// a permissions/IPC failure as a false empty archive of paid cards.

import { agentCardGalleryViewState } from "./agentCardGalleryState.ts";

describe("agentCardGalleryViewState", () => {
  it("maps a rejected query to an error state with the message, not empty", () => {
    const state = agentCardGalleryViewState({
      isLoading: false,
      isError: true,
      error: new Error("cards directory unreadable"),
      data: undefined,
    });
    assert.deepEqual(state, {
      kind: "error",
      message: "cards directory unreadable",
    });
  });

  it("error wins even when data has fallen back to an empty array", () => {
    // react-query keeps `data: undefined` on first failure, but the call
    // site coalesces to []. Guard the exact shape that caused the bug.
    const state = agentCardGalleryViewState({
      isLoading: false,
      isError: true,
      error: new Error("ipc failure"),
      data: [],
    });
    assert.equal(state.kind, "error");
  });

  it("stringifies non-Error rejections", () => {
    const state = agentCardGalleryViewState({
      isLoading: false,
      isError: true,
      error: "permission denied",
      data: undefined,
    });
    assert.deepEqual(state, { kind: "error", message: "permission denied" });
  });

  it("null/undefined rejections still produce a message", () => {
    const state = agentCardGalleryViewState({
      isLoading: false,
      isError: true,
      error: null,
      data: undefined,
    });
    assert.deepEqual(state, { kind: "error", message: "unknown error" });
  });

  it("loading while fetching", () => {
    const state = agentCardGalleryViewState({
      isLoading: true,
      isError: false,
      error: null,
      data: undefined,
    });
    assert.deepEqual(state, { kind: "loading" });
  });

  it("no data yet (not loading, not error) stays loading, not empty", () => {
    const state = agentCardGalleryViewState({
      isLoading: false,
      isError: false,
      error: null,
      data: undefined,
    });
    assert.deepEqual(state, { kind: "loading" });
  });

  it("resolved and empty is the only path to the empty state", () => {
    const state = agentCardGalleryViewState({
      isLoading: false,
      isError: false,
      error: null,
      data: [],
    });
    assert.deepEqual(state, { kind: "empty" });
  });

  it("resolved with cards renders cards", () => {
    const state = agentCardGalleryViewState({
      isLoading: false,
      isError: false,
      error: null,
      data: [{ storedFileName: "a.png" }],
    });
    assert.deepEqual(state, { kind: "cards" });
  });
});
