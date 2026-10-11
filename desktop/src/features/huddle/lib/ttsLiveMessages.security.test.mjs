import assert from "node:assert/strict";
import test from "node:test";

import { classifySpeakableAgentText } from "./ttsLiveMessages.ts";

test("strips attachment spoiler shells in linear time", () => {
  const channelId = "security-regression";
  const url = "https://cdn.example/voice.png";
  const blankLines = "\n".repeat(10_000);
  const result = classifySpeakableAgentText(
    {
      id: "security-regression-event",
      kind: 9,
      pubkey: "agent",
      content: `before\n||\n![image](${url})${blankLines}||\nafter`,
      tags: [
        ["h", channelId],
        ["imeta", `url ${url}`, "m image/png"],
      ],
    },
    new Set(["agent"]),
    "human",
    channelId,
  );

  assert.deepEqual(result, { text: "before\nafter", reason: null });
});
