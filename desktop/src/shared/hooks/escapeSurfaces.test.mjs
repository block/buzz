import assert from "node:assert/strict";
import test from "node:test";

import {
  acquireEscapeSurface,
  hasActiveEscapeSurface,
} from "./escapeSurfaces.ts";

test("no surfaces means background shortcuts may act", () => {
  assert.equal(hasActiveEscapeSurface(), false);
});

test("acquire and release track open surfaces", () => {
  const releaseA = acquireEscapeSurface();
  assert.equal(hasActiveEscapeSurface(), true);

  const releaseB = acquireEscapeSurface();
  releaseA();
  assert.equal(
    hasActiveEscapeSurface(),
    true,
    "one surface closing must not release the other's claim",
  );

  releaseB();
  assert.equal(hasActiveEscapeSurface(), false);
});

test("double release cannot corrupt the count", () => {
  const releaseA = acquireEscapeSurface();
  releaseA();
  releaseA();
  assert.equal(hasActiveEscapeSurface(), false);

  const releaseB = acquireEscapeSurface();
  assert.equal(
    hasActiveEscapeSurface(),
    true,
    "a leaked double-release must not mask a genuinely open surface",
  );
  releaseB();
  assert.equal(hasActiveEscapeSurface(), false);
});
