import { expect, test, type Page } from "@playwright/test";
import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const FIRST = TEST_IDENTITIES.alice.pubkey;
const SECOND = TEST_IDENTITIES.bob.pubkey;
const AMBIGUOUS =
  "The mention @Scout is ambiguous. Choose a recipient from the mention picker.";

async function install(page: Page, channel = "general") {
  await installMockBridge(page, {
    managedAgents:
      channel === "watercooler"
        ? ["a".repeat(64), "b".repeat(64)].map((pubkey) => ({
            pubkey,
            name: "Scout",
            status: "running",
            channelNames: ["watercooler"],
          }))
        : [],
    searchProfiles: [FIRST, SECOND].map((pubkey) => ({
      pubkey,
      displayName: "Scout",
    })),
  });
  await page.goto("/");
  await page.getByTestId(`channel-${channel}`).click();
  await expect(page.getByTestId("chat-title")).toHaveText(channel);
  if (channel === "watercooler")
    await page.getByRole("button", { name: "Start a new post..." }).click();
}

async function recipients(page: Page, content: string) {
  return page.evaluate(
    (content) =>
      (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [])
        .filter((event) => event.content === content)
        .map((event) =>
          event.tags.filter((tag) => tag[0] === "p").map((tag) => tag[1]),
        ),
    content,
  );
}

for (const channel of ["general", "watercooler"]) {
  test(`ambiguous typed name is visible and preserves ${channel === "general" ? "chat" : "standalone forum"} draft`, async ({
    page,
  }) => {
    await install(page, channel);
    const input = page.getByTestId("message-input");
    await input.fill("@Scout hello");
    await input.press("Escape");
    await page.getByTestId("send-message").click();
    await expect(page.getByText(AMBIGUOUS, { exact: false })).toBeVisible();
    await expect(input).toHaveText("@Scout hello");
    expect(await recipients(page, "@Scout hello")).toEqual([]);
    await waitForAnimations(page);
    await page.screenshot({
      path: `test-results/mention-recipients/ambiguous-${channel}.png`,
    });
  });
}

test("two selected same-name members send both exact identities", async ({
  page,
}) => {
  await install(page);
  const input = page.getByTestId("message-input");
  await input.fill("@Scout");
  await page.getByTestId(`mention-suggestion-${FIRST}`).click();
  await page.keyboard.type("and @Scout");
  await page.getByTestId(`mention-suggestion-${SECOND}`).click();
  await page.keyboard.type("hello");
  const content = `@Scout and @Scout (${SECOND}) hello`;
  await expect(input).toHaveText(content);
  await page.getByTestId("send-message").click();
  await expect.poll(() => recipients(page, content)).toEqual([[FIRST, SECOND]]);
});

test("ambiguous added mention blocks editing before clearing the draft", async ({
  page,
}) => {
  await install(page);
  const input = page.getByTestId("message-input");
  await input.fill("original message for ambiguity edit");
  await input.press("Enter");
  const row = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .last();
  await expect(row).toContainText("original message for ambiguity edit");
  await row.hover();
  await row.getByRole("button", { name: "More actions" }).click();
  await page.getByRole("menuitem", { name: "Edit message" }).click();
  await input.fill("edited @Scout hello");
  await page.getByTestId("send-message").click();
  await expect(page.getByText(AMBIGUOUS, { exact: false })).toBeVisible();
  await expect(input).toHaveText("edited @Scout hello");
  await expect(page.getByTestId("edit-target")).toBeVisible();
  expect(await recipients(page, "edited @Scout hello")).toEqual([]);
});

test("same-name teammates unfurl into distinct exact-key recipients", async ({
  page,
}) => {
  const pubkeys = ["a".repeat(64), "b".repeat(64)];
  await installMockBridge(page, {
    personas: pubkeys.map((_, i) => ({
      id: `scout-${i}`,
      displayName: "Scout",
      systemPrompt: "Help.",
    })),
    managedAgents: pubkeys.map((pubkey, i) => ({
      pubkey,
      personaId: `scout-${i}`,
      name: "Scout",
      status: "running",
      channelNames: ["general"],
    })),
    teams: [
      { id: "scouts", name: "Scouts", personaIds: ["scout-0", "scout-1"] },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill("@Scouts");
  await page.getByTestId("mention-suggestion-team-scouts").click();
  await page.keyboard.type("hello");
  const content = `Scouts(@Scout @Scout (${pubkeys[1]})) hello`;
  await expect(input).toHaveText(content);
  await page.getByTestId("send-message").click();
  await expect.poll(() => recipients(page, content)).toEqual([pubkeys]);
});

for (const removal of ["delete", "audience-toggle"]) {
  test(`same-name automatic recipients: ${removal} A preserves only B at send`, async ({
    page,
  }) => {
    const [a, b] = ["a".repeat(64), "b".repeat(64)];
    await page.addInitScript(() =>
      localStorage.setItem("buzz.messages.keepMentionedAgentsPinned", "true"),
    );
    await installMockBridge(page, {
      managedAgents: [a, b].map((pubkey) => ({
        pubkey,
        name: "Scout",
        status: "running",
        channelNames: ["general"],
      })),
    });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    const input = page.getByTestId("message-input");
    await input.fill("@Scout");
    await page.getByTestId(`mention-suggestion-${a}`).click();
    await page.keyboard.type("@Scout");
    await page.getByTestId(`mention-suggestion-${b}`).click();
    await page.keyboard.type("hello");
    await expect(input).toHaveText(`@Scout @Scout (${b}) hello`);
    await expect(page.getByTestId(`composer-address-lock-${a}`)).toBeVisible();
    await expect(page.getByTestId(`composer-address-lock-${b}`)).toBeVisible();
    if (removal === "delete") {
      // Select the literal prefix through the browser DOM, then use the real
      // editor delete path. Do not replace draft state or mock the composer.
      await input.evaluate((element) => {
        const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
        const range = document.createRange();
        let remaining = "@Scout ".length;
        let node = walker.nextNode();
        if (!node) throw new Error("Missing editor text");
        range.setStart(node, 0);
        while (node) {
          const length = node.textContent?.length ?? 0;
          if (remaining <= length) {
            range.setEnd(node, remaining);
            break;
          }
          remaining -= length;
          node = walker.nextNode();
        }
        const selection = window.getSelection();
        selection?.removeAllRanges();
        selection?.addRange(range);
        element.dispatchEvent(new Event("focus"));
      });
      await input.press("Backspace");
    } else {
      await page.locator("[data-mention-picker-trigger]").click();
      await page.getByTestId(`mention-always-address-${a}`).click();
      await input.press("Escape");
    }
    const content = `@Scout (${b}) hello`;
    await expect(input).toHaveText(content);
    await expect(page.getByTestId(`composer-address-lock-${a}`)).toHaveCount(0);
    await expect(page.getByTestId(`composer-address-lock-${b}`)).toBeVisible();
    await page.getByTestId("send-message").click();
    await expect.poll(() => recipients(page, content)).toEqual([[b]]);
  });
}

test("selected duplicate labels survive send, reopen, replacement and second reopen", async ({
  page,
}) => {
  await install(page);
  const input = page.getByTestId("message-input");
  await input.fill("@Scout");
  await page.getByTestId(`mention-suggestion-${FIRST}`).click();
  await page.keyboard.type("@Scout");
  await page.getByTestId(`mention-suggestion-${SECOND}`).click();
  await page.keyboard.type("roundtrip");
  const original = `@Scout @Scout (${SECOND}) roundtrip`;
  await page.getByTestId("send-message").click();
  await expect
    .poll(() => recipients(page, original))
    .toEqual([[FIRST, SECOND]]);
  const row = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .last();
  const openEdit = async () => {
    await waitForAnimations(page);
    await row.hover();
    await row.getByRole("button", { name: "More actions" }).click();
    await page.getByRole("menuitem", { name: "Edit message" }).click();
  };
  await openEdit();
  await expect(input).toHaveText(original);
  await input.evaluate((element) => {
    const range = document.createRange();
    range.selectNodeContents(element);
    range.collapse(false);
    window.getSelection()?.removeAllRanges();
    window.getSelection()?.addRange(range);
  });
  await page.keyboard.type(" edited");
  const replacement = `${original} edited`;
  await expect(input).toHaveText(replacement);
  await page.getByTestId("send-message").click();
  // The mock bridge's edit_message path emits a mock event, not a signed-event
  // capture. Assert the real composer's native command payload and reopened UI.
  const editPayload = (content: string) =>
    page.evaluate((content) => {
      const call = window.__BUZZ_E2E_COMMAND_LOG__
        ?.filter((call) => call.command === "edit_message")
        .at(-1);
      const input = (
        call?.payload as {
          input?: {
            content: string;
            mentionTags: string[][];
            mentionPubkeys: string[];
          };
        }
      )?.input;
      return input?.content === content
        ? {
            references: input.mentionTags.map((t) => t[1]).sort(),
            notifying: input.mentionPubkeys,
          }
        : null;
    }, content);
  await expect
    .poll(() => editPayload(replacement))
    .toEqual({ references: [FIRST, SECOND].sort(), notifying: [] });
  await expect(row).toContainText("roundtrip edited");
  await openEdit();
  await expect(input).toHaveText(replacement);
  // Delete the unqualified A, leaving the qualified B binding on the next edit.
  const secondReplacement = `@Scout (${SECOND}) roundtrip edited twice`;
  await input.fill(secondReplacement);
  await page.getByTestId("send-message").click();
  await expect
    .poll(() => editPayload(secondReplacement))
    .toEqual({ references: [SECOND], notifying: [] });
});
