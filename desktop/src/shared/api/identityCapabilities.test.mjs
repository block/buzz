import assert from "node:assert/strict";
import test, { mock } from "node:test";
import {
  setManagedIdentityMode,
  supportsLocalIdentityFeatures,
  requireLocalIdentityFeature,
} from "./identityCapabilities.ts";
import { relayClient } from "./relayClient.ts";
import { ChannelStarSyncManager } from "../../features/sidebar/lib/channelStarsSync.ts";
import { ChannelMuteSyncManager } from "../../features/sidebar/lib/channelMutesSync.ts";
import { ChannelSortSyncManager } from "../../features/sidebar/lib/channelSortSync.ts";
import { ChannelSectionSyncManager } from "../../features/sidebar/lib/channelSectionsSync.ts";
import { ProjectSidebarMembershipSyncManager } from "../../features/projects/lib/projectSidebarMembershipSync.ts";
import { CommunityThemeSyncManager } from "../theme/communityThemeSync.ts";
import {
  createReminder,
  fetchReminders,
} from "../../features/reminders/lib/reminderService.ts";
import { publishCommunityReadState } from "../../features/communities/communityMarkRead.ts";
import {
  installFakeWindow,
  makeFakeWindow,
} from "../../features/sidebar/lib/sidebarSyncTestHelpers.mjs";

test("managed mode never starts encrypted preference fetches, subscriptions or publish timers", async () => {
  const fw = makeFakeWindow();
  const restore = installFakeWindow(fw);
  let networkCalls = 0;
  for (const method of ["fetchEvents", "subscribeLive", "publishEvent"]) {
    mock.method(relayClient, method, () => {
      networkCalls++;
      throw new Error("must not use relay");
    });
  }
  setManagedIdentityMode(true);
  try {
    assert.equal(supportsLocalIdentityFeatures(), false);
    for (const [Manager, fetch, publish, subscribe] of [
      [
        ChannelStarSyncManager,
        "fetchRemoteStars",
        "publishStars",
        "subscribeToStars",
      ],
      [
        ChannelMuteSyncManager,
        "fetchRemoteMutes",
        "publishMutes",
        "subscribeToMutes",
      ],
      [
        ChannelSortSyncManager,
        "fetchRemoteSortPrefs",
        "publishSortPrefs",
        "subscribeToSortPrefs",
      ],
      [
        ChannelSectionSyncManager,
        "fetchRemoteSections",
        "publishSections",
        "subscribeToSections",
      ],
      [
        ProjectSidebarMembershipSyncManager,
        "fetchRemoteMembership",
        "publishMembership",
        "subscribe",
      ],
    ]) {
      const manager = new Manager("pubkey", "wss://relay.example");
      assert.equal((await manager[fetch]()).status, "failed");
      manager[publish]({ version: 1, channels: {}, projects: {} });
      await (await manager[subscribe](() => assert.fail("remote delivery")))();
      assert.equal(fw._hasTimer(), false, Manager.name);
      manager.destroy();
    }
    const theme = new CommunityThemeSyncManager("pubkey");
    theme.publish({
      version: 1,
      theme: "macchiato",
      accent: "blue",
      followSystem: false,
    });
    assert.equal((await theme.fetchRemote()).status, "unavailable");
    assert.equal(
      (await theme.subscribeAndFetch(() => {})).result.status,
      "unavailable",
    );
    await (await theme.subscribe(() => {}))();
    assert.equal(fw._hasTimer(), false);
    theme.destroy();
    await assert.rejects(fetchReminders("pubkey"), /unavailable/);
    await assert.rejects(createReminder({}, 123), /unavailable/);
    await assert.rejects(
      publishCommunityReadState({
        client: relayClient,
        pubkey: "pubkey",
        relayUrl: "wss://relay.example",
      }),
      /unavailable/,
    );
    assert.equal(networkCalls, 0);
  } finally {
    setManagedIdentityMode(false);
    restore();
    mock.reset();
  }
  assert.equal(supportsLocalIdentityFeatures(), true);
  assert.doesNotThrow(() => requireLocalIdentityFeature("Local features"));
});
