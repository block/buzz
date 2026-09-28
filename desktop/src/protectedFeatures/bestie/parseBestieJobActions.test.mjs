import assert from "node:assert/strict";
import test from "node:test";

import { parseBestieJobActionsFromMessage } from "./parseBestieJobActions.ts";

test("parses confirmed bestie-job fence add", () => {
  const actions = parseBestieJobActionsFromMessage(`Ok.
\`\`\`bestie-job
{"op":"add","confirmed":true,"job":{"title":"Brief","prompt":"Go","schedule":{"kind":"daily","hour":9,"minute":0}}}
\`\`\``);
  assert.equal(actions.length, 1);
  assert.equal(actions[0].op, "add");
  assert.equal(actions[0].confirmed, true);
  assert.equal(actions[0].job.title, "Brief");
});

test("unconfirmed add becomes draft (no create)", () => {
  const actions = parseBestieJobActionsFromMessage(`\`\`\`bestie-job
{"op":"add","job":{"title":"Brief","prompt":"Go","schedule":{"kind":"daily","hour":9,"minute":0}}}
\`\`\``);
  assert.equal(actions.length, 1);
  assert.equal(actions[0].op, "draft");
  assert.equal(actions[0].job.title, "Brief");
});

test("parses explicit draft fence", () => {
  const actions = parseBestieJobActionsFromMessage(`\`\`\`bestie-job
{"op":"draft","job":{"title":"Brief","prompt":"Go","schedule":{"kind":"interval","everySeconds":600}}}
\`\`\``);
  assert.equal(actions.length, 1);
  assert.equal(actions[0].op, "draft");
  assert.equal(actions[0].job.schedule.kind, "interval");
});
