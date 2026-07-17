import assert from "node:assert/strict";
import { test } from "node:test";

import { mediaProxyUrl } from "./mediaUrl.ts";

const HASH = "a".repeat(64);

function deferred() {
  let resolve;
  const promise = new Promise((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

test("mediaProxyUrl: uses the IPv4 loopback literal for the localhost proxy", () => {
  assert.equal(
    mediaProxyUrl(54321, `${HASH}.png`),
    `http://127.0.0.1:54321/media/${HASH}.png`,
  );
});

test("resetMediaCaches: ignores relay origin lookups from the previous generation", async () => {
  const previousWindow = globalThis.window;
  const staleOrigin = deferred();
  let relayOriginCalls = 0;

  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke(command) {
        if (command === "get_media_proxy_port") return Promise.resolve(54321);
        if (command === "get_relay_http_url") {
          relayOriginCalls += 1;
          return relayOriginCalls === 1
            ? staleOrigin.promise
            : Promise.resolve("https://active.example");
        }
        return Promise.reject(new Error(`Unexpected command: ${command}`));
      },
    },
  };

  try {
    // A unique URL triggers module-load fetching with the stale relay lookup
    // still unresolved, matching a cold launch before applyCommunity finishes.
    const mediaUrl = await import(`./mediaUrl.ts?race=${Date.now()}`);
    await new Promise((resolve) => setTimeout(resolve, 0));

    mediaUrl.resetMediaCaches();
    const activeUrl = `https://active.example/media/${HASH}.png`;
    mediaUrl.rewriteRelayUrl(activeUrl);
    await new Promise((resolve) => setTimeout(resolve, 0));

    // Complete the old lookup after reset. It must not overwrite the active
    // community origin fetched by the new generation.
    staleOrigin.resolve("https://stale.example");
    await new Promise((resolve) => setTimeout(resolve, 0));

    assert.equal(
      mediaUrl.rewriteRelayUrl(activeUrl),
      `http://127.0.0.1:54321/media/${HASH}.png`,
    );
  } finally {
    globalThis.window = previousWindow;
  }
});
