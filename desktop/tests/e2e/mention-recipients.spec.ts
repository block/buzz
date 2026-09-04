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
  return page.evaluate((content) => {
    const signed = (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [])
      .filter((event) => event.content === content)
      .map((event) =>
        event.tags.filter((tag) => tag[0] === "p").map((tag) => tag[1]),
      );
    if (signed.length > 0) return signed;
    // Thread sends use native IPC in the mock bridge, not signed capture.
    return (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
      .filter((call) => call.command === "send_channel_message")
      .map(
        (call) =>
          call.payload as { content?: string; mentionPubkeys?: string[] },
      )
      .filter((payload) => payload.content === content)
      .map((payload) => payload.mentionPubkeys ?? []);
  }, content);
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
  await expect(page.getByTestId("edit-target")).toBeVisible();
  await expect(input).toHaveText("original message for ambiguity edit");
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

for (const removal of ["delete", "audience-remove", "audience-unpin"]) {
  test(`same-name automatic recipients: ${removal} A preserves exact remaining recipients`, async ({
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
    // Main retains automatic audiences only in threads, never root posts.
    await page.goto(
      "/#/channels/9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50?messageId=mock-general-welcome&thread=mock-general-welcome",
    );
    const composer = page.getByTestId("thread-composer-overlay");
    await expect(composer).toBeVisible();
    const input = composer.getByTestId("message-input");
    await input.fill("@Scout");
    await composer.getByTestId(`mention-suggestion-${a}`).click();
    await page.keyboard.type("@Scout");
    await composer.getByTestId(`mention-suggestion-${b}`).click();
    await page.keyboard.type("hello");
    await expect(input).toHaveText(`@Scout @Scout (${b}) hello`);
    await expect(
      composer.getByTestId(`composer-address-lock-${a}`),
    ).toBeVisible();
    await expect(
      composer.getByTestId(`composer-address-lock-${b}`),
    ).toBeVisible();
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
    } else if (removal === "audience-remove") {
      await composer.getByTestId(`composer-address-lock-remove-${a}`).click();
    } else {
      await composer.locator("[data-mention-picker-trigger]").click();
      await composer.getByTestId(`mention-always-address-${a}`).click();
      await input.press("Escape");
    }
    // The tray unpins without deleting an authored mention; removal deletes it.
    const content = `${removal === "audience-unpin" ? "@Scout " : ""}@Scout (${b}) hello`;
    await expect(input).toHaveText(content);
    await expect(
      composer.getByTestId(`composer-address-lock-${a}`),
    ).toHaveCount(0);
    await expect(
      composer.getByTestId(`composer-address-lock-${b}`),
    ).toBeVisible();
    await composer.getByTestId("send-message").click();
    await expect
      .poll(() => recipients(page, content))
      .toEqual([removal === "audience-unpin" ? [a, b] : [b]]);
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
    element.focus();
  });
  await expect(input).toBeFocused();
  await expect
    .poll(() =>
      input.evaluate((element) => {
        const selection = window.getSelection();
        return selection?.isCollapsed && element.contains(selection.anchorNode);
      }),
    )
    .toBe(true);
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

test("edit focus transfers after menu exit; Escape still restores the trigger", async ({
  page,
}) => {
  await install(page);
  const input = page.getByTestId("message-input");
  await input.fill("menu focus handoff");
  await input.press("Enter");
  const row = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .last();
  await expect(row).toContainText("menu focus handoff");
  await row.hover();
  const trigger = row.getByRole("button", { name: "More actions" });
  await trigger.click();
  await page.getByRole("menu").press("Escape");
  await expect(trigger).toBeFocused();

  // Hold the real Radix exit lifecycle, rather than sleeping until the race
  // happens to pass. No composer state or focus handlers are mocked.
  await page.addStyleTag({
    content: `@keyframes held-menu-exit { from { opacity: 1; } to { opacity: 0; } }
      [data-radix-menu-content][data-state="closed"] {
        animation: held-menu-exit 1s linear paused !important;
      }`,
  });
  await trigger.click();
  await page.getByRole("menuitem", { name: "Edit message" }).click();
  const closingMenu = page.locator(
    '[data-radix-menu-content][data-state="closed"]',
  );
  await expect(closingMenu).toHaveCount(1);
  await expect(page.getByTestId("edit-target")).toHaveCount(0);
  await expect(input).toHaveText("");
  // Leave the closing menu while its exit is held. Radix can still process
  // pointer-leave here; it must not own focus after the edit handoff.
  await input.hover();
  await expect(page.getByTestId("edit-target")).toHaveCount(0);
  await closingMenu.evaluate((element) => {
    const animations = element.getAnimations();
    if (!animations.length)
      throw new Error("Expected held menu exit animation");
    for (const animation of animations) animation.finish();
  });
  await expect(closingMenu).toHaveCount(0);
  await expect(input).toHaveText("menu focus handoff");
  await expect(input).toBeFocused();
  await page.keyboard.type(" edited");
  await expect(input).toHaveText("menu focus handoff edited");
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
    .toEqual({ references: [["mention", SECOND]], notifying: [SECOND] });
  await expect(row).toContainText("Scout Jones hello");
});

for (const scale of [1, 1.5]) {
  test(`exact-key chips wrap in narrow composer, sent message and reopen at ${scale}x text`, async ({
    page,
  }, testInfo) => {
    await page.setViewportSize({ width: 800, height: 900 });
    await install(page);
    await page.evaluate((scale) => {
      document.documentElement.style.fontSize = `${16 * scale}px`;
    }, scale);
    const input = page.getByTestId("message-input");
    await input.fill("@Scout");
    await page.getByTestId(`mention-suggestion-${FIRST}`).click();
    await page.keyboard.type("@Scout");
    await page.getByTestId(`mention-suggestion-${SECOND}`).click();
    await page.keyboard.type("layout journey");
    const content = `@Scout @Scout (${SECOND}) layout journey`;
    const geometry = async (host: import("@playwright/test").Locator) =>
      host.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        const chips = [...element.querySelectorAll(".mention-chip")];
        return {
          width: bounds.width,
          scrollWidth: element.scrollWidth,
          clientWidth: element.clientWidth,
          chips: chips.map((chip) => ({
            text: chip.textContent,
            literalKey: chip.classList.contains("mention-literal-key"),
            wrap: getComputedStyle(chip).overflowWrap,
            iconDisplay: getComputedStyle(chip, "::before").display,
            rects: [...chip.getClientRects()].map((r) => ({
              left: r.left - bounds.left,
              right: r.right - bounds.left,
              width: r.width,
            })),
          })),
        };
      });
    const assertFits = async (
      host: import("@playwright/test").Locator,
      stage: string,
    ) => {
      const result = await geometry(host);
      expect(result.chips.length).toBeGreaterThanOrEqual(2);
      expect(result.scrollWidth).toBeLessThanOrEqual(result.clientWidth + 1);
      for (const chip of result.chips) {
        expect(chip.wrap).toBe("anywhere");
        if (stage !== "sent") {
          expect(chip.iconDisplay).toBe(
            chip.literalKey ? "none" : "inline-block",
          );
        }
        for (const rect of chip.rects) {
          expect(rect.left).toBeGreaterThanOrEqual(-1);
          expect(rect.right).toBeLessThanOrEqual(result.width + 1);
        }
      }
      if (stage !== "sent") {
        for (const prefix of await host
          .locator(".mention-prefix-hidden")
          .all()) {
          const literalKey = await prefix.evaluate((element) =>
            element.classList.contains("mention-literal-key"),
          );
          await expect(prefix).toHaveCSS("opacity", literalKey ? "1" : "0");
          await expect(prefix).toHaveCSS(
            "display",
            literalKey ? "inline" : "inline-block",
          );
        }
      }
      await testInfo.attach(`${stage}-geometry`, {
        body: JSON.stringify(result, null, 2),
        contentType: "application/json",
      });
      await waitForAnimations(page);
      await page.screenshot({
        path: `test-results/mention-recipients/layout-${scale}-${stage}.png`,
      });
    };
    await expect(input).toHaveText(content);
    await assertFits(input, "composer");
    await page.getByTestId("send-message").click();
    await expect
      .poll(() => recipients(page, content))
      .toEqual([[FIRST, SECOND]]);
    const row = page
      .getByTestId("message-timeline")
      .getByTestId("message-row")
      .last();
    await expect(row).toContainText("layout journey");
    const markdown = row
      .locator(".message-markdown")
      .filter({ hasText: "layout journey" })
      .last();
    await assertFits(markdown, "sent");
    await expect(row.locator("[data-mention]").last()).toHaveAttribute(
      "aria-label",
      `Scout (${SECOND})`,
    );
    await row.hover();
    await row.getByRole("button", { name: "More actions" }).click();
    await page.getByRole("menuitem", { name: "Edit message" }).click();
    await expect(input).toHaveText(content);
    await assertFits(input, "reopen");
  });
}

for (const replacement of [
  "@Charlie repaired history",
  "plain repaired history",
]) {
  test(`historical ambiguous thread edit to ${replacement} reopens and forwards without stale keys`, async ({
    page,
  }) => {
    await installMockBridge(page, {
      searchProfiles: [
        { pubkey: FIRST, displayName: "Scout" },
        { pubkey: SECOND, displayName: "Scout" },
        { pubkey: TEST_IDENTITIES.charlie.pubkey, displayName: "Charlie" },
      ],
    });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await page.waitForFunction(() =>
      window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
        channelName: "general",
      }),
    );
    const ids = await page.evaluate(
      ({ first, second }) => {
        const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        if (!emit) throw new Error("Missing emitter");
        const root = emit({
          channelName: "general",
          content: "Historical edit source thread",
          pubkey: first,
        });
        const reply = emit({
          channelName: "general",
          content: "@Scout historical ambiguity",
          mentionPubkeys: [first, second],
          parentEventId: root.id,
        });
        return { root: root.id, reply: reply.id };
      },
      { first: FIRST, second: SECOND },
    );
    await page
      .locator(
        `[data-testid="message-thread-summary"][data-thread-head-id="${ids.root}"]`,
      )
      .click();
    const panel = page.getByTestId("message-thread-panel");
    const row = panel.locator(`[data-message-id="${ids.reply}"]`);
    const openMenu = async () => {
      await waitForAnimations(page);
      await row.hover();
      await row.getByTestId(`more-actions-${ids.reply}`).click();
    };
    await expect(row).toContainText("historical ambiguity");
    await openMenu();
    await page.getByRole("menuitem", { name: "Edit message" }).click();
    const input = panel.getByTestId("message-input");
    // Edit starts after the menu exit lifecycle; do not send a new thread reply
    // by filling the still-idle composer before the historical body is loaded.
    await expect(panel.getByTestId("edit-target")).toBeVisible();
    await expect(input).toHaveText("@Scout historical ambiguity");
    await input.fill(replacement);
    await panel.getByTestId("send-message").click();
    await expect(row).toContainText("repaired history");
    const expected = replacement.startsWith("@")
      ? [TEST_IDENTITIES.charlie.pubkey]
      : [];
    await expect
      .poll(() =>
        page.evaluate(() => {
          const call = window.__BUZZ_E2E_COMMAND_LOG__
            ?.filter((call) => call.command === "edit_message")
            .at(-1);
          const args = (
            call?.payload as {
              input?: { mentionTags?: string[][]; mentionPubkeys?: string[] };
            }
          )?.input;
          return {
            refs: args?.mentionTags ?? [],
            notifying: args?.mentionPubkeys ?? [],
          };
        }),
      )
      .toEqual({
        refs: expected.map((key) => ["mention", key]),
        notifying: expected,
      });
    await openMenu();
    await page.getByRole("menuitem", { name: "Edit message" }).click();
    await expect(input).toHaveText(replacement);
    // Save the unchanged reopened snapshot, then forward through the actual menu.
    await panel.getByTestId("send-message").click();
    await expect(panel.getByTestId("edit-target")).toHaveCount(0);
    await openMenu();
    await page.getByRole("menuitem", { name: "Send to channel" }).click();
    await expect.poll(() => recipients(page, replacement)).toEqual([expected]);
    await waitForAnimations(page);
    await page.screenshot({
      path: `test-results/mention-recipients/history-${expected.length ? "replacement" : "removed"}-forwarded.png`,
    });
  });
}

for (const mixed of [false, true]) {
  test(`absent-roster overlapping history (${mixed ? "mixed" : "ambiguous"}) saves, reopens and forwards only the longer alias`, async ({
    page,
  }, testInfo) => {
    const short = (mixed ? ["1"] : ["1", "2"]).map((key) => key.repeat(64));
    const long = ["3", "4"].map((key) => key.repeat(64));
    const replacement = "@Scout Jones repaired history";
    await installMockBridge(page, {
      searchProfiles: [
        ...short.map((pubkey) => ({ pubkey, displayName: "Scout" })),
        ...long.map((pubkey) => ({ pubkey, displayName: "Scout Jones" })),
      ],
    });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await page.waitForFunction(() =>
      window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
        channelName: "general",
      }),
    );
    const ids = await page.evaluate(
      ({ delivered }) => {
        const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
        if (!emit) throw new Error("Missing emitter");
        const root = emit({
          channelName: "general",
          content: "Historical edit source thread",
        });
        const reply = emit({
          channelName: "general",
          content: "@Scout and @Scout Jones historical ambiguity",
          mentionPubkeys: delivered,
          parentEventId: root.id,
        });
        return { root: root.id, reply: reply.id };
      },
      { delivered: [...short, ...long] },
    );
    await page
      .locator(
        `[data-testid="message-thread-summary"][data-thread-head-id="${ids.root}"]`,
      )
      .click();
    const panel = page.getByTestId("message-thread-panel");
    const row = panel.locator(`[data-message-id="${ids.reply}"]`);
    const openMenu = async () => {
      await waitForAnimations(page);
      await row.hover();
      await row.getByTestId(`more-actions-${ids.reply}`).click();
    };
    await expect(row).toContainText("historical ambiguity");
    await openMenu();
    await page.getByRole("menuitem", { name: "Edit message" }).click();
    const input = panel.getByTestId("message-input");
    // Edit starts after the menu exit lifecycle; do not send a new thread reply
    // by filling the still-idle composer before the historical body is loaded.
    await expect(panel.getByTestId("edit-target")).toBeVisible();
    await expect(input).toHaveText(
      "@Scout and @Scout Jones historical ambiguity",
    );
    await input.fill(replacement);
    await panel.getByTestId("send-message").click();
    await expect(row).toContainText("repaired history");
    const expected = long;
    await expect
      .poll(() =>
        page.evaluate(() => {
          const call = window.__BUZZ_E2E_COMMAND_LOG__
            ?.filter((call) => call.command === "edit_message")
            .at(-1);
          const args = (
            call?.payload as {
              input?: { mentionTags?: string[][]; mentionPubkeys?: string[] };
            }
          )?.input;
          return {
            refs: args?.mentionTags ?? [],
            notifying: args?.mentionPubkeys ?? [],
          };
        }),
      )
      .toEqual({
        refs: expected.map((key) => ["mention", key]),
        notifying: [],
      });
    await openMenu();
    await page.getByRole("menuitem", { name: "Edit message" }).click();
    await expect(input).toHaveText(replacement);
    // Save the unchanged reopened snapshot, then forward through the actual menu.
    await panel.getByTestId("send-message").click();
    await expect(panel.getByTestId("edit-target")).toHaveCount(0);
    await openMenu();
    await page.getByRole("menuitem", { name: "Send to channel" }).click();
    await expect.poll(() => recipients(page, replacement)).toEqual([expected]);
    await waitForAnimations(page);
    await page.screenshot({
      path: testInfo.outputPath(
        `absent-roster-${mixed ? "mixed" : "ambiguous"}-forwarded.png`,
      ),
    });
  });
}

for (const selection of ["picker", "automatic"]) {
  test(`qualified ${selection} selection retires a pending paste of the original label`, async ({
    page,
  }) => {
    const [a, b, pasted] = ["a".repeat(64), "b".repeat(64), "c".repeat(64)];
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
      searchProfiles: [{ pubkey: pasted, displayName: "Scout" }],
    });
    await page.goto(
      "/#/channels/9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50?messageId=mock-general-welcome&thread=mock-general-welcome",
    );
    const composer = page.getByTestId("thread-composer-overlay");
    const input = composer.getByTestId("message-input");
    await input.fill("@Scout");
    await composer.getByTestId(`mention-suggestion-${a}`).click();
    await page.evaluate(() => window.__BUZZ_E2E_HOLD_USERS_BATCH__?.(true));
    await input.evaluate((element, pubkey) => {
      const clipboardData = new DataTransfer();
      clipboardData.setData("text/plain", "@Scout pasted ");
      clipboardData.setData(
        "text/html",
        `<span data-buzz-copy="markdown"><span data-mention="" data-mention-label="Scout" data-mention-pubkey="${pubkey}">@Scout</span> pasted </span>`,
      );
      element.dispatchEvent(
        new ClipboardEvent("paste", {
          bubbles: true,
          cancelable: true,
          clipboardData,
        }),
      );
    }, pasted);
    await expect(input).toHaveText("@Scout @Scout pasted ");
    await expect
      .poll(() =>
        page.evaluate(() => window.__BUZZ_E2E_USERS_BATCH_PENDING__?.() ?? 0),
      )
      .toBeGreaterThan(0);
    if (selection === "picker") {
      await page.keyboard.type("@Scout");
      await composer.getByTestId(`mention-suggestion-${b}`).click();
    } else {
      await composer.locator("[data-mention-picker-trigger]").click();
      await composer.getByTestId(`mention-always-address-${b}`).click();
      await input.press("Escape");
    }
    await expect(input).toContainText(`@Scout (${b})`);
    const content = (await input.innerText()).trim();
    expect(
      await page.evaluate(
        () => window.__BUZZ_E2E_HOLD_USERS_BATCH__?.(false) ?? 0,
      ),
    ).toBeGreaterThan(0);
    await composer.getByTestId("send-message").click();
    await expect(input).toHaveText(`@Scout @Scout (${b}) `);
    await expect.poll(() => recipients(page, content)).toEqual([[a, b]]);
  });
}

for (const mismatchedKey of [false, true]) {
  test(`qualified chip copy/paste ${mismatchedKey ? "rejects a mismatched key" : "preserves its exact recipient"}`, async ({
    page,
  }) => {
    await install(page);
    const input = page.getByTestId("message-input");
    await input.fill("@Scout");
    await page.getByTestId(`mention-suggestion-${FIRST}`).click();
    await page.keyboard.type("and @Scout");
    await page.getByTestId(`mention-suggestion-${SECOND}`).click();
    await page.keyboard.type("qualified clipboard roundtrip");
    await page.getByTestId("send-message").click();
    const chip = page
      .getByTestId("message-row")
      .filter({ hasText: "qualified clipboard roundtrip" })
      .locator(`[data-mention-pubkey="${SECOND}"]`);
    await expect(chip).toHaveText(`Scout (${SECOND})`);
    const flavors = await chip.evaluate((element) => {
      const range = document.createRange();
      range.selectNode(element);
      const selection = window.getSelection();
      selection?.removeAllRanges();
      selection?.addRange(range);
      const clipboardData = new DataTransfer();
      const event = new ClipboardEvent("copy", {
        bubbles: true,
        cancelable: true,
        clipboardData,
      });
      element.dispatchEvent(event);
      return {
        handled: event.defaultPrevented,
        text: clipboardData.getData("text/plain"),
        html: clipboardData.getData("text/html"),
      };
    });
    expect(flavors.handled).toBe(true);
    expect(flavors.text.trim()).toBe(`@Scout (${SECOND})`);
    expect(flavors.html).toContain(`data-mention-pubkey="${SECOND}"`);
    if (mismatchedKey) {
      // Both keys have the trusted alias Scout; the qualifier must ALSO agree.
      flavors.html = flavors.html.replace(
        `data-mention-pubkey="${SECOND}"`,
        `data-mention-pubkey="${FIRST}"`,
      );
    }
    // Remount to discard source-composer selections. A channel (not a DM)
    // also avoids an automatic conversation p tag masking a lost mention.
    await page.reload();
    await page.getByTestId("channel-general").click();
    await expect(page.getByTestId("chat-title")).toHaveText("general");
    await input.focus();
    await input.evaluate((element, { text, html }) => {
      const clipboardData = new DataTransfer();
      clipboardData.setData("text/plain", text);
      clipboardData.setData("text/html", html);
      element.dispatchEvent(
        new ClipboardEvent("paste", {
          bubbles: true,
          cancelable: true,
          clipboardData,
        }),
      );
    }, flavors);
    await expect(input).toHaveText(flavors.text);
    await page.getByTestId("send-message").click();
    await expect
      .poll(() => recipients(page, flavors.text.trim()))
      .toEqual([mismatchedKey ? [] : [SECOND]]);
  });
}
