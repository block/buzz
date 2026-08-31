/**
 * Untrusted catalog-browse avatars must never fire a network image request.
 *
 * The community catalog projects publisher-controlled `avatarUrl` values into
 * member/persona rows. Radix's `AvatarImage` resolves its loading status by
 * assigning `image.src` on a `new window.Image()`, so a browsed row would fetch
 * up to 64 attacker-chosen hosts — handing the viewer's IP and browse timing to
 * publishers — before the user adds anything. `referrerPolicy` does not stop the
 * request itself. `ProfileAvatar untrusted` must render the initials/icon
 * placeholder with zero remote fetch; a trusted local `avatarDataUrl` still
 * renders because it carries no network origin.
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

// Radix builds a detached `new window.Image()` and assigns `.src` to probe
// load status; that assignment is the actual network request. Spy on it so the
// test observes the fetch rather than post-load DOM (which never mounts in
// jsdom because the probe never fires `load`).
const imageSrcAssignments = [];

class SpyImage {
  constructor() {
    this.complete = false;
    this.naturalWidth = 0;
    this._src = "";
  }
  addEventListener() {}
  removeEventListener() {}
  set src(value) {
    this._src = value;
    imageSrcAssignments.push(value);
  }
  get src() {
    return this._src;
  }
}

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  dom.window.Image = SpyImage;
  globalThis.Image = SpyImage;
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  imageSrcAssignments.length = 0;
});

after(() => dom.window.close());

let React;
let render;
let act;
let ProfileAvatar;

before(async () => {
  React = (await import("react")).default;
  ({ render, act } = await import("@testing-library/react"));
  ({ ProfileAvatar } = await import("./ProfileAvatar.tsx"));
});

const PUBLISHER_URL = "https://attacker.example/beacon.png";

const networkAssignments = () =>
  imageSrcAssignments.filter((src) => /^https?:/i.test(src));

async function renderAvatar(props) {
  await act(async () => {
    render(React.createElement(ProfileAvatar, props));
  });
}

test("untrusted avatar fires no network image request", async () => {
  await renderAvatar({
    avatarUrl: PUBLISHER_URL,
    label: "Mallory",
    untrusted: true,
  });

  assert.deepEqual(networkAssignments(), []);
});

test("a trusted avatar still fetches the publisher URL", async () => {
  // Reversal witness: the guard is what suppresses the fetch. Without
  // `untrusted`, the same URL is requested — the exact leak Carl flagged.
  await renderAvatar({ avatarUrl: PUBLISHER_URL, label: "Mallory" });

  assert.deepEqual(networkAssignments(), [PUBLISHER_URL]);
});

test("untrusted avatar still renders a locally cached data URL", async () => {
  // A trusted, locally cached data URL carries no network origin, so it must
  // keep rendering even while the remote fetch is blocked.
  const dataUrl = "data:image/png;base64,AA";
  await renderAvatar({
    avatarUrl: PUBLISHER_URL,
    avatarDataUrl: dataUrl,
    label: "Mallory",
    untrusted: true,
  });

  assert.deepEqual(networkAssignments(), []);
  assert.deepEqual(imageSrcAssignments, [dataUrl]);
});

test("untrusted avatar renders an inline data: avatarUrl (emoji avatar)", async () => {
  // Emoji avatars persist as an inline `data:image/svg+xml` value in
  // `avatarUrl`, not a hosted URL. Blocking it isn't privacy — a `data:` URL
  // makes zero network requests — it's a regression that drops every emoji
  // avatar in catalog browse to initials. Under `untrusted`, an inline `data:`
  // scheme must still render.
  const emojiDataUrl =
    "data:image/svg+xml,%3Csvg%20xmlns='http://www.w3.org/2000/svg'/%3E";
  await renderAvatar({
    avatarUrl: emojiDataUrl,
    label: "Mallory",
    untrusted: true,
  });

  assert.deepEqual(networkAssignments(), []);
  assert.deepEqual(imageSrcAssignments, [emojiDataUrl]);
});
