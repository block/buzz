import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

const HASH = "a".repeat(64);
const RELAY_ORIGIN = "https://relay.example";
const EXTERNAL_AVATAR_URL = `https://blossom.example/media/${HASH}.png`;
const MISROUTED_AVATAR_URL = `http://127.0.0.1:54321/media/${HASH}.png`;
const dom = new JSDOM(
  '<!doctype html><html><body><div id="root"></div></body></html>',
  { url: "http://localhost" },
);

let relayOriginCalls = 0;
let resolveRelayOriginRetry;
const relayOriginRetry = new Promise((resolve) => {
  resolveRelayOriginRetry = resolve;
});

class LoadedImage extends dom.window.EventTarget {
  complete = true;
  naturalWidth = 1;
  referrerPolicy = "";
  #src = "";

  get src() {
    return this.#src;
  }

  set src(value) {
    this.#src = value;
  }
}

before(() => {
  dom.window.Image = LoadedImage;
  dom.window.__TAURI_INTERNALS__ = {
    invoke(command) {
      if (command === "get_media_proxy_port") return Promise.resolve(54321);
      if (command === "get_relay_http_url") {
        relayOriginCalls += 1;
        if (relayOriginCalls === 1) {
          return Promise.reject(new Error("Relay origin is not ready"));
        }
        return relayOriginRetry;
      }
      return Promise.reject(new Error(`Unexpected command: ${command}`));
    },
  };
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    Image: LoadedImage,
    IS_REACT_ACT_ENVIRONMENT: true,
    MutationObserver: dom.window.MutationObserver,
    window: dom.window,
  });
});

after(() => dom.window.close());

test("shared avatars recover external URLs when the relay origin resolves after the proxy port", async () => {
  const React = await import("react");
  const { act } = React;
  const { createRoot } = await import("react-dom/client");
  const { ProfileAvatar } = await import("@/features/profile/ui/ProfileAvatar");
  const { UserAvatar } = await import("./UserAvatar.tsx");
  const root = createRoot(document.getElementById("root"));

  await act(async () => {
    root.render(
      React.createElement(
        React.Fragment,
        null,
        React.createElement(UserAvatar, {
          avatarUrl: EXTERNAL_AVATAR_URL,
          displayName: "Channel agent",
          testId: "user-avatar",
        }),
        React.createElement(ProfileAvatar, {
          avatarUrl: EXTERNAL_AVATAR_URL,
          label: "Running agent",
          testId: "profile-avatar",
        }),
      ),
    );
  });

  const userAvatar = document.querySelector(
    '[data-testid="user-avatar-image"]',
  );
  const profileAvatar = document.querySelector(
    '[data-testid="profile-avatar-image"]',
  );
  assert.equal(userAvatar?.getAttribute("src"), MISROUTED_AVATAR_URL);
  assert.equal(profileAvatar?.getAttribute("src"), MISROUTED_AVATAR_URL);
  assert.equal(relayOriginCalls, 1);

  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 100));
  });
  assert.equal(relayOriginCalls, 2);

  await act(async () => {
    resolveRelayOriginRetry(RELAY_ORIGIN);
    await relayOriginRetry;
  });

  assert.equal(userAvatar?.getAttribute("src"), EXTERNAL_AVATAR_URL);
  assert.equal(profileAvatar?.getAttribute("src"), EXTERNAL_AVATAR_URL);

  await act(async () => root.unmount());
});
