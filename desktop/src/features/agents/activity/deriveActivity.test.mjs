import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  deriveActivityFromObserverFrames,
  inferActivityRoots,
  normalizeActivityPath,
} from "./deriveActivity.ts";

const BASE_T = Date.parse("2026-08-17T12:00:00.000Z");
const OPTS = { agentId: "agent-pubkey-1", agentName: "Fizz", agentIndex: 0 };

function ts(offsetMs = 0) {
  return new Date(BASE_T + offsetMs).toISOString();
}

function frame(overrides = {}) {
  return {
    seq: 1,
    timestamp: ts(0),
    kind: "acp_read",
    agentIndex: 0,
    channelId: "chan-1",
    sessionId: "sess-1",
    turnId: "turn-1",
    payload: null,
    ...overrides,
  };
}

function sessionUpdateFrame(seq, offsetMs, update, frameOverrides = {}) {
  return frame({
    seq,
    timestamp: ts(offsetMs),
    payload: {
      jsonrpc: "2.0",
      method: "session/update",
      params: { sessionId: "sess-1", update },
    },
    ...frameOverrides,
  });
}

function toolCallFrame(seq, offsetMs, update = {}, frameOverrides = {}) {
  return sessionUpdateFrame(
    seq,
    offsetMs,
    {
      sessionUpdate: "tool_call",
      toolCallId: update.toolCallId ?? `tool-${seq}`,
      status: "pending",
      ...update,
    },
    frameOverrides,
  );
}

function toolCallUpdateFrame(seq, offsetMs, update = {}, frameOverrides = {}) {
  return sessionUpdateFrame(
    seq,
    offsetMs,
    {
      sessionUpdate: "tool_call_update",
      status: "completed",
      ...update,
    },
    frameOverrides,
  );
}

function narrationFrame(seq, offsetMs, text, frameOverrides = {}) {
  return sessionUpdateFrame(
    seq,
    offsetMs,
    {
      sessionUpdate: "agent_message_chunk",
      content: { type: "text", text },
    },
    frameOverrides,
  );
}

describe("deriveActivityFromObserverFrames", () => {
  it("fans out an edit tool call into one event per location", () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        toolCallFrame(1, 0, {
          toolCallId: "call-1",
          title: "apply_patch",
          kind: "edit",
          locations: [{ path: "/repo/src/a.ts" }, { path: "/repo/src/b.ts" }],
        }),
      ],
      { ...OPTS, roots: ["/repo"] },
    );

    assert.equal(events.length, 2);
    // First-ever touches of create-eligible edits promote to "c".
    assert.deepEqual(
      events.map((event) => event.kind),
      ["c", "c"],
    );
    assert.deepEqual(
      events.map((event) => event.path),
      ["src/a.ts", "src/b.ts"],
    );
    assert.equal(events[0].sourceToolCallId, "call-1");
    assert.equal(events[0].sourceSeq, 1);
    assert.equal(events[0].sourceTurnId, "turn-1");
    assert.equal(events[0].t, BASE_T);
  });

  it("suppresses create promotion when the path was read first", () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        toolCallFrame(1, 0, {
          toolCallId: "read-1",
          kind: "read",
          locations: [{ path: "/repo/src/a.ts" }],
        }),
        toolCallFrame(2, 1_000, {
          toolCallId: "edit-1",
          kind: "edit",
          locations: [{ path: "/repo/src/a.ts" }],
        }),
      ],
      { ...OPTS, roots: ["/repo"] },
    );

    assert.deepEqual(
      events.map((event) => event.kind),
      ["r", "w"],
    );
  });

  it("keeps delete and move as plain writes", () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        toolCallFrame(1, 0, {
          kind: "delete",
          locations: [{ path: "/repo/old.txt" }],
        }),
        toolCallFrame(2, 1_000, {
          kind: "move",
          locations: [{ path: "/repo/new.txt" }],
        }),
      ],
      { ...OPTS, roots: ["/repo"] },
    );

    assert.deepEqual(
      events.map((event) => event.kind),
      ["w", "w"],
    );
  });

  it("falls back to path arguments when locations are missing", () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        toolCallFrame(1, 0, {
          kind: "read",
          rawInput: { path: "src/main.rs" },
        }),
      ],
      { ...OPTS, roots: [] },
    );

    assert.equal(events.length, 1);
    assert.equal(events[0].kind, "r");
    assert.equal(events[0].path, "src/main.rs");
  });

  it("treats execute locations as read touches and pathless execute as status", () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        toolCallFrame(1, 0, {
          kind: "execute",
          title: "developer__shell",
          locations: [{ path: "/repo/src/x.ts" }],
        }),
        toolCallFrame(2, 1_000, {
          kind: "execute",
          title: "developer__shell",
          rawInput: { command: "ls -la" },
        }),
      ],
      { ...OPTS, roots: ["/repo"] },
    );

    assert.deepEqual(
      events.map((event) => event.kind),
      ["r", "s"],
    );
    assert.equal(events[0].path, "src/x.ts");
    assert.equal(events[1].path, undefined);
  });

  it("sends think and fetch tool calls to status events", () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        toolCallFrame(1, 0, { kind: "think", title: "reasoning" }),
        toolCallFrame(2, 1_000, { kind: "fetch", title: "http_get" }),
      ],
      OPTS,
    );

    assert.deepEqual(
      events.map((event) => event.kind),
      ["s", "s"],
    );
  });

  it('maps flat tool names when the adapter classifies everything as "other"', () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        toolCallFrame(1, 0, {
          kind: "other",
          title: "write",
          rawInput: { path: "notes.md", content: "hi" },
        }),
        toolCallFrame(2, 1_000, {
          kind: "other",
          title: "edit",
          rawInput: { path: "todo.md", before: "a", after: "b" },
        }),
        toolCallFrame(3, 2_000, {
          kind: "other",
          title: "tree",
          rawInput: { path: "src" },
        }),
      ],
      OPTS,
    );

    // write is create-eligible (first touch → c); edit never is; tree reads.
    assert.deepEqual(
      events.map((event) => [event.kind, event.path]),
      [
        ["c", "notes.md"],
        ["w", "todo.md"],
        ["r", "src"],
      ],
    );
  });

  it("maps shell with locations to reads and without to status", () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        toolCallFrame(1, 0, {
          title: "developer__shell",
          locations: [{ path: "/repo/Cargo.toml" }],
        }),
        toolCallFrame(2, 1_000, {
          title: "developer__shell",
          rawInput: { command: "cargo test", cwd: "/repo" },
        }),
      ],
      { ...OPTS, roots: ["/repo"] },
    );

    assert.deepEqual(
      events.map((event) => event.kind),
      ["r", "s"],
    );
    // shell never parses paths out of its arguments (cwd would be noise).
    assert.equal(events[1].path, undefined);
  });

  it("maps text_editor commands through the developer command table", () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        toolCallFrame(1, 0, {
          title: "developer__text_editor",
          rawInput: { command: "write", path: "/repo/a.ts" },
        }),
        toolCallFrame(2, 1_000, {
          title: "developer__text_editor",
          rawInput: { command: "view", path: "/repo/b.ts" },
        }),
      ],
      { ...OPTS, roots: ["/repo"] },
    );

    assert.deepEqual(
      events.map((event) => event.kind),
      ["c", "r"],
    );
  });

  it("renders labels by translating the extension prefix to dot form", () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        toolCallFrame(1, 0, {
          title: "developer__shell",
          rawInput: { command: "ls" },
        }),
        toolCallFrame(2, 1_000, { title: "bare_tool" }),
        toolCallFrame(3, 2_000, { toolCallId: "call-3" }),
      ],
      OPTS,
    );

    assert.equal(events[0].label, "developer.shell");
    assert.equal(events[1].label, "bare_tool");
    // No title at all falls back to the tool call id.
    assert.equal(events[2].label, "call-3");
  });

  it("emits a single status event for unknown tools without paths", () => {
    const { events } = deriveActivityFromObserverFrames(
      [toolCallFrame(1, 0, { title: "mystery_tool", rawInput: { q: "x" } })],
      OPTS,
    );

    assert.equal(events.length, 1);
    assert.equal(events[0].kind, "s");
    assert.equal(events[0].label, "mystery_tool");
  });

  it("merges locations arriving only on the completion update", () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        toolCallFrame(10, 0, {
          toolCallId: "call-x",
          title: "apply_patch",
          kind: "edit",
        }),
        toolCallUpdateFrame(20, 5_000, {
          toolCallId: "call-x",
          locations: [{ path: "/repo/src/store.rs" }],
        }),
      ],
      { ...OPTS, roots: ["/repo"] },
    );

    assert.equal(events.length, 1);
    assert.equal(events[0].kind, "c");
    assert.equal(events[0].path, "src/store.rs");
    // Timestamp and seq come from the first frame that mentioned the call.
    assert.equal(events[0].t, BASE_T);
    assert.equal(events[0].sourceSeq, 10);
  });

  it("dedupes repeated paths within one tool call", () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        toolCallFrame(1, 0, {
          kind: "read",
          locations: [{ path: "/repo/a.ts" }, { path: "/repo/a.ts" }],
        }),
      ],
      { ...OPTS, roots: ["/repo"] },
    );

    assert.equal(events.length, 1);
  });

  it("attaches preceding narration as intent", () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        narrationFrame(1, 0, "Let me fix"),
        narrationFrame(2, 100, "the store."),
        toolCallFrame(3, 1_000, {
          kind: "edit",
          locations: [{ path: "/repo/store.rs" }],
        }),
        toolCallFrame(4, 2_000, {
          kind: "edit",
          locations: [{ path: "/repo/other.rs" }],
        }),
      ],
      { ...OPTS, roots: ["/repo"] },
    );

    assert.equal(events[0].intent, "Let me fix the store.");
    // Narration is consumed by the first tool call after it.
    assert.equal(events[1].intent, undefined);
  });

  it("does not bleed narration across turns", () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        narrationFrame(1, 0, "Planning the fix.", { turnId: "turn-1" }),
        toolCallFrame(
          2,
          1_000,
          { kind: "edit", locations: [{ path: "/repo/a.ts" }] },
          { turnId: "turn-2" },
        ),
      ],
      { ...OPTS, roots: ["/repo"] },
    );

    assert.equal(events.length, 1);
    assert.equal(events[0].intent, undefined);
  });

  it("compacts long narration to trailing sentences under the cap", () => {
    const longSentence = `This is a very long opening sentence ${"x".repeat(220)}.`;
    const { events } = deriveActivityFromObserverFrames(
      [
        narrationFrame(1, 0, `${longSentence} Now patch the store.`),
        toolCallFrame(2, 1_000, {
          kind: "edit",
          locations: [{ path: "/repo/store.rs" }],
        }),
      ],
      { ...OPTS, roots: ["/repo"] },
    );

    assert.equal(events[0].intent, "Now patch the store.");
  });

  it("hard-cuts a single oversized sentence with a leading ellipsis", () => {
    const oversized = `word ${"y".repeat(300)}`;
    const { events } = deriveActivityFromObserverFrames(
      [
        narrationFrame(1, 0, oversized),
        toolCallFrame(2, 1_000, {
          kind: "edit",
          locations: [{ path: "/repo/store.rs" }],
        }),
      ],
      { ...OPTS, roots: ["/repo"] },
    );

    assert.ok(events[0].intent.startsWith("…"));
    assert.ok(events[0].intent.length <= 240);
  });

  it("returns a single agent stamped from the first frame", () => {
    const { agents } = deriveActivityFromObserverFrames(
      [
        frame({ seq: 1, kind: "turn_started", timestamp: ts(0) }),
        toolCallFrame(2, 5_000, {
          kind: "read",
          locations: [{ path: "/repo/a.ts" }],
        }),
      ],
      { ...OPTS, roots: ["/repo"] },
    );

    assert.equal(agents.length, 1);
    assert.equal(agents[0].id, "agent-pubkey-1");
    assert.equal(agents[0].name, "Fizz");
    assert.equal(agents[0].spawnT, BASE_T);
    assert.equal(agents[0].doneT, undefined);
    assert.ok(agents[0].color.length > 0);
  });

  it("ignores non-acp_read frames and unrelated session updates", () => {
    const { events } = deriveActivityFromObserverFrames(
      [
        frame({ seq: 1, kind: "turn_started" }),
        frame({
          seq: 2,
          kind: "acp_write",
          payload: { method: "session/update" },
        }),
        sessionUpdateFrame(3, 100, { sessionUpdate: "plan", entries: [] }),
        frame({ seq: 4, kind: "turn_completed" }),
      ],
      OPTS,
    );

    assert.equal(events.length, 0);
  });
});

describe("inferActivityRoots", () => {
  it("returns the common directory prefix of touched absolute paths", () => {
    const roots = inferActivityRoots([
      toolCallFrame(1, 0, {
        kind: "read",
        locations: [{ path: "/Users/z/repo/src/a.ts" }],
      }),
      toolCallFrame(2, 1_000, {
        kind: "edit",
        locations: [{ path: "/Users/z/repo/lib/b.ts" }],
      }),
    ]);

    assert.deepEqual(roots, ["/Users/z/repo"]);
  });

  it("reads path arguments when locations are absent", () => {
    const roots = inferActivityRoots([
      toolCallFrame(1, 0, {
        title: "write",
        rawInput: { path: "/Users/z/repo/notes.md" },
      }),
    ]);

    assert.deepEqual(roots, ["/Users/z/repo"]);
  });

  it("falls back to no roots when the common prefix is too shallow", () => {
    const roots = inferActivityRoots([
      toolCallFrame(1, 0, {
        kind: "read",
        locations: [{ path: "/Users/z/repo/a.ts" }],
      }),
      toolCallFrame(2, 1_000, {
        kind: "read",
        locations: [{ path: "/etc/nginx/nginx.conf" }],
      }),
    ]);

    assert.deepEqual(roots, []);
  });

  it("ignores relative paths and non-tool frames", () => {
    const roots = inferActivityRoots([
      frame({ seq: 1, kind: "turn_started" }),
      toolCallFrame(2, 0, {
        kind: "read",
        rawInput: { path: "src/a.ts" },
      }),
    ]);

    assert.deepEqual(roots, []);
  });
});

describe("normalizeActivityPath", () => {
  it("relativizes against the longest matching root", () => {
    const roots = ["/repo", "/repo/packages/app"];
    assert.equal(
      normalizeActivityPath("/repo/packages/app/src/x.ts", roots),
      "src/x.ts",
    );
    assert.equal(normalizeActivityPath("/repo/README.md", roots), "README.md");
  });

  it("abbreviates paths outside all roots", () => {
    assert.equal(
      normalizeActivityPath("/etc/nginx/conf.d/site.conf", ["/repo"]),
      "~/…/nginx/conf.d/site.conf",
    );
  });

  it("maps a root itself to its basename", () => {
    assert.equal(normalizeActivityPath("/repo", ["/repo"]), "repo");
  });

  it("passes relative paths through and strips leading ./", () => {
    assert.equal(normalizeActivityPath("./src/a.ts", []), "src/a.ts");
    assert.equal(normalizeActivityPath("src/a.ts", []), "src/a.ts");
  });
});
