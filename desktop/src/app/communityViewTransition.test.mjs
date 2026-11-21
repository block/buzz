import assert from "node:assert/strict";
import test, { afterEach, mock } from "node:test";

import {
  completeCommunityViewTransition,
  replaceCommunityDestinationRoute,
  runCommunityViewTransition,
} from "./communityViewTransition.ts";

const originalDocument = globalThis.document;
const originalWindow = globalThis.window;
const originalNavigator = Object.getOwnPropertyDescriptor(
  globalThis,
  "navigator",
);

afterEach(() => {
  completeCommunityViewTransition();
  globalThis.document = originalDocument;
  globalThis.window = originalWindow;
  if (originalNavigator) {
    Object.defineProperty(globalThis, "navigator", originalNavigator);
  } else {
    delete globalThis.navigator;
  }
  mock.restoreAll();
});

function installBrowser(
  startViewTransition,
  navigator = {
    platform: "MacIntel",
    userAgent:
      "Mozilla/5.0 (Macintosh) AppleWebKit/605.1.15 Version/18 Safari/605.1.15",
  },
) {
  globalThis.window = { clearTimeout, setTimeout };
  globalThis.document = { startViewTransition };
  Object.defineProperty(globalThis, "navigator", {
    configurable: true,
    value: navigator,
  });
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

for (const platform of ["Linux x86_64", "Linux aarch64"]) {
  test(`${platform} WebKit bypasses snapshots and awaits the navigation update`, async () => {
    const snapshot = mock.fn(() => {
      throw new Error("Unsafe WebKit snapshot path must not run");
    });
    installBrowser(snapshot, {
      platform,
      userAgent:
        "Mozilla/5.0 (X11; Linux) AppleWebKit/605.1.15 Version/60.5 Safari/605.1.15",
    });
    let updates = 0;
    let finished = false;
    await runCommunityViewTransition(async () => {
      updates += 1;
      await new Promise((resolve) => setTimeout(resolve, 0));
      finished = true;
      completeCommunityViewTransition();
    });
    assert.equal(snapshot.mock.callCount(), 0);
    assert.equal(updates, 1);
    assert.equal(finished, true);
  });
}

test("Linux WebKit fallback contains navigation rejection", async () => {
  const snapshot = mock.fn();
  installBrowser(snapshot, {
    platform: "Linux x86_64",
    userAgent: "Mozilla/5.0 AppleWebKit/605.1.15 Safari/605.1.15",
  });
  const expected = new Error("navigation failed");
  const error = mock.method(console, "error", () => {});
  await assert.doesNotReject(() =>
    runCommunityViewTransition(async () => {
      throw expected;
    }),
  );
  assert.equal(snapshot.mock.callCount(), 0);
  assert.equal(error.mock.callCount(), 1);
  assert.equal(error.mock.calls[0].arguments[1], expected);
});

for (const [name, platform, userAgent] of [
  [
    "Linux Chromium",
    "Linux x86_64",
    "Mozilla/5.0 AppleWebKit/537.36 Chrome/145.0.0.0 Safari/537.36",
  ],
  [
    "macOS WebKit",
    "MacIntel",
    "Mozilla/5.0 AppleWebKit/605.1.15 Version/18 Safari/605.1.15",
  ],
  [
    "Windows Edge",
    "Win32",
    "Mozilla/5.0 AppleWebKit/537.36 Chrome/145.0.0.0 Safari/537.36 Edg/145.0.0.0",
  ],
  [
    "Android WebKit",
    "Linux aarch64",
    "Mozilla/5.0 (Linux; Android 14) AppleWebKit/605.1.15 Safari/605.1.15",
  ],
]) {
  test(`${name} retains snapshots and target-readiness waiting`, async () => {
    const snapshot = mock.fn((callback) => transitionFor(callback));
    installBrowser(snapshot, { platform, userAgent });
    let finished = false;
    const pending = runCommunityViewTransition(() => {}).then(() => {
      finished = true;
    });
    await new Promise((resolve) => setTimeout(resolve, 0));
    assert.equal(snapshot.mock.callCount(), 1);
    assert.equal(finished, false);
    completeCommunityViewTransition();
    await pending;
    assert.equal(finished, true);
  });
}

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

for (const platform of ["Linux x86_64", "Linux aarch64", "unsupported"]) {
  function installFallback() {
    const snapshot = mock.fn(() => {
      throw new Error("Native snapshots must not run in the fallback");
    });
    installBrowser(platform === "unsupported" ? undefined : snapshot, {
      platform,
      userAgent: "Mozilla/5.0 AppleWebKit/605.1.15 Safari/605.1.15",
    });
    return snapshot;
  }

  test(`${platform} fallback waits for target readiness after the update`, async () => {
    const snapshot = installFallback();
    let updated = false;
    let finished = false;
    const pending = runCommunityViewTransition(() => {
      updated = true;
    }).then(() => {
      finished = true;
    });
    try {
      await new Promise((resolve) => setImmediate(resolve));
      assert.equal(updated, true);
      assert.equal(finished, false);
      assert.equal(snapshot.mock.callCount(), 0);
    } finally {
      completeCommunityViewTransition();
      await pending;
    }
    assert.equal(finished, true);
  });

  test(`${platform} fallback still awaits an update when readiness arrives early`, async () => {
    installFallback();
    let releaseUpdate;
    const updating = new Promise((resolve) => {
      releaseUpdate = resolve;
    });
    let finished = false;
    const pending = runCommunityViewTransition(() => updating).then(() => {
      finished = true;
    });
    completeCommunityViewTransition();
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(finished, false);
    releaseUpdate();
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(finished, true);
    await pending;
    assert.equal(finished, true);
  });

  test(`${platform} fallback supersedes the previous wait without clearing the next`, async () => {
    installFallback();
    let firstFinished = false;
    const first = runCommunityViewTransition(() => {}).then(() => {
      firstFinished = true;
    });
    let secondFinished = false;
    try {
      await new Promise((resolve) => setImmediate(resolve));
      assert.equal(firstFinished, false);
      const second = runCommunityViewTransition(() => {}).then(() => {
        secondFinished = true;
      });
      await first;
      assert.equal(firstFinished, true);
      assert.equal(secondFinished, false);
      completeCommunityViewTransition();
      await new Promise((resolve) => setImmediate(resolve));
      assert.equal(secondFinished, true);
      await second;
      assert.equal(secondFinished, true);
    } finally {
      completeCommunityViewTransition();
      await first;
    }
  });

  for (const timeoutMs of [undefined, 0, 20]) {
    test(`${platform} fallback bounds its wait (${timeoutMs ?? "default"} ms) and clears the timer`, async () => {
      installFallback();
      let expire;
      const timer = mock.method(window, "setTimeout", (callback, delay) => {
        assert.equal(delay, timeoutMs ?? 5_000);
        expire = callback;
        return 42;
      });
      const clear = mock.method(window, "clearTimeout", () => {});
      let finished = false;
      const pending = runCommunityViewTransition(() => {}, {
        timeoutMs,
      }).then(() => {
        finished = true;
      });
      try {
        await new Promise((resolve) => setImmediate(resolve));
        assert.equal(finished, false);
        assert.equal(timer.mock.callCount(), 1);
        expire();
        await new Promise((resolve) => setImmediate(resolve));
        assert.equal(finished, true);
      } finally {
        completeCommunityViewTransition();
        await pending;
      }
      assert.equal(finished, true);
      assert.equal(clear.mock.callCount(), 1);
      assert.equal(clear.mock.calls[0].arguments[0], 42);
    });
  }

  test(`${platform} fallback contains update failure and clears its timer`, async () => {
    installFallback();
    const expected = new Error("navigation failed");
    const error = mock.method(console, "error", () => {});
    const clear = mock.method(window, "clearTimeout", () => {});
    await assert.doesNotReject(() =>
      runCommunityViewTransition(() => {
        throw expected;
      }),
    );
    assert.equal(error.mock.callCount(), 1);
    assert.equal(error.mock.calls[0].arguments[1], expected);
    assert.equal(clear.mock.callCount(), 1);
  });
}

for (const fallbackFirst of [true, false]) {
  test(`a retired ${fallbackFirst ? "fallback" : "native"} timeout cannot release the next transition`, async () => {
    const snapshot = mock.fn((callback) => transitionFor(callback));
    const browser = (fallback) => ({
      platform: fallback ? "Linux x86_64" : "MacIntel",
      userAgent: "Mozilla/5.0 AppleWebKit/605.1.15 Safari/605.1.15",
    });
    installBrowser(snapshot, browser(fallbackFirst));
    const expirations = [];
    mock.method(window, "setTimeout", (callback) => {
      expirations.push(callback);
      return expirations.length;
    });
    mock.method(window, "clearTimeout", () => {});
    const first = runCommunityViewTransition(() => {});
    Object.defineProperty(globalThis, "navigator", {
      configurable: true,
      value: browser(!fallbackFirst),
    });
    let finished = false;
    const second = runCommunityViewTransition(() => {}).then(() => {
      finished = true;
    });
    try {
      await first;
      // Even an already-queued timer from the retired wait owns only that wait.
      expirations[0]();
      await new Promise((resolve) => setImmediate(resolve));
      assert.equal(finished, false);
      expirations[1]();
      await new Promise((resolve) => setImmediate(resolve));
      assert.equal(finished, true);
      await second;
      assert.equal(finished, true);
    } finally {
      completeCommunityViewTransition();
      await second;
    }
  });
}
