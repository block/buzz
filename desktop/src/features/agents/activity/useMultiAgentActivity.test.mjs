/**
 * Tests for disambiguatePathNamespaces — the multi-agent path-namespace layer.
 *
 * When agents work out of DIFFERENT workspace roots, each agent's pathful
 * events are prefixed with the basename of its first root so `src/main.rs` in
 * two repos lands on two distinct activity rows. Agents sharing a root, agents
 * with no inferred root, and pathless (status) events must pass through
 * untouched. Output arrays are aligned 1:1 with the input derivation order.
 */

import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { disambiguatePathNamespaces } from "./useMultiAgentActivity.ts";

const BASE_T = Date.parse("2026-08-17T12:00:00.000Z");

/** Minimal ActivityEvent: extra fields are optional for this pure function. */
function event(agentId, kind, path, offsetMs = 0) {
  return {
    agentId,
    t: BASE_T + offsetMs,
    kind,
    ...(path !== undefined ? { path } : {}),
  };
}

/** Minimal PerAgentDerivation-shaped object. */
function derivation(agentId, roots, events) {
  return {
    agentId,
    events,
    agents: [{ id: agentId, name: agentId, colorIndex: 0 }],
    roots,
    frameCount: events.length,
    lastSeq: events.length,
  };
}

describe("disambiguatePathNamespaces", () => {
  it("test_two_agents_different_roots_both_prefixed_and_aligned", () => {
    const a = derivation(
      "agent-a",
      ["/Users/a/buzz"],
      [
        event("agent-a", "r", "src/main.rs"),
        event("agent-a", "w", "src/lib.rs", 1),
      ],
    );
    const b = derivation(
      "agent-b",
      ["/Users/a/berd"],
      [event("agent-b", "r", "src/main.rs")],
    );

    const out = disambiguatePathNamespaces([a, b]);

    assert.equal(out.length, 2, "output aligned with input length");
    // output[0] corresponds to input[0] (agent-a), output[1] to input[1].
    assert.deepEqual(
      out[0].map((e) => e.path),
      ["buzz/src/main.rs", "buzz/src/lib.rs"],
    );
    assert.equal(out[0][0].agentId, "agent-a");
    assert.deepEqual(
      out[1].map((e) => e.path),
      ["berd/src/main.rs"],
    );
    assert.equal(out[1][0].agentId, "agent-b");
  });

  it("test_two_agents_same_root_no_prefixing", () => {
    const a = derivation(
      "agent-a",
      ["/Users/a/buzz"],
      [event("agent-a", "r", "src/main.rs")],
    );
    const b = derivation(
      "agent-b",
      ["/Users/a/buzz"],
      [event("agent-b", "w", "src/other.rs")],
    );

    const out = disambiguatePathNamespaces([a, b]);

    assert.equal(out[0][0].path, "src/main.rs");
    assert.equal(out[1][0].path, "src/other.rs");
    // No prefixing means the exact input arrays pass through.
    assert.equal(out[0], a.events);
    assert.equal(out[1], b.events);
  });

  it("test_one_rooted_one_rootless_single_distinct_root_no_prefixing", () => {
    // Only ONE distinct non-empty root exists across derivations, so there is
    // nothing to disambiguate — nobody gets prefixed, including the rooted one.
    const rooted = derivation(
      "agent-a",
      ["/Users/a/buzz"],
      [event("agent-a", "r", "src/main.rs")],
    );
    const rootless = derivation(
      "agent-b",
      [],
      [event("agent-b", "w", "notes.md")],
    );

    const out = disambiguatePathNamespaces([rooted, rootless]);

    assert.equal(out[0][0].path, "src/main.rs", "rooted agent unprefixed");
    assert.equal(out[1][0].path, "notes.md", "rootless agent unprefixed");
  });

  it("test_two_distinct_roots_plus_rootless_only_rooted_get_prefixed", () => {
    const a = derivation(
      "agent-a",
      ["/Users/a/buzz"],
      [event("agent-a", "r", "src/main.rs")],
    );
    const b = derivation(
      "agent-b",
      ["/Users/a/berd"],
      [event("agent-b", "w", "src/main.rs")],
    );
    const c = derivation("agent-c", [], [event("agent-c", "r", "todo.txt")]);

    const out = disambiguatePathNamespaces([a, b, c]);

    assert.equal(out[0][0].path, "buzz/src/main.rs");
    assert.equal(out[1][0].path, "berd/src/main.rs");
    assert.equal(out[2][0].path, "todo.txt", "rootless agent stays bare");
  });

  it("test_status_events_without_path_pass_through_unprefixed", () => {
    const statusEvent = event("agent-a", "s", undefined);
    const a = derivation(
      "agent-a",
      ["/Users/a/buzz"],
      [statusEvent, event("agent-a", "r", "src/main.rs", 1)],
    );
    const b = derivation(
      "agent-b",
      ["/Users/a/berd"],
      [event("agent-b", "s", undefined)],
    );

    const out = disambiguatePathNamespaces([a, b]);

    assert.equal(
      out[0][0].path,
      undefined,
      "status event path stays undefined",
    );
    // Pathless events are passed through as-is (same object, not cloned).
    assert.equal(out[0][0], statusEvent);
    assert.equal(
      out[0][1].path,
      "buzz/src/main.rs",
      "sibling pathful event prefixed",
    );
    assert.equal(out[1][0].path, undefined);
  });

  it("test_single_agent_never_prefixed", () => {
    const a = derivation(
      "agent-a",
      ["/Users/a/buzz"],
      [event("agent-a", "r", "src/main.rs")],
    );

    const out = disambiguatePathNamespaces([a]);

    assert.equal(out.length, 1);
    assert.equal(out[0][0].path, "src/main.rs");
    assert.equal(out[0], a.events, "single agent's events pass through as-is");
  });
});
