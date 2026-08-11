import assert from "node:assert/strict";
import test from "node:test";

import {
  classifyAsk,
  extractTaskLine,
  isConfigNoise,
  stripMessageNoise,
} from "./taskExtraction.ts";

test("stripMessageNoise removes markdown, links, code and JSON blobs", () => {
  const cleaned = stripMessageNoise(
    '**Two emails** went out. See [the doc](https://example.com/x) and https://example.com/raw plus `inline` and ```{"a":1}``` end.',
  );
  assert.equal(
    cleaned,
    "Two emails went out. See the doc and plus inline and end.",
  );
});

test("extractTaskLine prefers a question aimed at the reader", () => {
  const line = extractTaskLine(
    "Campaign 2 update. The numbers moved. Does the Backups list show any backup dated today, or is the newest one still the 30 July entry? More detail follows.",
  );
  assert.equal(
    line,
    "Does the Backups list show any backup dated today, or is the newest one still the 30 July entry?",
  );
});

test("extractTaskLine falls back to ask language, then first sentence", () => {
  assert.equal(
    extractTaskLine(
      "Context first sentence here. Please review your v02 proposal before Monday. Closing remark.",
    ),
    "Please review your v02 proposal before Monday.",
  );
  assert.equal(
    extractTaskLine("Engineering signed off on the desktop build. All good."),
    "Engineering signed off on the desktop build.",
  );
});

test("regression: narrative blocked + need input status is headsUp, not blocked", () => {
  const result = classifyAsk(
    "Deploy status: we are blocked + need input, review of the infra config is pending.",
  );
  assert.equal(result.type, "headsUp");
  assert.equal(result.ask, null);
  assert.equal(result.askCount, 0);
});

test("regression: discussing the decision on tag costs is headsUp, not decision", () => {
  const result = classifyAsk(
    "The team is still weighing the decision on tag costs and will confirm the budget next week.",
  );
  assert.equal(result.type, "headsUp");
  assert.equal(result.ask, null);
  assert.equal(result.askCount, 0);
});

test("questions aimed at a named third party are deprioritised", () => {
  const result = classifyAsk(
    "Status update. Should @lee take the relay migration, agreed? When is the next backup run?",
  );
  assert.equal(result.ask, "When is the next backup run?");
  // A direct @-addressed ask still classifies when it is the only candidate.
  const direct = classifyAsk("Can @lee restart the relay today?");
  assert.equal(direct.type, "question");
  assert.equal(direct.ask, "Can @lee restart the relay today?");
});

test("askCount counts distinct reader asks", () => {
  const two = classifyAsk(
    "Can you approve the budget? Also, could you review the deck?",
  );
  assert.equal(two.askCount, 2);
  const one = classifyAsk("Please review your plan.");
  assert.equal(one.askCount, 1);
  // A plain question headline still reports at least one ask.
  const plain = classifyAsk("Is the backup scheduled?");
  assert.equal(plain.askCount, 1);
  const none = classifyAsk("hello there everyone.");
  assert.equal(none.askCount, 0);
});

test("extractTaskLine truncates very long sentences on a word boundary", () => {
  const long = `Reading your message as approval to proceed on the three fixes we converged on earlier because you want it out now rather than waiting on further confirmation from the rest of the team`;
  const line = extractTaskLine(long);
  assert.ok(line.length <= 111);
  assert.ok(line.endsWith("…"));
  assert.ok(!line.includes("  "));
});

test("isConfigNoise flags config-nudge payloads only", () => {
  assert.equal(
    isConfigNoise('run codex login ```buzz:config-nudge {"agent_name":"X"}```'),
    true,
  );
  assert.equal(isConfigNoise("please review the config doc"), false);
});
