/**
 * A cached avatar must paint on its first frame.
 *
 * Radix's `AvatarImage` probes load status with a detached `new window.Image()`
 * and only mounts the visible `<img>` once that probe reports the image loaded.
 * The probe's `src` assignment is the network request, so it fetches every
 * mounted avatar regardless of the visible `<img>`'s `loading` attribute.
 * `loading="lazy"` therefore deferred nothing; in WKWebView it only made each
 * newly mounted, already-cached `<img>` reload asynchronously, so new message
 * groups showed a blank or gray avatar for a few frames (#8193).
 */

import assert from "node:assert/strict";
import { after, afterEach, before, test } from "node:test";

import { JSDOM } from "jsdom";

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

const imageSrcAssignments = [];

// A probe image that is already in the browser cache: complete as soon as its
// `src` is assigned.
class CachedImage {
  constructor() {
    this.complete = false;
    this.naturalWidth = 0;
    this._src = "";
  }
  addEventListener() {}
  removeEventListener() {}
  set src(value) {
    this._src = value;
    this.complete = true;
    this.naturalWidth = 64;
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
  dom.window.Image = CachedImage;
  globalThis.Image = CachedImage;
});

afterEach(async () => {
  const { cleanup } = await import("@testing-library/react");
  cleanup();
  imageSrcAssignments.length = 0;
});

after(() => dom.window.close());

let React;
let render;
let UserAvatar;
let ProfileAvatar;

before(async () => {
  React = (await import("react")).default;
  ({ render } = await import("@testing-library/react"));
  ({ UserAvatar } = await import("./UserAvatar.tsx"));
  ({ ProfileAvatar } = await import(
    "../../features/profile/ui/ProfileAvatar.tsx"
  ));
});

const AVATAR_URL = "https://media.example/avatar.png";

test("a cached avatar renders its image on the first render without lazy loading", () => {
  const { getByTestId } = render(
    React.createElement(UserAvatar, {
      avatarUrl: AVATAR_URL,
      displayName: "Bruce",
      testId: "avatar",
    }),
  );

  const image = getByTestId("avatar-image");
  assert.equal(image.getAttribute("src"), AVATAR_URL);
  assert.notEqual(image.getAttribute("loading"), "lazy");
});

test("a cached profile avatar renders its image on the first render without lazy loading", () => {
  const { getByTestId } = render(
    React.createElement(ProfileAvatar, {
      avatarUrl: AVATAR_URL,
      label: "Bruce",
      testId: "profile-avatar",
    }),
  );

  const image = getByTestId("profile-avatar-image");
  assert.equal(image.getAttribute("src"), AVATAR_URL);
  assert.notEqual(image.getAttribute("loading"), "lazy");
});

// Pins the Radix behavior this fix relies on, not the fix itself: if a Radix
// upgrade stops probing on mount, revisit whether avatars should lazy-load.
test("Radix behavior: the load probe requests the avatar as soon as it mounts", () => {
  render(
    React.createElement(UserAvatar, {
      avatarUrl: AVATAR_URL,
      displayName: "Bruce",
    }),
  );

  // The request happens before any visible <img> exists, so a `loading`
  // attribute on that <img> cannot defer it.
  assert.ok(imageSrcAssignments.includes(AVATAR_URL));
});
