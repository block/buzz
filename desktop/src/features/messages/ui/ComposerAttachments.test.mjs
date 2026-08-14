import assert from "node:assert/strict";
import test from "node:test";

import { getSnapshotKind } from "./ComposerAttachments.tsx";

const SHA256 = "a".repeat(64);

function attachment(filename) {
  return { filename, sha256: SHA256 };
}

test("getSnapshotKind recognizes .buzzteam without regressing legacy team snapshots", () => {
  assert.equal(getSnapshotKind(attachment("engineering.buzzteam")), "team");
  assert.equal(getSnapshotKind(attachment("engineering.team.json")), "team");
  assert.equal(getSnapshotKind(attachment("engineering.team.png")), "team");
});

test("getSnapshotKind rejects a .buzzteam attachment without a verified hash", () => {
  assert.equal(
    getSnapshotKind({ filename: "engineering.buzzteam", sha256: "short" }),
    null,
  );
});
