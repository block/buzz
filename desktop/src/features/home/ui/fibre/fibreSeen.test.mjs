import assert from "node:assert/strict";
import test from "node:test";

import { fibreDotState, fibreSeenStorageKey } from "./fibreSeen.ts";

test("fibreDotState is unseen until opened", () => {
  assert.equal(fibreDotState({ updatedAt: 10 }, undefined), "unseen");
});

test("fibreDotState is updated when the fibre changes after it was seen", () => {
  assert.equal(fibreDotState({ updatedAt: 20 }, 10), "updated");
});

test("fibreDotState is empty when seen at the current updatedAt", () => {
  assert.equal(fibreDotState({ updatedAt: 20 }, 20), null);
});

test("seen storage key is scoped to relay and pubkey", () => {
  assert.equal(
    fibreSeenStorageKey("wss://relay.example", "abc"),
    "buzz-fibre-seen.v1:wss://relay.example:abc",
  );
  assert.equal(
    fibreSeenStorageKey(undefined, undefined),
    "buzz-fibre-seen.v1:local:anonymous",
  );
});
