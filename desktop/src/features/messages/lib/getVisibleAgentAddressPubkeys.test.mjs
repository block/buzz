import assert from "node:assert/strict";
import test from "node:test";

import { getVisibleAgentAddressPubkeys } from "./getVisibleAgentAddressPubkeys.ts";

const DARIA = "a".repeat(64);
const RIZZ = "b".repeat(64);

test("hides an address prefix already represented by an inline mention", () => {
  assert.deepEqual(
    getVisibleAgentAddressPubkeys("@Daria please review this", [DARIA], {
      daria: DARIA,
    }),
    [],
  );
});

test("keeps a tag-backed prefix when its inline mention was deleted", () => {
  assert.deepEqual(
    getVisibleAgentAddressPubkeys("please review this", [DARIA], {
      daria: DARIA,
    }),
    [DARIA],
  );
});

test("filters only addressed agents that are present inline", () => {
  assert.deepEqual(
    getVisibleAgentAddressPubkeys(
      "@Daria please pair with someone",
      [DARIA, RIZZ],
      {
        daria: DARIA,
        rizz: RIZZ,
      },
    ),
    [RIZZ],
  );
});
