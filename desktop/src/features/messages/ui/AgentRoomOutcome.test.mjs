import assert from "node:assert/strict";
import test from "node:test";

import {
  buildThreadMemoryBody,
  extractAgentSourceReceipt,
  findLatestAgentOutcome,
  getThreadMemoryTargets,
} from "./AgentRoomOutcome.tsx";

const head = {
  id: "a".repeat(64),
  author: "Jay",
  body: "How should we protect the property-lead model?",
};
const answer = {
  id: "b".repeat(64),
  author: "Strategy",
  body: "Separate the operating brief from the pitch. See [model](/workspace/model.md) and https://example.com/evidence.",
  isAgent: true,
  pubkey: "1".repeat(64),
};

test("builds a reviewed thread memory with deduplicated source receipts", () => {
  const sources = extractAgentSourceReceipt({
    body: `${answer.body} https://example.com/evidence and \`desktop/src/App.tsx\``,
    tags: [["source", "/workspace/model.md", "Operating model"]],
  });
  assert.deepEqual(sources, [
    { label: "Operating model", location: "/workspace/model.md" },
    {
      label: "https://example.com/evidence",
      location: "https://example.com/evidence",
    },
    { label: "desktop/src/App.tsx", location: "desktop/src/App.tsx" },
  ]);

  assert.equal(findLatestAgentOutcome([head, answer]), answer);
  assert.deepEqual(
    getThreadMemoryTargets(
      [
        {
          pubkey: "1".repeat(64),
          name: "Strategy",
          status: "running",
          agentSource: "managed",
        },
        {
          pubkey: "2".repeat(64),
          name: "Remote",
          status: "deployed",
          agentSource: "relay",
        },
      ],
      ["1".repeat(64), "2".repeat(64)],
      [head, answer],
    ).map((agent) => agent.name),
    ["Strategy"],
  );

  const memory = buildThreadMemoryBody({
    channelId: "channel-id",
    channelName: "strategy",
    outcome: answer,
    sources,
    threadHead: head,
  });
  assert.match(memory, /## Agreed outcome/);
  assert.match(memory, /Operating model: \/workspace\/model\.md/);
  assert.match(memory, /buzz:\/\/message\?channel=channel-id&id=a{64}/);
});
