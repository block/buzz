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

test("iOS standalone detection and Safari scheme", async () => {
  const { isIosStandalone, safariSchemeUrl } = await import("./externalLinks.ts");
  const win = (ua, standalone, mm = false) => ({ navigator: { userAgent: ua, standalone, platform: "iPhone", maxTouchPoints: 5 }, matchMedia: () => ({ matches: mm }) });
  const IOS = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15";
  assert.equal(isIosStandalone(win(IOS, true)), true);
  assert.equal(isIosStandalone(win(IOS, false, true)), true);
  assert.equal(isIosStandalone(win(IOS, false)), false);
  assert.equal(isIosStandalone(win("Mozilla/5.0 (Linux; Android 14)", false, true)), false);
  assert.equal(safariSchemeUrl("https://example.com/a?b=1"), "x-safari-https://example.com/a?b=1");
  assert.equal(safariSchemeUrl("mailto:x@y.z"), null);
});

test("iOS version parsing", async () => {
  const { iosMajorVersion } = await import("./externalLinks.ts");
  assert.equal(iosMajorVersion("Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X)"), 18);
  assert.equal(iosMajorVersion("Mozilla/5.0 (iPhone; CPU iPhone OS 16_7_2 like Mac OS X)"), 16);
  assert.equal(iosMajorVersion("Mozilla/5.0 (Macintosh) Version/26.0 Safari/605.1.15"), 26);
});
