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

function fakeEnv(ua, standalone) {
  const calls = { open: [], href: [] };
  let loc = O + "/#/channels/abc";
  const win = {
    navigator: { userAgent: ua, standalone, platform: "iPhone", maxTouchPoints: 5 },
    matchMedia: () => ({ matches: false }),
    location: { origin: O, get href() { return loc; }, set href(v) { calls.href.push(v); loc = v; } },
    open: (...a) => calls.open.push(a),
  };
  const listeners = [];
  const doc = { defaultView: win, addEventListener: (t, fn, cap) => listeners.push({ t, fn, cap }) };
  const click = (href, extra = {}) => {
    const attrs = { href };
    const a = { target: "", getAttribute: (k) => attrs[k] ?? null, setAttribute: (k, v) => (attrs[k] = v) };
    const ev = { type: "click", button: 0, target: { closest: () => a }, prevented: false, stopped: false,
      preventDefault() { this.prevented = true; }, stopImmediatePropagation() { this.stopped = true; }, ...extra };
    for (const l of listeners) if (l.t === ev.type) l.fn(ev);
    return ev;
  };
  return { win, doc, calls, click, listeners };
}

test("guard: plain Safari tab opens the link in a new tab and blocks the in-app side panel", async () => {
  const { installExternalLinkGuard } = await import("./externalLinks.ts");
  const env = fakeEnv("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)", false);
  installExternalLinkGuard(env.doc);
  assert.ok(env.listeners.every((l) => l.cap === true));
  const ev = env.click("https://www.google.com/recaptcha/admin?x=1");
  assert.equal(ev.prevented, true);
  assert.equal(ev.stopped, true); // React's link-panel handler never runs
  assert.deepEqual(env.calls.open, [["https://www.google.com/recaptcha/admin?x=1", "_blank", "noopener,noreferrer"]]);
  assert.deepEqual(env.calls.href, []);
});

test("guard: iOS 17+ installed PWA hands the link's own URL (not location) to Safari", async () => {
  const { installExternalLinkGuard } = await import("./externalLinks.ts");
  const env = fakeEnv("Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)", true);
  installExternalLinkGuard(env.doc);
  const ev = env.click("https://example.org/page?q=2");
  assert.equal(ev.stopped, true);
  assert.deepEqual(env.calls.href, ["x-safari-https://example.org/page?q=2"]);
  assert.deepEqual(env.calls.open, []);
});

test("guard: in-app routes and modifier clicks are left alone", async () => {
  const { installExternalLinkGuard } = await import("./externalLinks.ts");
  const env = fakeEnv("Mozilla/5.0 (Linux; Android 14)", false);
  installExternalLinkGuard(env.doc);
  assert.equal(env.click("/#/agents").prevented, false);
  const mod = env.click("https://example.net/", { metaKey: true });
  assert.equal(mod.prevented, false);
  assert.deepEqual(env.calls.open, []);
});
