import assert from "node:assert/strict";
import test from "node:test";

import { resolveHostLabel } from "./resolveHostLabel.ts";

const HOST = "c28e6260aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa34af";

test("resolveHostLabel prefers knownHosts over truncated pubkey", () => {
  assert.equal(
    resolveHostLabel({
      hostPubkey: HOST,
      knownHosts: { [HOST]: "agentbox" },
    }),
    "agentbox",
  );
});

test("resolveHostLabel matches knownHosts keys case-insensitively", () => {
  assert.equal(
    resolveHostLabel({
      hostPubkey: HOST.toUpperCase(),
      knownHosts: { [HOST]: "agentbox" },
    }),
    "agentbox",
  );
});

test("resolveHostLabel falls back to truncated pubkey", () => {
  const label = resolveHostLabel({ hostPubkey: HOST });
  assert.match(label, /^c28e6260/i);
  assert.ok(label.includes("…") || label.includes("..."));
});

test("resolveHostLabel ignores blank known labels", () => {
  const label = resolveHostLabel({
    hostPubkey: HOST,
    knownHosts: { [HOST]: "   " },
  });
  assert.notEqual(label, "   ");
  assert.match(label, /^c28e6260/i);
});
