import assert from "node:assert/strict";
import test from "node:test";

import { extractWorkDriveOwnerConfirmation } from "./ownerConfirmation.ts";

const payload = {
  type: "switchboard_workdrive_profile",
  tenant_id: "tenant",
  channel_id: "5f8584fb-46fd-43c7-984d-4c25cbf79ea6",
  owner_pubkey: "a".repeat(64),
  capability_profile: "transcript_upload",
  upgrade_connection_id: "connection",
  scopes: [
    "WorkDrive.users.READ",
    "WorkDrive.team.READ",
    "WorkDrive.teamfolders.READ",
    "WorkDrive.files.READ",
    "ZohoFiles.files.READ",
    "WorkDrive.files.CREATE",
  ],
  actions: ["workdrive.files.upload"],
};

const message = (value) =>
  `Approve this bounded connection upgrade.\n\n\`\`\`buzz:owner-confirmation\n${JSON.stringify(value)}\n\`\`\``;

test("accepts the exact create-only WorkDrive profile", () => {
  assert.deepEqual(
    extractWorkDriveOwnerConfirmation(message(payload)),
    payload,
  );
});

test("rejects added operations and fields", () => {
  assert.equal(
    extractWorkDriveOwnerConfirmation(
      message({
        ...payload,
        actions: ["workdrive.files.upload", "workdrive.files.delete"],
      }),
    ),
    null,
  );
  assert.equal(
    extractWorkDriveOwnerConfirmation(
      message({ ...payload, callback_url: "https://evil.test" }),
    ),
    null,
  );
});

test("rejects any changed scope set", () => {
  assert.equal(
    extractWorkDriveOwnerConfirmation(
      message({ ...payload, scopes: payload.scopes.slice(1) }),
    ),
    null,
  );
});
