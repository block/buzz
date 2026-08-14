import assert from "node:assert/strict";
import test from "node:test";

import {
  isDanglingProjectMemberPublish,
  PartialAnnouncementPublishError,
} from "./projectOwnerControl.ts";

const OWNER = "a".repeat(64);

function event(kind, id) {
  return {
    id,
    kind,
    pubkey: OWNER,
    created_at: 100,
    content: "",
    tags: [["d", "platform"]],
  };
}

// ── isDanglingProjectMemberPublish ──────────────────────────────────────────
//
// The ACP side publishes [project, repository] announcements sequentially and
// reports already-live events alongside a failure. addRepo may resume only
// from the exact "project landed, repository did not" state.

test("project-landed/repository-missing partial publish is resumable", () => {
  const error = new PartialAnnouncementPublishError("publish failed", [
    event(30621, "1".repeat(64)),
  ]);
  assert.equal(isDanglingProjectMemberPublish(error), true);
});

test("a clean failure (nothing published) is not resumable", () => {
  const error = new PartialAnnouncementPublishError("publish failed", []);
  assert.equal(isDanglingProjectMemberPublish(error), false);
  assert.equal(isDanglingProjectMemberPublish(new Error("boom")), false);
});

test("a failure after both events landed is not resumable", () => {
  const error = new PartialAnnouncementPublishError("publish failed", [
    event(30621, "1".repeat(64)),
    event(30617, "2".repeat(64)),
  ]);
  assert.equal(isDanglingProjectMemberPublish(error), false);
});
