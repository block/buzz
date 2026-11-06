import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { isPlausibleReadMarker } from "./readState/readStateFormat.ts";

// Execute the production callback and resolver, with their state dependencies
// injected. Importing the complete hook also starts unrelated client modules.
const source = ts.createSourceFile(
  "useUnreadChannels.ts",
  readFileSync(new URL("./useUnreadChannels.ts", import.meta.url), "utf8"),
  ts.ScriptTarget.Latest,
  true,
);
let callback;
const resolverSource = ts.createSourceFile(
  "channelReadMarker.ts",
  readFileSync(new URL("./channelReadMarker.ts", import.meta.url), "utf8"),
  ts.ScriptTarget.Latest,
  true,
);
const resolver = resolverSource.statements.find(
  (node) =>
    ts.isFunctionDeclaration(node) &&
    node.name?.text === "resolveChannelReadMarkerUnix",
);
function visit(node) {
  if (
    ts.isVariableDeclaration(node) &&
    node.name.getText(source) === "markAllChannelsRead"
  ) {
    callback = node.initializer.arguments[0];
  }
  ts.forEachChild(node, visit);
}
visit(source);
assert.ok(callback);
assert.ok(resolver);
const code = ts.transpileModule(
  `${resolver.getText(resolverSource).replace(/^export /, "")}\nglobalThis.markAll = (${callback.getText(source)});`,
  { compilerOptions: { target: ts.ScriptTarget.ES2022 } },
).outputText;

function markAll(observedLatest) {
  const now = 10_000;
  let marker = now - 3_600;
  let retained;
  let synchronized;
  let bumps = 0;
  const forced = { general: true };
  const context = vm.createContext({
    Date: { now: () => now * 1_000 },
    isPlausibleReadMarker,
    unreadChannelIdsRef: { current: new Set(["general"]) },
    forcedUnreadRef: { current: forced },
    getEffectiveTimestamp: () => marker,
    markContextRead: (_channel, timestamp) => {
      marker = Math.max(marker, timestamp);
    },
    observedPersistence: {
      latestForChannel: () => observedLatest,
      syncMarkers: (_channels, markers) => {
        synchronized = Array.from(markers, ([channel, timestamp]) => [
          channel,
          timestamp,
        ]);
      },
      clearAll: (channels) => {
        retained = Array.from(channels);
      },
    },
    pubkey: "self",
    forcedUnreadStore: {
      write: (_pubkey, state) => assert.deepEqual(state, {}),
    },
    bumpLatestVersion: () => {
      bumps += 1;
    },
  });
  vm.runInContext(code, context);
  context.markAll();
  assert.equal(bumps, 1);
  assert.deepEqual(forced, {});
  return { marker, retained, synchronized, now };
}

test("mark all advances an old marker while retaining a future observed event", () => {
  const result = markAll(10_000 + 365 * 24 * 60 * 60);
  assert.equal(result.marker, result.now);
  assert.deepEqual(result.synchronized, [["general", result.now]]);
  assert.deepEqual(result.retained, ["general"]);
  for (const createdAt of [9_401, 9_500, 9_600, 9_700, 9_999]) {
    assert.ok(createdAt <= result.marker, "legitimate messages are now read");
  }
});

test("mark all covers the gesture time when observed evidence is old", () => {
  const result = markAll(9_900);
  assert.equal(result.marker, result.now);
  assert.deepEqual(result.retained, []);
});

test("mark all advances a forced-unread channel without observed evidence", () => {
  const result = markAll(undefined);
  assert.equal(result.marker, result.now);
  assert.deepEqual(result.synchronized, [["general", result.now]]);
});

test("mark all keeps the existing policy for a plausible observed timestamp", () => {
  const result = markAll(10_030);
  assert.equal(result.marker, 10_030);
  assert.deepEqual(result.retained, []);
});
