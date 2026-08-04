import assert from "node:assert/strict";
import test, { mock } from "node:test";
import { relayClient } from "@/shared/api/relayClient";
import {
  CommunityThemeSyncManager,
  isNewerCommunityThemeCoordinate,
  shouldSeedCommunityTheme,
} from "./communityThemeSync.ts";

const preference = {
  version: 1,
  theme: "houston",
  accent: "#3b82f6",
  followSystem: false,
};

function installFakeTimer() {
  globalThis.window ??= {};
  let callback = null;
  let delay = null;
  const originalSet = window.setTimeout;
  const originalClear = window.clearTimeout;
  window.setTimeout = (fn, requestedDelay) => {
    callback = fn;
    delay = requestedDelay;
    return 1;
  };
  window.clearTimeout = () => {
    callback = null;
    delay = null;
  };
  return {
    fire: () => {
      const fn = callback;
      callback = null;
      delay = null;
      fn?.();
    },
    pending: () => callback !== null,
    delay: () => delay,
    restore: () => {
      window.setTimeout = originalSet;
      window.clearTimeout = originalClear;
    },
  };
}

test("destroy cancels a debounced community write before relay teardown", () => {
  const timer = installFakeTimer();
  const publishes = [];
  mock.method(relayClient, "publishEvent", (...args) => {
    publishes.push(args);
    return Promise.resolve();
  });
  try {
    const manager = new CommunityThemeSyncManager("alice");
    manager.publish(preference);
    assert.equal(timer.pending(), true);
    manager.destroy();
    assert.equal(timer.pending(), false);
    timer.fire();
    assert.equal(publishes.length, 0);
  } finally {
    timer.restore();
    mock.reset();
  }
});

test("destroy is safe without a pending community write", () => {
  const manager = new CommunityThemeSyncManager("alice");
  assert.doesNotThrow(() => manager.destroy());
  assert.equal(manager.getPending(), null);
});

function relayEvent(overrides = {}) {
  return {
    id: "event-id",
    pubkey: "alice",
    kind: 30078,
    content: "not-decryptable",
    created_at: 123,
    tags: [["d", "community-theme"]],
    ...overrides,
  };
}

test("fetch distinguishes absent remote state from unreadable existing state", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  try {
    const manager = new CommunityThemeSyncManager("alice");
    assert.deepEqual(await manager.fetchRemote(), { status: "absent" });
  } finally {
    mock.reset();
  }

  mock.method(relayClient, "fetchEvents", () =>
    Promise.resolve([relayEvent()]),
  );
  try {
    const manager = new CommunityThemeSyncManager("alice");
    assert.deepEqual(await manager.fetchRemote(), { status: "invalid" });
  } finally {
    mock.reset();
  }
});

test("fetch reports relay failures as unavailable rather than absent", async () => {
  mock.method(relayClient, "fetchEvents", () =>
    Promise.reject(new Error("offline")),
  );
  try {
    const manager = new CommunityThemeSyncManager("alice");
    assert.deepEqual(await manager.fetchRemote(), { status: "unavailable" });
  } finally {
    mock.reset();
  }
});

test("only confirmed absence permits seeding relay state", () => {
  assert.equal(shouldSeedCommunityTheme({ status: "absent" }), true);
  assert.equal(shouldSeedCommunityTheme({ status: "invalid" }), false);
  assert.equal(shouldSeedCommunityTheme({ status: "unavailable" }), false);
});

test("acknowledged coordinates use relay same-second ordering", () => {
  const acknowledged = { createdAt: 123, eventId: "published" };
  assert.equal(
    isNewerCommunityThemeCoordinate(
      { createdAt: 123, eventId: "a-winner" },
      acknowledged,
    ),
    true,
  );
  assert.equal(
    isNewerCommunityThemeCoordinate(
      { createdAt: 123, eventId: "z-loser" },
      acknowledged,
    ),
    false,
  );
});

test("new remote invalidates no-op suppression for A to B to A", async () => {
  const timer = installFakeTimer();
  const published = [];
  const acknowledgements = [];
  let signedEventId = "published-z";
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke(command, args) {
      if (command === "nip44_encrypt_to_self") return Promise.resolve("cipher");
      if (command === "sign_event") {
        return Promise.resolve(
          JSON.stringify(
            relayEvent({
              id: signedEventId,
              content: args.content,
              created_at: args.createdAt,
            }),
          ),
        );
      }
      throw new Error(`unexpected command: ${command}`);
    },
  };
  mock.method(relayClient, "publishEvent", (event) => {
    published.push(event);
    return Promise.resolve();
  });
  try {
    const manager = new CommunityThemeSyncManager("alice", (event) => {
      acknowledgements.push(event);
    });
    manager.publish(preference);
    timer.fire();
    await waitUntil(() => published.length === 1);

    manager.acceptRemote({
      preference: { ...preference, theme: "dracula" },
      createdAt: published[0].created_at,
      eventId: "remote-a",
    });
    signedEventId = "republished-a";
    manager.publish(preference);
    timer.fire();
    await waitUntil(() => published.length === 2);

    assert.equal(acknowledgements.length, 2);
    assert.equal(acknowledgements[1].eventId, "republished-a");
    assert.equal(manager.getPending(), null);
  } finally {
    delete globalThis.window.__TAURI_INTERNALS__;
    timer.restore();
    mock.reset();
  }
});

test("serializes an in-flight publish before sending the latest edit", async () => {
  const timer = installFakeTimer();
  const first = Promise.withResolvers();
  const published = [];
  let signed = 0;
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke(command, args) {
      if (command === "nip44_encrypt_to_self") return Promise.resolve("cipher");
      if (command === "sign_event") {
        signed += 1;
        return Promise.resolve(
          JSON.stringify(
            relayEvent({
              id: `event-${signed}`,
              content: args.content,
              created_at: args.createdAt,
            }),
          ),
        );
      }
      throw new Error(`unexpected command: ${command}`);
    },
  };
  mock.method(relayClient, "publishEvent", (event) => {
    published.push(event);
    return published.length === 1 ? first.promise : Promise.resolve();
  });
  try {
    const manager = new CommunityThemeSyncManager("alice");
    manager.publish(preference);
    timer.fire();
    await waitUntil(() => published.length === 1);

    const latest = { ...preference, theme: "dracula" };
    manager.publish(latest);
    timer.fire();
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(published.length, 1);

    first.resolve();
    await waitUntil(() => timer.pending());
    assert.equal(timer.delay(), 0);
    timer.fire();
    await waitUntil(() => published.length === 2);

    assert.ok(published[1].created_at > published[0].created_at);
    assert.deepEqual(manager.getPending(), null);
  } finally {
    delete globalThis.window.__TAURI_INTERNALS__;
    timer.restore();
    mock.reset();
  }
});

test("republishes above a newer remote observed while publish is in flight", async () => {
  const timer = installFakeTimer();
  const first = Promise.withResolvers();
  const published = [];
  const acknowledgements = [];
  let signed = 0;
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke(command, args) {
      if (command === "nip44_encrypt_to_self") return Promise.resolve("cipher");
      if (command === "sign_event") {
        signed += 1;
        return Promise.resolve(
          JSON.stringify(
            relayEvent({
              id: `event-${signed}`,
              content: args.content,
              created_at: args.createdAt,
            }),
          ),
        );
      }
      throw new Error(`unexpected command: ${command}`);
    },
  };
  mock.method(relayClient, "publishEvent", (event) => {
    published.push(event);
    return published.length === 1 ? first.promise : Promise.resolve();
  });
  try {
    const manager = new CommunityThemeSyncManager("alice", (event) => {
      acknowledgements.push(event);
    });
    manager.publish(preference);
    timer.fire();
    await waitUntil(() => published.length === 1);

    manager.acceptRemote({
      preference: { ...preference, theme: "dracula" },
      createdAt: published[0].created_at + 100,
      eventId: "remote-winner",
    });
    first.resolve();
    await waitUntil(() => timer.pending());
    assert.equal(acknowledgements.length, 0);
    assert.deepEqual(manager.getPending(), preference);

    timer.fire();
    await waitUntil(() => acknowledgements.length === 1);
    assert.equal(published.length, 2);
    assert.ok(published[1].created_at > published[0].created_at + 100);
    assert.deepEqual(manager.getPending(), null);
  } finally {
    delete globalThis.window.__TAURI_INTERNALS__;
    timer.restore();
    mock.reset();
  }
});

test("delayed live decryption fences publish acknowledgement and preserves local intent", async () => {
  const timer = installFakeTimer();
  const firstPublish = Promise.withResolvers();
  const remotePlaintext = Promise.withResolvers();
  const published = [];
  const acknowledgements = [];
  let liveCallback;
  let signed = 0;
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke(command, args) {
      if (command === "nip44_encrypt_to_self") return Promise.resolve("cipher");
      if (command === "nip44_decrypt_from_self") return remotePlaintext.promise;
      if (command === "sign_event") {
        signed += 1;
        return Promise.resolve(
          JSON.stringify(
            relayEvent({
              id: `event-${signed}`,
              content: args.content,
              created_at: args.createdAt,
            }),
          ),
        );
      }
      throw new Error(`unexpected command: ${command}`);
    },
  };
  mock.method(relayClient, "subscribeLive", (_filter, callback) => {
    liveCallback = callback;
    return Promise.resolve(async () => {});
  });
  mock.method(relayClient, "publishEvent", (event) => {
    published.push(event);
    return published.length === 1 ? firstPublish.promise : Promise.resolve();
  });
  try {
    const manager = new CommunityThemeSyncManager("alice", (event) => {
      acknowledgements.push(event);
    });
    await manager.subscribe(() => {});
    manager.publish(preference);
    timer.fire();
    await waitUntil(() => published.length === 1);

    liveCallback(
      relayEvent({
        id: "remote-winner",
        content: "delayed-ciphertext",
        created_at: published[0].created_at + 100,
      }),
    );
    firstPublish.resolve();
    await new Promise((resolve) => setImmediate(resolve));
    assert.equal(acknowledgements.length, 0);
    assert.deepEqual(manager.getPending(), preference);

    remotePlaintext.resolve(
      JSON.stringify({ ...preference, theme: "dracula" }),
    );
    await waitUntil(() => timer.pending());
    assert.equal(acknowledgements.length, 0);
    assert.deepEqual(manager.getPending(), preference);

    timer.fire();
    await waitUntil(() => acknowledgements.length === 1);
    assert.equal(published.length, 2);
    assert.ok(published[1].created_at > published[0].created_at + 100);
    assert.deepEqual(manager.getPending(), null);
  } finally {
    delete globalThis.window.__TAURI_INTERNALS__;
    timer.restore();
    mock.reset();
  }
});

test("transient publish failure retries and acknowledges exact event", async () => {
  const timer = installFakeTimer();
  const published = [];
  const acknowledgements = [];
  let attempts = 0;
  globalThis.window.__TAURI_INTERNALS__ = {
    invoke(command, args) {
      if (command === "nip44_encrypt_to_self") return Promise.resolve("cipher");
      if (command === "sign_event") {
        return Promise.resolve(
          JSON.stringify(
            relayEvent({
              id: "published-event",
              content: args.content,
              created_at: args.createdAt,
            }),
          ),
        );
      }
      throw new Error(`unexpected command: ${command}`);
    },
  };
  mock.method(relayClient, "publishEvent", (event) => {
    attempts += 1;
    if (attempts === 1) return Promise.reject(new Error("timeout"));
    published.push(event);
    return Promise.resolve();
  });
  try {
    const manager = new CommunityThemeSyncManager("alice", (event) => {
      acknowledgements.push(event);
    });
    manager.publish(preference);
    timer.fire();
    await waitUntil(() => timer.pending());
    assert.equal(timer.delay(), 1_000);
    assert.deepEqual(manager.getPending(), preference);

    timer.fire();
    await waitUntil(() => acknowledgements.length === 1);
    assert.equal(attempts, 2);
    assert.equal(published.length, 1);
    assert.equal(manager.getPending(), null);
    assert.deepEqual(acknowledgements[0], {
      preference,
      createdAt: published[0].created_at,
      eventId: "published-event",
    });
  } finally {
    delete globalThis.window.__TAURI_INTERNALS__;
    timer.restore();
    mock.reset();
  }
});

async function waitUntil(condition) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (condition()) return;
    await new Promise((resolve) => setImmediate(resolve));
  }
  assert.fail("condition not met");
}
