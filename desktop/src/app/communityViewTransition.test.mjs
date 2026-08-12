import assert from "node:assert/strict";
import test, { afterEach, mock } from "node:test";

import {
  completeCommunityViewTransition,
  isLinuxWebKitGtk,
  replaceCommunityDestinationRoute,
  runCommunityViewTransition,
} from "./communityViewTransition.ts";

const originalDocument = globalThis.document;
const originalWindow = globalThis.window;

afterEach(() => {
  globalThis.document = originalDocument;
  globalThis.window = originalWindow;
  mock.restoreAll();
});

function installBrowser(startViewTransition) {
  globalThis.window = { clearTimeout, setTimeout };
  globalThis.document = { startViewTransition };
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

const WEBKITGTK_UA =
  "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15";

test("isLinuxWebKitGtk matches WebKitGTK and nothing else", () => {
  assert.equal(isLinuxWebKitGtk(WEBKITGTK_UA), true);
  assert.equal(
    isLinuxWebKitGtk(
      "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36",
    ),
    false,
  );
  assert.equal(
    isLinuxWebKitGtk(
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.4 Safari/605.1.15",
    ),
    false,
  );
  assert.equal(isLinuxWebKitGtk(""), false);
});

test("linux webkitgtk runs the update without starting a transition", async () => {
  const startViewTransition = mock.fn((callback) => transitionFor(callback));
  installBrowser(startViewTransition);
  const navigatorDescriptor = Object.getOwnPropertyDescriptor(
    globalThis,
    "navigator",
  );
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: { userAgent: WEBKITGTK_UA },
  });

  try {
    let updated = false;
    await runCommunityViewTransition(async () => {
      updated = true;
    });
    assert.equal(updated, true);
    assert.equal(startViewTransition.mock.callCount(), 0);
  } finally {
    if (navigatorDescriptor) {
      Object.defineProperty(globalThis, "navigator", navigatorDescriptor);
    } else {
      delete globalThis.navigator;
    }
  }
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
