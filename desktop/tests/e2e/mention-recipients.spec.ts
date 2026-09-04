import type { Editor } from "@tiptap/core";
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

test("editing to a longer typed member drops the original shorter reference", async ({
  page,
}) => {
  await installMockBridge(page, {
    searchProfiles: [
      { pubkey: FIRST, displayName: "Scout" },
      { pubkey: SECOND, displayName: "Scout Jones" },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill("@Scout");
  await page.getByTestId(`mention-suggestion-${FIRST}`).click();
  await page.keyboard.type("hello");
  await page.getByTestId("send-message").click();
  await expect.poll(() => recipients(page, "@Scout hello")).toEqual([[FIRST]]);
  const row = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .last();
  await expect(row).toContainText("Scout hello");
  await waitForAnimations(page);
  await row.hover();
  await row.getByRole("button", { name: "More actions" }).click();
  await page.getByRole("menuitem", { name: "Edit message" }).click();
  await expect(input).toHaveText("@Scout hello");
  await input.fill("@Scout Jones hello");
  await page.getByTestId("send-message").click();
  await expect
    .poll(() =>
      page.evaluate(() => {
        const call = window.__BUZZ_E2E_COMMAND_LOG__
          ?.filter((call) => call.command === "edit_message")
          .at(-1);
        const input = (
          call?.payload as {
            input?: {
              content: string;
              mentionTags?: string[][];
              mentionPubkeys: string[];
            };
          }
        )?.input;
        return input?.content === "@Scout Jones hello"
          ? {
              references: input.mentionTags ?? [],
              notifying: input.mentionPubkeys,
            }
          : null;
      }),
    )
    .toEqual({ references: [], notifying: [SECOND] });
  await expect(row).toContainText("Scout Jones hello");
});

for (const name of ["Morgarita", "claude code", "Scout"]) {
  test(`restored ${name} separator survives missing injected editor styles`, async ({
    page,
  }) => {
    const keys = ["a".repeat(64), "b".repeat(64)];
    const selected = name === "Scout" ? keys.slice(1) : keys.slice(0, 1);
    await installMockBridge(page, {
      managedAgents: keys.map((pubkey) => ({
        pubkey,
        name,
        status: "running",
        channelNames: ["general"],
      })),
    });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    const composer = page.getByTestId("channel-composer-overlay");
    const input = composer.getByTestId("message-input");
    await composer.locator("[data-mention-picker-trigger]").click();
    for (const key of name === "Scout" ? keys : selected) {
      await composer.getByTestId(`mention-always-address-${key}`).click();
    }
    if (name === "Scout") {
      await composer.getByTestId(`mention-always-address-${keys[0]}`).click();
    }
    await composer.locator("[data-mention-picker-trigger]").click();
    const initialPrefix = `@${name}${name === "Scout" ? ` (${keys[1]})` : ""} `;
    await expect(input).toHaveText(initialPrefix);
    await input.pressSequentially("hello");
    await input.press("Enter");
    await expect
      .poll(() => recipients(page, `${initialPrefix}hello`))
      .toEqual([selected]);
    // Send clears label registrations. The new draft can use the bare label,
    // but it must still bind only the surviving identity (B for Scout).
    const prefix = `@${name} `;
    await expect(input).toHaveText(prefix);
    await page.getByTestId("channel-random").click();
    await expect(page.getByTestId("chat-title")).toHaveText("random");
    // Generated-only text must not become an authored draft on navigation.
    expect(
      await page.evaluate(() =>
        Object.keys(localStorage)
          .filter((key) => key.startsWith("buzz-drafts.v2:"))
          .flatMap((key) =>
            Object.values(JSON.parse(localStorage.getItem(key) ?? "{}")),
          ),
      ),
    ).toEqual([]);
    await page.getByTestId("channel-general").click();
    await expect(input).toHaveText(prefix);
    // Check hydration before removing styles or typing: whitespace-normalized
    // toHaveText alone cannot distinguish a lost separator.
    await expect
      .poll(() =>
        input.evaluate((element) => {
          const editor = (element as HTMLElement & { editor: Editor }).editor;
          return [
            editor.state.doc.textContent,
            editor.state.selection.from,
            editor.state.selection.to,
          ];
        }),
      )
      .toEqual([prefix, prefix.length + 1, prefix.length + 1]);
    await expect(input).toBeFocused();
    await expect(input.locator(".agent-mention-highlight")).toHaveCount(
      selected.length,
    );
    // The reproduced failure had no injected TipTap whitespace stylesheet.
    // Remove only that transient dependency, not app CSS or editor selection.
    await input.evaluate(() => {
      document.querySelector("style[data-tiptap-style]")?.remove();
    });
    await expect
      .poll(() =>
        input.evaluate((element) => {
          const editor = (element as HTMLElement & { editor: Editor }).editor;
          return {
            text: editor.state.doc.textContent,
            from: editor.state.selection.from,
            to: editor.state.selection.to,
          };
        }),
      )
      .toEqual({
        text: prefix,
        from: prefix.length + 1,
        to: prefix.length + 1,
      });
    await input.pressSequentially("follow-up");
    await expect(input).toHaveText(`${prefix}follow-up`);
    await expect(input).toHaveCSS("white-space", "break-spaces");
    await expect(input.locator(".agent-mention-highlight")).toHaveCount(
      selected.length,
    );
    await input.press("Enter");
    await expect
      .poll(() => recipients(page, `${prefix}follow-up`))
      .toEqual([selected]);
    // Inspect all kind-9 publications, not just a matching body: no extra A send.
    await expect
      .poll(() =>
        page.evaluate(() => {
          type Event = { kind: number; content: string; tags: string[][] };
          const summarize = (events: Event[]) =>
            events
              .filter((event) => event.kind === 9)
              .map((event) => ({
                content: event.content,
                p: event.tags
                  .filter((tag) => tag[0] === "p")
                  .map((tag) => tag[1]),
              }));
          const wire = (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
            .filter((entry) => entry.command === "plugin:websocket|send")
            .flatMap((entry) => {
              const data = (entry.payload as { message?: { data?: string } })
                ?.message?.data;
              if (!data) return [];
              const frame = JSON.parse(data);
              return frame[0] === "EVENT" ? [frame[1]] : [];
            });
          return {
            signed: summarize(window.__BUZZ_E2E_SIGNED_EVENTS__ ?? []),
            wire: summarize(wire),
          };
        }),
      )
      .toEqual({
        signed: [
          { content: `${initialPrefix}hello`, p: selected },
          { content: `${prefix}follow-up`, p: selected },
        ],
        wire: [
          { content: `${initialPrefix}hello`, p: selected },
          { content: `${prefix}follow-up`, p: selected },
        ],
      });
  });
}
