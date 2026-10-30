import { expect, test, type Locator, type Page } from "@playwright/test";

import {
  KIND_HUDDLE_STARTED,
  KIND_STREAM_MESSAGE,
} from "../../src/shared/constants/kinds";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { GENERAL_CHANNEL_ID, openGeneral } from "../helpers/channelHeadReady";

const ALICE = TEST_IDENTITIES.alice.pubkey;
const BOB = TEST_IDENTITIES.bob.pubkey;
const SELF = "deadbeef".repeat(8);
const BOB_TYLER_DM_ID = "7eb9f239-9393-50b0-bd76-d85eef0511c7";
const GIF_INFO = {
  gif: { provider: "klipy", search: "/gifs/search", share: "/gifs/share" },
  supported_extensions: ["buzz-gif"],
};

const PROFILES = [
  { pubkey: ALICE, displayName: "Alice Chen" },
  { pubkey: BOB, displayName: "Bob Ross" },
];

type SignedEvent = { content: string; kind: number; tags: string[][] };

function mainComposer(page: Page) {
  return page
    .getByTestId("message-composer")
    .filter({ hasNot: page.getByTestId("edit-target") })
    .first();
}

async function typeInto(input: Locator, text: string) {
  await input.click();
  await input.pressSequentially(text);
}

async function pickMention(page: Page, label: string, pubkey: string) {
  const suggestion = page.getByTestId(`mention-suggestion-${pubkey}`);
  await expect(suggestion).toBeVisible();
  await suggestion.click();
  await expect(page.getByTestId("message-input")).toContainText(`@${label}`);
}

async function signedMessages(page: Page, content: string) {
  return page.evaluate(
    ({ expected, kind }) => {
      const events = (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? []) as SignedEvent[];
      return events.filter(
        (event) => event.kind === kind && event.content === expected,
      );
    },
    { expected: content, kind: KIND_STREAM_MESSAGE },
  );
}

const tagValues = (event: SignedEvent, name: string) =>
  event.tags.filter((tag) => tag[0] === name).map((tag) => tag[1]);

async function openChannelWithBridge(
  page: Page,
  options: Parameters<typeof installMockBridge>[1] = {},
) {
  await installMockBridge(page, { searchProfiles: PROFILES, ...options });
  await page.goto("/");
  await openGeneral(page);
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

for (const command of ["dm", "msg"]) {
  test(`/${command} opens the DM, sends the message there with the recipient tag and navigates`, async ({
    page,
  }) => {
    await openChannelWithBridge(page);
    const input = page.getByTestId("message-input");

    await typeInto(input, `/${command} @Ali`);
    await pickMention(page, "Alice Chen", ALICE);
    await input.pressSequentially("hey");
    await expect(input).toHaveText(`/${command} @Alice Chen hey`);
    await input.press("Enter");

    await expect
      .poll(async () => (await signedMessages(page, "hey")).length)
      .toBe(1);
    const [sent] = await signedMessages(page, "hey");
    const dmChannelId = tagValues(sent, "h")[0];
    expect(dmChannelId).not.toBe(GENERAL_CHANNEL_ID);
    expect(tagValues(sent, "p")).toContain(ALICE);

    await expect(page).toHaveURL(new RegExp(`/channels/${dmChannelId}`));
    await expect(
      page.getByTestId("message-timeline").getByText("hey", { exact: true }),
    ).toBeVisible();
    await expect(page.getByTestId("message-input")).toBeEmpty();
  });
}

test("/dm without a mention is a usage error and keeps the draft", async ({
  page,
}) => {
  await openChannelWithBridge(page);
  const input = page.getByTestId("message-input");
  await typeInto(input, "/dm hello there");
  await input.press("Enter");

  await expect(page.getByText("Usage: /dm @user… [message]")).toBeVisible();
  await expect(input).toHaveText("/dm hello there");
  await expect(page).toHaveURL(new RegExp(`/channels/${GENERAL_CHANNEL_ID}`));
});

test("only the leading mentions are recipients; later mentions stay in the text", async ({
  page,
}) => {
  await openChannelWithBridge(page);
  const input = page.getByTestId("message-input");

  await typeInto(input, "/dm @Ali");
  await pickMention(page, "Alice Chen", ALICE);
  await input.pressSequentially("ask @Bo");
  await pickMention(page, "Bob Ross", BOB);
  await input.pressSequentially("later");
  await input.press("Enter");

  const content = "ask @Bob Ross later";
  await expect
    .poll(async () => (await signedMessages(page, content)).length)
    .toBe(1);
  const [sent] = await signedMessages(page, content);
  expect(tagValues(sent, "p")).toContain(ALICE);
  expect(tagValues(sent, "p")).not.toContain(BOB);
  await expect(page).toHaveURL(
    new RegExp(`/channels/${tagValues(sent, "h")[0]}`),
  );
});

test("a DM with more than eight other people is refused with a visible error", async ({
  page,
}) => {
  const people = Array.from({ length: 9 }, (_, index) => ({
    pubkey: `${(index + 1).toString(16)}`.repeat(64),
    displayName: `Person${index + 1}`,
  }));
  await openChannelWithBridge(page, { searchProfiles: people });
  const input = page.getByTestId("message-input");

  await typeInto(input, "/dm ");
  for (const person of people) {
    await input.pressSequentially(`@${person.displayName}`);
    await pickMention(page, person.displayName, person.pubkey);
  }
  await input.pressSequentially("hi");
  await input.press("Enter");

  await expect(
    page.getByText("A DM can include at most 8 other people."),
  ).toBeVisible();
  await expect(input).toContainText("/dm @Person1");
  await expect(page).toHaveURL(new RegExp(`/channels/${GENERAL_CHANNEL_ID}`));
  expect(await signedMessages(page, "hi")).toEqual([]);
});

test("a failed /dm send keeps the draft and does not navigate; the retry sends once", async ({
  page,
}) => {
  await openChannelWithBridge(page, { sendMessageErrors: ["relay: blocked"] });
  const input = page.getByTestId("message-input");

  await typeInto(input, "/dm @Ali");
  await pickMention(page, "Alice Chen", ALICE);
  await input.pressSequentially("hey");
  await input.press("Enter");

  await expect(page.getByText("relay: blocked")).toBeVisible();
  await expect(input).toHaveText("/dm @Alice Chen hey");
  await expect(page).toHaveURL(new RegExp(`/channels/${GENERAL_CHANNEL_ID}`));
  expect(await signedMessages(page, "hey")).toHaveLength(1);

  await input.press("Enter");
  await expect
    .poll(async () => (await signedMessages(page, "hey")).length)
    .toBe(2);
  await expect(page).not.toHaveURL(
    new RegExp(`/channels/${GENERAL_CHANNEL_ID}`),
  );
  await expect(input).toBeEmpty();
  await expect(
    page.getByTestId("message-timeline").getByText("hey", { exact: true }),
  ).toHaveCount(1);
});

test("/dm to the DM you are already in sends there without navigating again", async ({
  page,
}) => {
  await installMockBridge(page, { searchProfiles: PROFILES });
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page).toHaveURL(new RegExp(`/channels/${BOB_TYLER_DM_ID}`));
  const historyLength = await page.evaluate(() => window.history.length);
  const input = page.getByTestId("message-input");

  await typeInto(input, "/dm @Bo");
  await pickMention(page, "Bob Ross", BOB);
  await input.pressSequentially("again");
  await input.press("Enter");

  await expect
    .poll(async () => (await signedMessages(page, "again")).length)
    .toBe(1);
  const [sent] = await signedMessages(page, "again");
  expect(tagValues(sent, "h")).toEqual([BOB_TYLER_DM_ID]);
  expect(tagValues(sent, "p")).toContain(BOB);
  await expect(input).toBeEmpty();
  expect(await page.evaluate(() => window.history.length)).toBe(historyLength);
  await expect(page).toHaveURL(new RegExp(`/channels/${BOB_TYLER_DM_ID}`));
});

test("/dm to yourself alone is a usage error", async ({ page }) => {
  await openChannelWithBridge(page, {
    searchProfiles: [{ pubkey: SELF, displayName: "Tyler Self" }],
  });
  const input = page.getByTestId("message-input");
  await typeInto(input, "/dm @Tyl");
  await pickMention(page, "Tyler Self", SELF);
  await input.pressSequentially("note");
  await input.press("Enter");
  await expect(page.getByText("Usage: /dm @user… [message]")).toBeVisible();
  await expect(input).toContainText("/dm @Tyler Self");
});

test("the /msg alias is hidden from the list but /dm is listed", async ({
  page,
}) => {
  await openChannelWithBridge(page);
  const composer = mainComposer(page);
  const list = composer.getByTestId("slash-command-autocomplete");
  const input = page.getByTestId("message-input");

  await typeInto(input, "/m");
  await expect(list).toBeHidden();
  await input.press("Backspace");
  await input.pressSequentially("d");
  await expect(list.getByTestId("slash-command-dm")).toHaveAccessibleName(
    /^\/dm @user… \[message\]/,
  );
});

async function installFakeHuddleMicrophone(page: Page) {
  await page.addInitScript(() => {
    const mediaDevices = navigator.mediaDevices ?? ({} as MediaDevices);
    if (!navigator.mediaDevices) {
      Object.defineProperty(navigator, "mediaDevices", {
        configurable: true,
        value: mediaDevices,
      });
    }
    Object.defineProperty(mediaDevices, "enumerateDevices", {
      configurable: true,
      value: async () => [
        {
          deviceId: "mock-huddle-microphone",
          groupId: "mock-huddle-audio",
          kind: "audioinput",
          label: "Mock Huddle Microphone",
          toJSON() {
            return this;
          },
        } as MediaDeviceInfo,
      ],
    });
    Object.defineProperty(mediaDevices, "getUserMedia", {
      configurable: true,
      value: async () => {
        const context = new AudioContext({ sampleRate: 48_000 });
        const oscillator = context.createOscillator();
        const gain = context.createGain();
        const destination = context.createMediaStreamDestination();
        gain.gain.value = 0;
        oscillator.connect(gain);
        gain.connect(destination);
        oscillator.start();
        return destination.stream;
      },
    });
  });
}

async function mockCommandCalls(page: Page, command: string) {
  return page.evaluate(
    (name) =>
      (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
        .filter((entry) => entry.command === name)
        .map((entry) => entry.payload as Record<string, unknown>),
    command,
  );
}

test("/huddle starts a huddle in the channel", async ({ page }) => {
  await installFakeHuddleMicrophone(page);
  await openChannelWithBridge(page);
  const input = page.getByTestId("message-input");

  await typeInto(input, "/huddle");
  await input.press("Enter");
  await input.press("Enter");

  await expect
    .poll(async () => (await mockCommandCalls(page, "start_huddle")).length)
    .toBe(1);
  const [started] = await mockCommandCalls(page, "start_huddle");
  expect(started.parentChannelId).toBe(GENERAL_CHANNEL_ID);
  await expect(input).toBeEmpty();
});

test("a failed /huddle start keeps the draft and shows the error", async ({
  page,
}) => {
  await installFakeHuddleMicrophone(page);
  await openChannelWithBridge(page);
  await page.evaluate(() => {
    const internals = (
      window as unknown as {
        __TAURI_INTERNALS__: {
          invoke: (c: string, p: unknown, o: unknown) => Promise<unknown>;
        };
      }
    ).__TAURI_INTERNALS__;
    const original = internals.invoke.bind(internals);
    internals.invoke = async (command, payload, options) => {
      if (command === "start_huddle") throw new Error("Microphone blocked");
      return original(command, payload, options);
    };
  });
  const input = page.getByTestId("message-input");

  await typeInto(input, "/huddle");
  await input.press("Enter");
  await input.press("Enter");

  await expect(page.getByText("Microphone blocked").first()).toBeVisible();
  await expect(input).toHaveText("/huddle");
});

test("/huddle joins the huddle already running in the channel", async ({
  page,
}) => {
  await installFakeHuddleMicrophone(page);
  await openChannelWithBridge(page);
  const ephemeralChannelId = "10000000-0000-4000-8000-000000000001";
  // The mock bridge has no join_huddle; answer it with the join info the app
  // reads and keep a record of the call.
  await page.evaluate(() => {
    const w = window as unknown as {
      __JOIN_HUDDLE_CALLS__: unknown[];
      __TAURI_INTERNALS__: {
        invoke: (c: string, p: unknown, o: unknown) => Promise<unknown>;
      };
    };
    w.__JOIN_HUDDLE_CALLS__ = [];
    const original = w.__TAURI_INTERNALS__.invoke.bind(w.__TAURI_INTERNALS__);
    w.__TAURI_INTERNALS__.invoke = async (command, payload, options) => {
      if (command !== "join_huddle") return original(command, payload, options);
      w.__JOIN_HUDDLE_CALLS__.push(payload);
      return {
        ephemeral_channel_id: (payload as { ephemeralChannelId: string })
          .ephemeralChannelId,
      };
    };
  });
  await expect
    .poll(() =>
      page.evaluate(
        (kind) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "general",
            kind,
          }) ?? false,
        KIND_HUDDLE_STARTED,
      ),
    )
    .toBe(true);
  await page.evaluate(
    ({ ephemeralChannelId, kind }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: JSON.stringify({ ephemeral_channel_id: ephemeralChannelId }),
        createdAt: Math.floor(Date.now() / 1000),
        id: "1".repeat(64),
        kind,
      });
    },
    { ephemeralChannelId, kind: KIND_HUDDLE_STARTED },
  );
  await expect(
    page.getByRole("button", { name: "Join active huddle (1 participant)" }),
  ).toBeVisible();
  const input = page.getByTestId("message-input");

  await typeInto(input, "/huddle");
  await input.press("Enter");
  await input.press("Enter");

  await expect(input).toBeEmpty();
  const joins = await page.evaluate(
    () =>
      (window as unknown as { __JOIN_HUDDLE_CALLS__: unknown[] })
        .__JOIN_HUDDLE_CALLS__,
  );
  expect(joins).toEqual([
    {
      parentChannelId: GENERAL_CHANNEL_ID,
      ephemeralChannelId,
      huddleThreadEventId: "1".repeat(64),
    },
  ]);
  expect(await mockCommandCalls(page, "start_huddle")).toEqual([]);
});

async function routeGifs(page: Page, info: object | null) {
  await page.route("http://localhost:3000/info", (route) =>
    route.fulfill({
      body: JSON.stringify(info ?? { supported_extensions: [] }),
      contentType: "application/nostr+json",
    }),
  );
}

test("/gif cats opens the GIF picker on the GIFs tab with the search filled in", async ({
  page,
}) => {
  const searches: string[] = [];
  await routeGifs(page, GIF_INFO);
  await page.route("http://localhost:3000/gifs/search", async (route) => {
    searches.push(JSON.parse(route.request().postData() ?? "{}").query);
    await route.fulfill({
      body: JSON.stringify({ result: true, data: { data: [] } }),
      contentType: "application/json",
    });
  });
  await openChannelWithBridge(page);
  const input = page.getByTestId("message-input");

  await typeInto(input, "/gi");
  await expect(
    mainComposer(page).getByTestId("slash-command-gif"),
  ).toBeVisible();
  await input.press("Enter");
  await input.pressSequentially("cats");
  await expect(input).toHaveText("/gif cats");
  await input.press("Enter");

  const search = page.getByRole("searchbox", { name: "Search KLIPY" });
  await expect(page.getByRole("tab", { name: "GIFs" })).toHaveAttribute(
    "data-state",
    "active",
  );
  await expect(search).toHaveValue("cats");
  await expect(search).toBeFocused();
  await expect.poll(() => searches).toEqual(["cats"]);
  await expect(input).toBeEmpty();

  await page.keyboard.press("Escape");
  await expect(search).toBeHidden();
  await typeInto(input, "/giphy");
  await input.press("Enter");
  await expect(search).toHaveValue("");
});

test("/gif is hidden from the list when the relay has no GIF endpoint", async ({
  page,
}) => {
  await routeGifs(page, null);
  await openChannelWithBridge(page);
  const input = page.getByTestId("message-input");

  await typeInto(input, "/gi");
  await expect(
    mainComposer(page).getByTestId("slash-command-autocomplete"),
  ).toBeHidden();

  await input.pressSequentially("f cats");
  await input.press("Enter");
  await expect(
    page.getByText("GIF search isn't available in this community."),
  ).toBeVisible();
  await expect(input).toHaveText("/gif cats");
  await expect(
    page.getByRole("searchbox", { name: "Search KLIPY" }),
  ).toHaveCount(0);
});

test("/gif in the thread composer opens the thread composer's picker", async ({
  page,
}) => {
  await routeGifs(page, GIF_INFO);
  await page.route("http://localhost:3000/gifs/search", (route) =>
    route.fulfill({
      body: JSON.stringify({ result: true, data: { data: [] } }),
      contentType: "application/json",
    }),
  );
  await openChannelWithBridge(page);
  const rootMessage = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .first();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();
  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();
  const input = threadPanel.getByTestId("message-input");

  await typeInto(input, "/gif dogs");
  await input.press("Enter");

  const search = page.getByRole("searchbox", { name: "Search KLIPY" });
  await expect(search).toHaveValue("dogs");
  await expect(
    threadPanel.getByTestId("composer-emoji-button"),
  ).toHaveAttribute("aria-expanded", "true");
  await expect(
    mainComposer(page).getByTestId("composer-emoji-button"),
  ).toHaveAttribute("aria-expanded", "false");
});
