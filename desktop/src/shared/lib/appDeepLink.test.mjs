import assert from "node:assert/strict";
import test from "node:test";

import { APP_DEEP_LINK_SCHEME, toAppDeepLink } from "./appDeepLink.ts";

test("toAppDeepLink rewrites buzz:// to hulabuzz://", () => {
  assert.equal(
    toAppDeepLink("buzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32"),
    "hulabuzz://channel/580ca78b-9dae-46f3-8854-bd671853ba32",
  );
  assert.equal(
    toAppDeepLink("buzz://message?channel=abc&id=deadbeef&thread=deadbeef"),
    "hulabuzz://message?channel=abc&id=deadbeef&thread=deadbeef",
  );
});

test("toAppDeepLink leaves non-canonical hrefs alone", () => {
  assert.equal(toAppDeepLink("hulabuzz://channel/x"), "hulabuzz://channel/x");
  assert.equal(toAppDeepLink("https://example.com"), "https://example.com");
});

test("APP_DEEP_LINK_SCHEME is hulabuzz", () => {
  assert.equal(APP_DEEP_LINK_SCHEME, "hulabuzz");
});
