import assert from "node:assert/strict";
import test from "node:test";

import { personaSaveNotice } from "./personaSaveNotice.ts";

test("test_plain_save_notice_says_nothing_about_the_catalog", () => {
  const notice = personaSaveNotice("Helper", null);
  assert.equal(notice, "Updated Helper.");
  assert.ok(!/catalog/i.test(notice));
});

test("test_accepted_publish_notice_claims_the_catalog_has_the_edit", () => {
  assert.match(
    personaSaveNotice("Helper", "published"),
    /published it to the community catalog/,
  );
});

// The whole point of routing "Save and publish" through the strict command is
// that a queued edit must NOT be reported as published — the relay hasn't taken
// it yet, so the catalog still shows the old definition.
test("test_queued_publish_notice_does_not_claim_the_edit_is_published", () => {
  const notice = personaSaveNotice("Helper", "queued");
  assert.match(notice, /queued/);
  assert.ok(
    !/\bpublished\b/.test(notice),
    "a queued edit must not be described as published",
  );
});
