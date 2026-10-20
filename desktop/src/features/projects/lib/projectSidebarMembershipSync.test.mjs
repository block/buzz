import assert from "node:assert/strict";
import test, { mock } from "node:test";

import { relayClient } from "@/shared/api/relayClient";
import {
  installFakeWindow,
  makeFakeWindow,
} from "../../sidebar/lib/sidebarSyncTestHelpers.mjs";
import { ProjectSidebarMembershipSyncManager } from "./projectSidebarMembershipSync.ts";

const RELAY = "wss://projects.test";

function store(projects = {}) {
  return { version: 1, projects };
}

test("first sync seeds non-empty local membership when remote is absent", async () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  const fakeWindow = makeFakeWindow();
  const restore = installFakeWindow(fakeWindow);
  try {
    const manager = new ProjectSidebarMembershipSyncManager("pubkey", RELAY);
    const result = await manager.bootstrap(
      store({
        "30621:alice:buzz": { selected: true, updatedAt: 1 },
      }),
    );

    assert.equal(result.action, "hold");
    assert.notEqual(manager.getPendingStore(), null);
    manager.destroy();
  } finally {
    restore();
    mock.reset();
  }
});

test("failed fetch never seeds local membership", async () => {
  mock.method(relayClient, "fetchEvents", () =>
    Promise.reject(new Error("offline")),
  );
  const fakeWindow = makeFakeWindow();
  const restore = installFakeWindow(fakeWindow);
  try {
    const manager = new ProjectSidebarMembershipSyncManager("pubkey", RELAY);
    const result = await manager.bootstrap(
      store({
        "30621:alice:buzz": { selected: true, updatedAt: 1 },
      }),
    );

    assert.equal(result.action, "hold");
    assert.equal(manager.getPendingStore(), null);
    manager.destroy();
  } finally {
    restore();
    mock.reset();
  }
});

test("destroy cancels a pending membership publish", () => {
  mock.method(relayClient, "fetchEvents", () => Promise.resolve([]));
  const fakeWindow = makeFakeWindow();
  const restore = installFakeWindow(fakeWindow);
  try {
    const manager = new ProjectSidebarMembershipSyncManager("pubkey", RELAY);
    manager.publishMembership(
      store({
        "30621:alice:buzz": { selected: true, updatedAt: 1 },
      }),
    );
    manager.destroy();

    assert.equal(manager.getPendingStore(), null);
  } finally {
    restore();
    mock.reset();
  }
});
