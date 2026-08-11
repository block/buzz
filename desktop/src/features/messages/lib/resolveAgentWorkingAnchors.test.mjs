import assert from "node:assert/strict";
import { describe, it } from "node:test";

import { resolveAgentWorkingAnchors } from "./resolveAgentWorkingAnchors.ts";

const AGENT =
  "aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111aaaa1111";
const HUMAN =
  "bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222bbbb2222";

describe("resolveAgentWorkingAnchors", () => {
  it("returns empty when no agents are working", () => {
    assert.deepEqual(
      resolveAgentWorkingAnchors(
        [
          {
            id: "m1",
            createdAt: 1,
            isAgent: false,
            reactions: [
              {
                emoji: "👀",
                count: 1,
                users: [{ pubkey: AGENT, displayName: "A", avatarUrl: null }],
              },
            ],
          },
        ],
        [],
      ),
      [],
    );
  });

  it("anchors on messages with 👀 from a working agent", () => {
    const anchors = resolveAgentWorkingAnchors(
      [
        {
          id: "prompt",
          createdAt: 1,
          isAgent: false,
          reactions: [
            {
              emoji: "👀",
              count: 1,
              users: [{ pubkey: AGENT, displayName: "A", avatarUrl: null }],
            },
          ],
        },
        {
          id: "other",
          createdAt: 2,
          isAgent: false,
          reactions: [],
        },
      ],
      [AGENT],
    );
    assert.deepEqual(anchors, [
      { messageId: "prompt", agentPubkeys: [AGENT] },
    ]);
  });

  it("falls back to newest human message when 👀 has not landed", () => {
    const anchors = resolveAgentWorkingAnchors(
      [
        { id: "old", createdAt: 1, isAgent: false, reactions: [] },
        { id: "prompt", createdAt: 3, isAgent: false, reactions: [] },
        { id: "bot", createdAt: 4, isAgent: true, reactions: [] },
      ],
      [AGENT],
    );
    assert.deepEqual(anchors, [
      { messageId: "prompt", agentPubkeys: [AGENT] },
    ]);
  });
});
