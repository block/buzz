import assert from "node:assert/strict";
import test from "node:test";

import {
  acknowledgeNativeTeamSnapshotError,
  consumeNativeTeamSnapshotError,
  markNativeTeamSnapshotErrorDisplayed,
  requestNativeTeamSnapshotError,
} from "./nativeTeamSnapshotError.ts";

test("native team snapshot error remains pending until the route acknowledges display", () => {
  consumeNativeTeamSnapshotError();
  const acknowledgements = [];

  assert.equal(
    requestNativeTeamSnapshotError(
      { id: "native-error", message: "Unreadable snapshot." },
      (id) => acknowledgements.push(id),
    ),
    true,
  );
  assert.deepEqual(consumeNativeTeamSnapshotError(), {
    id: "native-error",
    message: "Unreadable snapshot.",
  });
  assert.deepEqual(acknowledgements, []);
  assert.equal(markNativeTeamSnapshotErrorDisplayed("native-error"), true);
  assert.deepEqual(acknowledgements, []);

  assert.equal(acknowledgeNativeTeamSnapshotError("native-error"), true);
  assert.deepEqual(acknowledgements, ["native-error"]);
  assert.equal(consumeNativeTeamSnapshotError(), null);
});
