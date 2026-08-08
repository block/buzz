import assert from "node:assert/strict";
import { test } from "node:test";

import { parseSessionSnapshot } from "./documentSession.ts";

const VAULT = "/vault";

function snapshot(overrides = {}) {
  return JSON.stringify({
    activePath: `${VAULT}/a.md`,
    expandedPaths: [`${VAULT}/Notes`],
    openPaths: [`${VAULT}/a.md`, `${VAULT}/b.md`],
    vaultPath: VAULT,
    ...overrides,
  });
}

test("parses a well-formed snapshot", () => {
  assert.deepEqual(parseSessionSnapshot(snapshot(), VAULT), {
    activePath: `${VAULT}/a.md`,
    expandedPaths: [`${VAULT}/Notes`],
    openPaths: [`${VAULT}/a.md`, `${VAULT}/b.md`],
    vaultPath: VAULT,
  });
});

test("discards a snapshot from a different vault", () => {
  // Its paths do not exist here, so filtering would leave a misleading
  // half-session; dropping it is correct.
  assert.equal(parseSessionSnapshot(snapshot(), "/other-vault"), null);
});

test("drops an active path that is not open", () => {
  const parsed = parseSessionSnapshot(
    snapshot({ activePath: `${VAULT}/not-open.md` }),
    VAULT,
  );
  assert.equal(parsed.activePath, null);
  assert.deepEqual(parsed.openPaths, [`${VAULT}/a.md`, `${VAULT}/b.md`]);
});

test("returns null for missing or malformed data", () => {
  assert.equal(parseSessionSnapshot(null, VAULT), null);
  assert.equal(parseSessionSnapshot("", VAULT), null);
  assert.equal(parseSessionSnapshot("not json", VAULT), null);
  assert.equal(parseSessionSnapshot("[]", VAULT), null);
  assert.equal(parseSessionSnapshot('{"vaultPath":123}', VAULT), null);
});

test("rejects non-string path arrays rather than trusting them", () => {
  assert.equal(
    parseSessionSnapshot(snapshot({ openPaths: [1, 2] }), VAULT),
    null,
  );
  assert.equal(
    parseSessionSnapshot(snapshot({ expandedPaths: [{}] }), VAULT),
    null,
  );
  assert.equal(
    parseSessionSnapshot(snapshot({ openPaths: "not-an-array" }), VAULT),
    null,
  );
});

test("an empty session is valid", () => {
  const parsed = parseSessionSnapshot(
    snapshot({ activePath: null, expandedPaths: [], openPaths: [] }),
    VAULT,
  );
  assert.deepEqual(parsed.openPaths, []);
  assert.equal(parsed.activePath, null);
});
