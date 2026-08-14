import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

const HASH = "a".repeat(64);
const RELAY_ORIGIN = "https://relay.example";
const RELAY_AVATAR_URL = `${RELAY_ORIGIN}/media/${HASH}.png`;
const EXTERNAL_AVATAR_URL = `https://blossom.example/media/${HASH}.png`;
const FALLBACK_AVATAR_URL = `buzz-media://localhost/media/${HASH}.png`;
const PROXIED_AVATAR_URL = `http://127.0.0.1:54321/media/${HASH}.png`;
const MISROUTED_AVATAR_URL = PROXIED_AVATAR_URL;
const dom = new JSDOM(
  '<!doctype html><html><body><div id="root"></div></body></html>',
  { url: "http://localhost" },
);

let relayOriginCalls = 0;
let resolveRelayOriginRetry;
const relayOriginRetry = new Promise((resolve) => {
  resolveRelayOriginRetry = resolve;
});
let getMediaProxyPort = () => Promise.resolve(54321);
let getRelayOrigin = () => {
  relayOriginCalls += 1;
  if (relayOriginCalls === 1) {
    return Promise.reject(new Error("Relay origin is not ready"));
  }
  return relayOriginRetry;
};

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
      if (command === "get_media_proxy_port") return getMediaProxyPort();
      if (command === "get_relay_http_url") return getRelayOrigin();
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

async function renderSharedAvatars(React, createRoot, avatarUrl) {
  const { ProfileAvatar } = await import("@/features/profile/ui/ProfileAvatar");
  const { UserAvatar } = await import("./UserAvatar.tsx");
  const root = createRoot(document.getElementById("root"));

  await React.act(async () => {
    root.render(
      React.createElement(
        React.Fragment,
        null,
        React.createElement(UserAvatar, {
          avatarUrl,
          displayName: "Channel agent",
          testId: "user-avatar",
        }),
        React.createElement(ProfileAvatar, {
          avatarUrl,
          label: "Running agent",
          testId: "profile-avatar",
        }),
      ),
    );
  });

  return {
    profileAvatar: document.querySelector(
      '[data-testid="profile-avatar-image"]',
    ),
    root,
    userAvatar: document.querySelector('[data-testid="user-avatar-image"]'),
  };
}

test("shared avatars recover external URLs when the relay origin resolves after the proxy port", async () => {
  const React = await import("react");
  const { createRoot } = await import("react-dom/client");
  const { profileAvatar, root, userAvatar } = await renderSharedAvatars(
    React,
    createRoot,
    EXTERNAL_AVATAR_URL,
  );
  assert.equal(userAvatar?.getAttribute("src"), MISROUTED_AVATAR_URL);
  assert.equal(profileAvatar?.getAttribute("src"), MISROUTED_AVATAR_URL);
  assert.equal(relayOriginCalls, 1);

  await React.act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 100));
  });
  assert.equal(relayOriginCalls, 2);

  await React.act(async () => {
    resolveRelayOriginRetry(RELAY_ORIGIN);
    await relayOriginRetry;
  });

  assert.equal(userAvatar?.getAttribute("src"), EXTERNAL_AVATAR_URL);
  assert.equal(profileAvatar?.getAttribute("src"), EXTERNAL_AVATAR_URL);

  await React.act(async () => root.unmount());
});

test("shared avatars switch to the authenticated media proxy when its port resolves", async () => {
  const React = await import("react");
  const { createRoot } = await import("react-dom/client");
  const { resetMediaCaches } = await import("@/shared/lib/mediaUrl");
  let resolveProxyPort;
  const proxyPort = new Promise((resolve) => {
    resolveProxyPort = resolve;
  });
  getMediaProxyPort = () => proxyPort;
  getRelayOrigin = () => Promise.resolve(RELAY_ORIGIN);
  resetMediaCaches();

  const { profileAvatar, root, userAvatar } = await renderSharedAvatars(
    React,
    createRoot,
    RELAY_AVATAR_URL,
  );
  assert.equal(userAvatar?.getAttribute("src"), FALLBACK_AVATAR_URL);
  assert.equal(profileAvatar?.getAttribute("src"), FALLBACK_AVATAR_URL);

  await React.act(async () => {
    resolveProxyPort(54321);
    await proxyPort;
  });

  assert.equal(userAvatar?.getAttribute("src"), PROXIED_AVATAR_URL);
  assert.equal(profileAvatar?.getAttribute("src"), PROXIED_AVATAR_URL);

  await React.act(async () => root.unmount());
});
