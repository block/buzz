// Regression tests: the Sound switch silences the chime without suppressing banners.

import assert from "node:assert/strict";
import { afterEach, beforeEach, describe, it } from "node:test";

// Stubs must be installed before the production modules below are imported.

const banners = [];
class StubNotification {
  static permission = "granted";
  constructor(title, options) {
    banners.push({ title, body: options?.body });
  }
  close() {}
}
globalThis.Notification = StubNotification;
globalThis.window.Notification = StubNotification;

const plays = [];
class StubAudio {
  constructor(src) {
    this.src = src;
    this.currentTime = 0;
  }
  play() {
    plays.push(this.src);
    return Promise.resolve();
  }
  pause() {}
  addEventListener() {}
}
globalThis.Audio = StubAudio;
globalThis.window.Audio = StubAudio;

const tauriMock = {
  invoke: () => new Promise(() => {}),
  transformCallback: () => Math.random(),
};
globalThis.__TAURI_INTERNALS__ = tauriMock;
globalThis.window.__TAURI_INTERNALS__ = tauriMock;

import React from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { useAppShellDesktopNotifications } from "@/app/useAppShellDesktopNotifications.ts";
import { CommunitiesProvider } from "@/features/communities/useCommunities.tsx";
import { remindersQueryKey } from "@/features/reminders/hooks.ts";
import { useReminderNotifications } from "@/features/reminders/useReminderNotifications.ts";
import { sanitizeNotificationSettings } from "./hooks.ts";
import { useFeedDesktopNotifications } from "./use-feed-desktop-notifications.ts";

const VIEWER = "a".repeat(64);
const ALICE = "b".repeat(64);
const CHANNEL = { id: "channel-1", name: "general", channelType: "stream" };
const DM_CHANNEL = { id: "dm-1", name: "Alice", channelType: "dm" };
const CHANNELS = [CHANNEL, DM_CHANNEL];

/** Complete settings with a distinct sound per slot so a play names its slot. */
function settings(overrides = {}) {
  return sanitizeNotificationSettings({
    sounds: {
      dm: "unison",
      mention: "ping",
      thread_reply: "doop",
      needs_action: "doodone",
    },
    ...overrides,
  });
}

const dmEvent = {
  id: "dm-event",
  pubkey: ALICE,
  created_at: 1,
  kind: 9,
  tags: [["h", DM_CHANNEL.id]],
  content: "hello",
  sig: "s".repeat(128),
};

const threadReplyEvent = {
  id: "reply-event",
  pubkey: ALICE,
  created_at: 2,
  kind: 9,
  tags: [
    ["h", CHANNEL.id],
    ["e", "root-event", "", "root"],
  ],
  content: "replying",
  sig: "s".repeat(128),
};

function homeFeed(mentions = []) {
  return {
    feed: { mentions, needsAction: [], activity: [], agentActivity: [] },
    meta: { since: 0, total: mentions.length, generatedAt: 0 },
  };
}

const mentionItem = {
  id: "mention-1",
  kind: 9,
  pubkey: ALICE,
  content: "@you look",
  createdAt: 3,
  channelId: CHANNEL.id,
  channelName: CHANNEL.name,
  tags: [],
  category: "mention",
};

function dueReminder(now) {
  return {
    id: "reminder-1",
    notBefore: now - 50,
    content: {
      status: "pending",
      note: "Call Alice",
      target: {
        eventId: "target-event",
        channelId: CHANNEL.id,
        preview: "the message",
        authorPubkey: ALICE,
      },
    },
    createdAt: now - 500,
    eventId: "reminder-event-1",
  };
}

function mount(Harness) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);

  return {
    queryClient,
    render: async (props) => {
      await act(async () => {
        root.render(
          React.createElement(
            QueryClientProvider,
            { client: queryClient },
            React.createElement(
              CommunitiesProvider,
              null,
              React.createElement(Harness, props),
            ),
          ),
        );
      });
    },
    unmount: async () => {
      await act(async () => {
        root.unmount();
      });
      document.body.removeChild(container);
    },
  };
}

async function settle() {
  for (let i = 0; i < 3; i++) {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 5));
    });
  }
}

const control = { handlers: null };

function AppShellHarness({ notificationSettings }) {
  control.handlers = useAppShellDesktopNotifications({
    channels: CHANNELS,
    enabled: true,
    goChannel: async () => {},
    goHome: async () => {},
    notificationSettings,
    openSearchHit: async () => {},
    pubkey: VIEWER,
  });
  return null;
}

function FeedHarness({ feed, notificationSettings }) {
  useFeedDesktopNotifications(
    feed,
    VIEWER,
    notificationSettings,
    async () => true,
    true,
    {},
    undefined,
    CHANNELS,
  );
  return null;
}

function ReminderHarness({ notificationSettings }) {
  useReminderNotifications(VIEWER, notificationSettings, CHANNELS);
  return null;
}

beforeEach(() => {
  banners.length = 0;
  plays.length = 0;
  window.localStorage.clear();
  control.handlers = null;
});

describe("Sound switch — live DM and thread-reply handlers", () => {
  let mounted;
  afterEach(async () => {
    await mounted?.unmount();
    mounted = null;
  });

  it("Sound off: a DM still posts its banner and plays no sound", async () => {
    mounted = mount(AppShellHarness);
    await mounted.render({
      notificationSettings: settings({ soundEnabled: false }),
    });

    await act(async () => {
      control.handlers.handleDmNotification(dmEvent, DM_CHANNEL);
    });
    await settle();

    assert.equal(banners.length, 1, "the DM banner must still be sent");
    assert.deepEqual(plays, [], "no sound may play while Sound is off");
  });

  it("Sound on: a DM posts its banner and plays the DM slot sound", async () => {
    mounted = mount(AppShellHarness);
    await mounted.render({ notificationSettings: settings() });

    await act(async () => {
      control.handlers.handleDmNotification(dmEvent, DM_CHANNEL);
    });
    await settle();

    assert.equal(banners.length, 1);
    assert.deepEqual(plays, ["/sounds/unison.mp3"]);
  });

  it("Sound off: a thread reply still posts its banner and plays no sound", async () => {
    mounted = mount(AppShellHarness);
    await mounted.render({
      notificationSettings: settings({ soundEnabled: false }),
    });

    await act(async () => {
      control.handlers.handleThreadReplyDesktopNotification(
        CHANNEL.id,
        threadReplyEvent,
      );
    });
    await settle();

    assert.equal(
      banners.length,
      1,
      "the thread-reply banner must still be sent",
    );
    assert.deepEqual(plays, []);
  });

  it("Sound on: a thread reply posts its banner and plays the thread slot sound", async () => {
    mounted = mount(AppShellHarness);
    await mounted.render({ notificationSettings: settings() });

    await act(async () => {
      control.handlers.handleThreadReplyDesktopNotification(
        CHANNEL.id,
        threadReplyEvent,
      );
    });
    await settle();

    assert.equal(banners.length, 1);
    assert.deepEqual(plays, ["/sounds/doop.mp3"]);
  });

  it("control: the per-event alert toggle still decides whether a DM alerts at all", async () => {
    mounted = mount(AppShellHarness);
    await mounted.render({
      notificationSettings: settings({ slotAlertsEnabled: { dm: false } }),
    });

    await act(async () => {
      control.handlers.handleDmNotification(dmEvent, DM_CHANNEL);
    });
    await settle();

    assert.equal(banners.length, 0, "a disabled event row sends no banner");
    assert.deepEqual(plays, []);
  });
});

describe("Sound switch — home-feed mention path", () => {
  let mounted;
  afterEach(async () => {
    await mounted?.unmount();
    mounted = null;
  });

  async function deliverMention(notificationSettings) {
    mounted = mount(FeedHarness);
    // First feed initialises the seen-set; the next one carries the new item.
    await mounted.render({ feed: homeFeed(), notificationSettings });
    await settle();
    await mounted.render({
      feed: homeFeed([mentionItem]),
      notificationSettings,
    });
    await settle();
  }

  it("Sound off: a mention still posts its banner and plays no sound", async () => {
    await deliverMention(settings({ soundEnabled: false }));

    assert.equal(banners.length, 1, "the mention banner must still be sent");
    assert.deepEqual(plays, []);
  });

  it("Sound on: a mention posts its banner and plays the mention slot sound", async () => {
    await deliverMention(settings());

    assert.equal(banners.length, 1);
    assert.deepEqual(plays, ["/sounds/ping.mp3"]);
  });
});

describe("Sound switch — due-reminder toast", () => {
  let mounted;
  afterEach(async () => {
    await mounted?.unmount();
    mounted = null;
  });

  async function fireDueReminder(notificationSettings) {
    const now = Math.floor(Date.now() / 1_000);
    // A watermark behind the reminder's `notBefore` makes it newly due.
    window.localStorage.setItem(
      `buzz:lastReminderCheck:${VIEWER}`,
      String(now - 100),
    );
    mounted = mount(ReminderHarness);
    mounted.queryClient.setQueryData(remindersQueryKey(VIEWER), [
      dueReminder(now),
    ]);
    await mounted.render({ notificationSettings });
    await settle();
  }

  it("Sound off: a due reminder still posts its banner and plays no sound", async () => {
    await fireDueReminder(settings({ soundEnabled: false }));

    assert.equal(banners.length, 1, "the reminder banner must still be sent");
    assert.deepEqual(plays, []);
  });

  it("Sound on: a due reminder posts its banner and plays the needs-action sound", async () => {
    await fireDueReminder(settings());

    assert.equal(banners.length, 1);
    assert.deepEqual(plays, ["/sounds/doodone.mp3"]);
  });
});
