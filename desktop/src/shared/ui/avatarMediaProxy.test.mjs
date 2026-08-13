import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

const HASH = "a".repeat(64);
const RELAY_AVATAR_URL = `https://relay.example/media/${HASH}.png`;
const PROXIED_AVATAR_URL = `http://127.0.0.1:54321/media/${HASH}.png`;
const dom = new JSDOM(
  '<!doctype html><html><body><div id="root"></div></body></html>',
  { url: "http://localhost" },
);

let resolveProxyPort;
const proxyPort = new Promise((resolve) => {
  resolveProxyPort = resolve;
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
      if (command === "get_media_proxy_port") return proxyPort;
      if (command === "get_relay_http_url") {
        return Promise.resolve("https://relay.example");
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

test("shared avatars switch to the authenticated media proxy when its port resolves", async () => {
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
          avatarUrl: RELAY_AVATAR_URL,
          displayName: "Channel agent",
          testId: "user-avatar",
        }),
        React.createElement(ProfileAvatar, {
          avatarUrl: RELAY_AVATAR_URL,
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
  assert.equal(
    userAvatar?.getAttribute("src"),
    `buzz-media://localhost/media/${HASH}.png`,
  );
  assert.equal(
    profileAvatar?.getAttribute("src"),
    `buzz-media://localhost/media/${HASH}.png`,
  );

  await act(async () => {
    resolveProxyPort(54321);
    await proxyPort;
    await new Promise((resolve) => setTimeout(resolve, 0));
  });

  assert.equal(userAvatar?.getAttribute("src"), PROXIED_AVATAR_URL);
  assert.equal(profileAvatar?.getAttribute("src"), PROXIED_AVATAR_URL);

  await act(async () => root.unmount());
});
