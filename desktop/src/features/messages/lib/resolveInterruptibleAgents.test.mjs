import assert from "node:assert/strict";
import test from "node:test";

import { resolveInterruptibleAgents } from "./resolveInterruptibleAgents.ts";

const LOCAL = "a".repeat(64);
const REMOTE = "b".repeat(64);

test("includes an active relay agent owned by another user", () => {
  assert.deepEqual(
    resolveInterruptibleAgents(
      [REMOTE],
      [],
      [{ name: "Shared Debug", pubkey: REMOTE }],
    ),
    [{ name: "Shared Debug", pubkey: REMOTE }],
  );
});

test("prefers the locally managed identity and omits inactive agents", () => {
  assert.deepEqual(
    resolveInterruptibleAgents(
      [LOCAL],
      [{ name: "Local Debug", pubkey: LOCAL, status: "running" }],
      [
        { name: "Relay Alias", pubkey: LOCAL },
        { name: "Inactive", pubkey: REMOTE },
      ],
    ),
    [{ name: "Local Debug", pubkey: LOCAL }],
  );
});
