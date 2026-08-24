import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

test("thread replies trust the relay-provided aux closure", async () => {
  const source = await readFile(
    new URL("./useThreadReplies.ts", import.meta.url),
    "utf8",
  );
  assert.doesNotMatch(
    source,
    /withThreadAux|fetchStructuralAuxForMessages|fetchAuxEventsByReference/,
  );
  assert.match(source, /replies\.push\(\.\.\.response\.events\)/);
});
