import assert from "node:assert/strict";
import test from "node:test";

import { parseBestieListActionsFromMessage } from "./parseBestieListActions.ts";

test("parses fenced bestie-list add actions", () => {
  const content = `Sure — I'll track that.

\`\`\`bestie-list
{"op":"add","items":[{"kind":"todo","text":"Ship phase 2"},{"kind":"reminder","text":"Ping JM","dueAt":1700000000}]}
\`\`\`
`;
  const actions = parseBestieListActionsFromMessage(content);
  assert.equal(actions.length, 1);
  assert.equal(actions[0].op, "add");
  assert.equal(actions[0].items.length, 2);
  assert.equal(actions[0].items[0].text, "Ship phase 2");
  assert.equal(actions[0].items[1].dueAt, 1700000000);
});

test("parses complete/remove ops and ignores free-form chat", () => {
  assert.deepEqual(
    parseBestieListActionsFromMessage("Just chatting about todos"),
    [],
  );
  const actions = parseBestieListActionsFromMessage(
    '```bestie-list\n{"op":"complete","id":"abc"}\n```',
  );
  assert.deepEqual(actions, [{ id: "abc", op: "complete" }]);
});

test("parses whole-message JSON with op", () => {
  const actions = parseBestieListActionsFromMessage(
    '{"op":"add","items":[{"kind":"todo","text":"Solo"}]}',
  );
  assert.equal(actions[0].items[0].text, "Solo");
});

test("parses ISO dueAt strings on reminder adds", () => {
  const actions = parseBestieListActionsFromMessage(
    '```bestie-list\n{"op":"add","items":[{"kind":"reminder","text":"ISO","dueAt":"2026-09-26T18:00:00.000Z"}]}\n```',
  );
  assert.equal(actions[0].items[0].dueAt, Math.floor(Date.parse("2026-09-26T18:00:00.000Z") / 1000));
});
