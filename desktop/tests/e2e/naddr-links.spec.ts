import { expect, test, type Page } from "@playwright/test";
import { naddrEncode, nsecEncode } from "nostr-tools/nip19";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const LONG_FORM_IDENTIFIER = "naddr-client-entry";
const LONG_FORM_ADDRESS = {
  identifier: LONG_FORM_IDENTIFIER,
  pubkey: TEST_IDENTITIES.alice.pubkey,
  kind: 30023,
  relays: ["wss://untrusted.example"],
};
const LONG_FORM_URI = `nostr:${naddrEncode(LONG_FORM_ADDRESS)}`;
const LONG_FORM_NOTE = {
  id: "a".repeat(64),
  pubkey: TEST_IDENTITIES.alice.pubkey,
  created_at: 1_753_891_200,
  content:
    "This long-form note opens **inside Buzz** and uses the current community relay.",
  tags: [
    ["d", LONG_FORM_IDENTIFIER],
    ["title", "Opening Nostr long-form notes"],
    ["published_at", "1753891200"],
  ],
};

async function openGeneralChannel(page: Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "general",
          }) ?? false,
      ),
    )
    .toBe(true);
}

async function emitMessage(page: Page, content: string) {
  await page.evaluate(
    ({ message, pubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: message,
        pubkey,
      });
    },
    { message: content, pubkey: TEST_IDENTITIES.alice.pubkey },
  );
}

test("opens bare and explicit naddr long-form references inside Buzz", async ({
  page,
}, testInfo) => {
  await installMockBridge(page, {
    longFormNotes: [LONG_FORM_NOTE],
  });
  await openGeneralChannel(page);
  await emitMessage(
    page,
    `Read [the launch brief](${LONG_FORM_URI}) or use ${LONG_FORM_URI}.`,
  );

  const message = page
    .getByTestId("message-row")
    .filter({ hasText: "Read the launch brief" });
  const links = message.locator("[data-naddr-link]");
  await expect(links).toHaveCount(2);

  await links.first().click();
  const dialog = page.getByTestId("long-form-note-dialog");
  await expect(dialog).toBeVisible();
  await expect(page.getByTestId("long-form-note-title")).toHaveText(
    "Opening Nostr long-form notes",
  );
  await expect(page.getByTestId("long-form-note-content")).toContainText(
    "opens inside Buzz",
  );

  const commandPayloads = await page.evaluate(
    () => window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [],
  );
  expect(commandPayloads).toContainEqual({
    command: "get_long_form_note",
    payload: {
      identifier: LONG_FORM_IDENTIFIER,
      pubkey: TEST_IDENTITIES.alice.pubkey,
    },
  });
  expect(
    commandPayloads.some(
      ({ command, payload }) =>
        command === "get_long_form_note" &&
        typeof payload === "object" &&
        payload !== null &&
        "relays" in payload,
    ),
  ).toBe(false);

  await waitForAnimations(page);
  await dialog.screenshot({
    path: testInfo.outputPath("long-form-note-dialog.png"),
  });
});

test("shows not-found state for an address absent from this community", async ({
  page,
}) => {
  await installMockBridge(page, { longFormNotes: [] });
  await openGeneralChannel(page);
  await emitMessage(page, LONG_FORM_URI);

  await page.locator("[data-naddr-link]").last().click();
  await expect(page.getByTestId("long-form-note-not-found")).toContainText(
    "Not found in this community",
  );
});

test("lets a failed long-form read retry without reopening the dialog", async ({
  page,
}) => {
  await installMockBridge(page, {
    longFormNotes: [LONG_FORM_NOTE],
    longFormReadErrors: ["relay offline", null],
  });
  await openGeneralChannel(page);
  await emitMessage(page, LONG_FORM_URI);

  await page.locator("[data-naddr-link]").last().click();
  await expect(page.getByTestId("long-form-note-error")).toBeVisible();
  await page.getByRole("button", { name: "Retry" }).click();
  await expect(page.getByTestId("long-form-note-content")).toContainText(
    "opens inside Buzz",
  );
});

test("does not open unsupported or code-formatted nostr references", async ({
  page,
}) => {
  const unsupported = `nostr:${naddrEncode({
    ...LONG_FORM_ADDRESS,
    kind: 30024,
  })}`;
  await installMockBridge(page, {
    longFormNotes: [LONG_FORM_NOTE],
  });
  await openGeneralChannel(page);
  await emitMessage(
    page,
    `Unsupported: ${unsupported}\n\nCode: \`${LONG_FORM_URI}\``,
  );

  const message = page
    .getByTestId("message-row")
    .filter({ hasText: "Unsupported:" });
  await expect(message.locator("[data-naddr-link]")).toHaveCount(0);
  await expect(message.locator("code")).toContainText(LONG_FORM_URI);
});

test("does not autolink secret-key nostr references in the composer", async ({
  page,
}) => {
  await installMockBridge(page);
  await openGeneralChannel(page);

  const composer = page.getByTestId("message-input");
  const nsec = `nostr:${nsecEncode(new Uint8Array(32).fill(1))}`;
  await composer.fill(nsec);
  await composer.press("Space");
  await expect(composer.locator("a")).toHaveCount(0);
});
