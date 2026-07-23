import assert from "node:assert/strict";
import test from "node:test";

import { resolveSnapshotSharedBy } from "./snapshotSharedBy.ts";

const SIGNER = "a".repeat(64);
const ATTRIBUTED_AUTHOR = "b".repeat(64);

test("snapshot provenance resolves the raw signer profile", () => {
  const label = resolveSnapshotSharedBy(
    { signerPubkey: SIGNER },
    {
      [SIGNER]: {
        avatarUrl: null,
        displayName: "Signer name",
        nip05Handle: null,
      },
      [ATTRIBUTED_AUTHOR]: {
        avatarUrl: null,
        displayName: "Spoofed actor name",
        nip05Handle: null,
      },
    },
  );

  assert.equal(label, "Signer name");
});

test("snapshot provenance is omitted when the raw signer is unavailable", () => {
  assert.equal(resolveSnapshotSharedBy({}, {}), undefined);
});
