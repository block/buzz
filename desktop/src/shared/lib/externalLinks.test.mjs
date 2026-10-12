import assert from "node:assert/strict";
import test from "node:test";
import { hardenExternalAnchor, isExternalHttpUrl } from "./externalLinks.ts";

const O = "https://hulabuzz.huladesk.com";
test("external http(s) links are detected; in-app routes are not", () => {
  assert.equal(isExternalHttpUrl("https://example.com/x", O), true);
  assert.equal(isExternalHttpUrl("http://example.com", O), true);
  assert.equal(isExternalHttpUrl("https://buzz.huladesk.com/media/a.png", O), true);
  assert.equal(isExternalHttpUrl("/#/channels/abc", O), false);
  assert.equal(isExternalHttpUrl("#/agents", O), false);
  assert.equal(isExternalHttpUrl(`${O}/#/inbox`, O), false);
  assert.equal(isExternalHttpUrl("mailto:a@b.c", O), false);
  assert.equal(isExternalHttpUrl(null, O), false);
});
test("hardened anchors open in a new browsing context with noopener noreferrer", () => {
  const attrs = { href: "https://example.com", rel: "nofollow" };
  const a = { target: "", getAttribute: (k) => attrs[k] ?? null, setAttribute: (k, v) => (attrs[k] = v) };
  assert.equal(hardenExternalAnchor(a, O), true);
  assert.equal(a.target, "_blank");
  assert.deepEqual(attrs.rel.split(" ").sort(), ["noopener", "noreferrer", "nofollow"].sort());
  const inApp = { target: "", getAttribute: () => "/#/agents", setAttribute() {} };
  assert.equal(hardenExternalAnchor(inApp, O), false);
  assert.equal(inApp.target, "");
});
