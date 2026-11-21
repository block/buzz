import assert from "node:assert/strict";
import test, { afterEach, mock } from "node:test";

import {
  completeCommunityViewTransition,
  replaceCommunityDestinationRoute,
  runCommunityViewTransition,
  shouldSkipCommunityViewTransition,
} from "./communityViewTransition.ts";

const originalDocument = globalThis.document;
const originalWindow = globalThis.window;
const originalNavigator = Object.getOwnPropertyDescriptor(
  globalThis,
  "navigator",
);

const WEBKITGTK_UA =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";
const MAC_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15";

afterEach(() => {
  globalThis.document = originalDocument;
  globalThis.window = originalWindow;
  if (originalNavigator) {
    Object.defineProperty(globalThis, "navigator", originalNavigator);
  } else {
    delete globalThis.navigator;
  }
  mock.restoreAll();
});

// `undefined` removes navigator entirely, so `typeof navigator === "undefined"`.
// afterEach puts the real one back.
function setNavigator(value) {
  if (value === undefined) {
    delete globalThis.navigator;
    return;
  }
  Object.defineProperty(globalThis, "navigator", { configurable: true, value });
}

function installBrowser(startViewTransition) {
  globalThis.window = { clearTimeout, setTimeout };
  globalThis.document = { startViewTransition };
  // node reports navigator.platform as "Linux x86_64", which is exactly what
  // the crash guard skips the transition for. Default these tests to a
  // platform that keeps the animation path; the linux cases set their own.
  setNavigator({ platform: "MacIntel", userAgent: MAC_UA });
}

function transitionFor(callback) {
  return { updateCallbackDone: Promise.resolve().then(callback) };
}

test("replaceCommunityDestinationRoute uses router history and encodes the channel id", () => {
  const replacements = [];
  replaceCommunityDestinationRoute("channel/with spaces", {
    replace: (href) => replacements.push(href),
  });
  assert.deepEqual(replacements, ["/channels/channel%2Fwith%20spaces"]);
});

test("the transition is skipped on linux and kept everywhere else", () => {
  setNavigator({ platform: "Linux x86_64", userAgent: WEBKITGTK_UA });
  assert.equal(shouldSkipCommunityViewTransition(), true);

  // Chromium on Linux is skipped too. The guard fails closed on purpose: it
  // cannot tell a WebKitGTK webview from a Linux browser without sniffing the
  // user agent, and losing an animation costs less than a segfault.
  setNavigator({
    platform: "Linux x86_64",
    userAgent:
      "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
  });
  assert.equal(shouldSkipCommunityViewTransition(), true);

  setNavigator({ platform: "MacIntel", userAgent: MAC_UA });
  assert.equal(shouldSkipCommunityViewTransition(), false);

  // Android reports a Linux platform but is not a WebKitGTK desktop webview.
  setNavigator({
    platform: "Linux armv8l",
    userAgent:
      "Mozilla/5.0 (Linux; Android 14) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Mobile Safari/537.36",
  });
  assert.equal(shouldSkipCommunityViewTransition(), false);

  setNavigator(undefined);
  assert.equal(shouldSkipCommunityViewTransition(), false);
});

test("linux runs the update without starting a transition", async () => {
  const startViewTransition = mock.fn((callback) => transitionFor(callback));
  installBrowser(startViewTransition);
  setNavigator({ platform: "Linux x86_64", userAgent: WEBKITGTK_UA });

  let updated = false;
  await runCommunityViewTransition(async () => {
    updated = true;
  });
  assert.equal(updated, true);
  assert.equal(startViewTransition.mock.callCount(), 0);
});

test("unsupported browsers execute the update and contain rejection", async () => {
  installBrowser(undefined);
  const expected = new Error("navigation failed");
  const error = mock.method(console, "error", () => {});

  await assert.doesNotReject(() =>
    runCommunityViewTransition(async () => {
      throw expected;
    }),
  );

  assert.equal(error.mock.callCount(), 1);
  assert.equal(error.mock.calls[0].arguments[1], expected);
});

test("supported transitions wait for target readiness", async () => {
  let updateFinished = false;
  let transitionFinished = false;
  installBrowser((callback) => transitionFor(callback));

  const pending = runCommunityViewTransition(async () => {
    updateFinished = true;
  }).then(() => {
    transitionFinished = true;
  });

  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(updateFinished, true);
  assert.equal(transitionFinished, false);

  completeCommunityViewTransition();
  await pending;
  assert.equal(transitionFinished, true);
});

test("a newer transition releases the previous transition", async () => {
  installBrowser((callback) => transitionFor(callback));

  let firstFinished = false;
  const first = runCommunityViewTransition(() => {}).then(() => {
    firstFinished = true;
  });
  await new Promise((resolve) => setTimeout(resolve, 0));

  const second = runCommunityViewTransition(() => {});
  await first;
  assert.equal(firstFinished, true);

  completeCommunityViewTransition();
  await second;
});

test("timeout releases a transition whose target never reports ready", async () => {
  installBrowser((callback) => transitionFor(callback));

  await assert.doesNotReject(() =>
    runCommunityViewTransition(() => {}, { timeoutMs: 1 }),
  );
});

test("view-transition callback rejection is contained", async () => {
  installBrowser((callback) => transitionFor(callback));
  const expected = new Error("route rejected");
  const error = mock.method(console, "error", () => {});

  await assert.doesNotReject(() =>
    runCommunityViewTransition(async () => {
      throw expected;
    }),
  );

  assert.equal(error.mock.callCount(), 1);
  assert.equal(error.mock.calls[0].arguments[1], expected);
});
