import assert from "node:assert/strict";
import test from "node:test";

import { projectChannelRequestApproval } from "./projectChannelRequestApproval.ts";

const OWNER = "a".repeat(64);
const VIEWER = "b".repeat(64);
const OTHER_AGENT = "c".repeat(64);

const project = {
  legacy: false,
  owner: OWNER,
};

test("controlled Project-owner agent requests preserve delegated signing", () => {
  assert.deepEqual(
    projectChannelRequestApproval(project, VIEWER, undefined, OWNER),
    { ownerControlAgentPubkey: OWNER },
  );
});

test("human owner and home admins approve without delegated signing", () => {
  assert.deepEqual(
    projectChannelRequestApproval(project, OWNER, undefined, OTHER_AGENT),
    {},
  );
  assert.deepEqual(
    projectChannelRequestApproval(project, VIEWER, "admin", OTHER_AGENT),
    {},
  );
});

test("unrelated managed-agent requests still require Project authority", () => {
  assert.equal(
    projectChannelRequestApproval(project, VIEWER, "member", OTHER_AGENT),
    null,
  );
  assert.equal(
    projectChannelRequestApproval(
      { ...project, legacy: true },
      VIEWER,
      "admin",
      OWNER,
    ),
    null,
  );
});
