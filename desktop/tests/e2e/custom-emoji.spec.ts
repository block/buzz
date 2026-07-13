import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Custom-emoji end-to-end guard.
//
// The composer renders a known `:shortcode:` as a real inline atom node
// (`img[data-custom-emoji]`) that selects/copies/deletes as one unit, while
// still serializing to `:shortcode:` on send. The message timeline renders the
// same shortcode as `img[data-custom-emoji]` via remarkCustomEmoji.
//
// The `:buzz:` shortcode lives in a member-authored kind:30030 set
// (d=`buzz:custom-emoji`) served by the mock bridge from two distinct
// pubkeys. `listCustomEmoji` reads every member's set over the relay WS and
// unions them (deduped by shortcode+url) into the workspace palette — which is
// live even in mock-bridge mode (the mock only intercepts Tauri commands), so
// this spec uses the simpler mock-bridge setup like messaging.spec.ts.
const SHORTCODE = "buzz";

async function waitForMockLiveSubscription(
  page: import("@playwright/test").Page,
  channelName: string,
) {
  await expect
    .poll(async () =>
      page.evaluate(
        ({ ch }) =>
          (
            window as Window & {
              __BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?: (input: {
                channelName: string;
              }) => boolean;
            }
          ).__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({ channelName: ch }) ??
          false,
        { ch: channelName },
      ),
    )
    .toBe(true);
}

async function openGeneral(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
  // The mock emoji sets point at example.com placeholder URLs. Fulfill them
  // with a real image so visibility assertions still prove the emoji rendered
  // as an actual, non-broken custom-emoji image.
  await page.route("https://example.com/e2e/**", (route) =>
    route.fulfill({
      contentType: "image/svg+xml",
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><rect width="32" height="32" fill="#22c55e"/></svg>',
    }),
  );
});

test("typing a known :shortcode: renders an inline emoji node in the composer", async ({
  page,
}) => {
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  // pressSequentially (not fill) so the node input rule fires on the final ":".
  await input.pressSequentially(`:${SHORTCODE}:`);

  const node = input.locator("img[data-custom-emoji]");
  await expect(node).toHaveCount(1);
  await expect(node).toHaveAttribute("alt", `:${SHORTCODE}:`);
  await expect(node).toHaveAttribute("data-shortcode", SHORTCODE);
  // The raw text must NOT linger alongside the node.
  await expect(input).not.toContainText(`:${SHORTCODE}:`);
});

test("custom emoji deletes as a single unit (like a built-in emoji)", async ({
  page,
}) => {
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially(`hi :${SHORTCODE}:`);

  const node = input.locator("img[data-custom-emoji]");
  await expect(node).toHaveCount(1);

  // One backspace at the end removes the whole atom node, not a character of
  // hidden text.
  await input.press("Backspace");
  await expect(node).toHaveCount(0);
  await expect(input).toContainText("hi");
});

test("custom emoji round-trips through select-all + send to the timeline", async ({
  page,
}) => {
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially(`:${SHORTCODE}:`);
  await expect(input.locator("img[data-custom-emoji]")).toHaveCount(1);

  // Select-all then a single delete clears the node as one unit, proving it is
  // part of the selectable document (the bug was the caret skipping it).
  await input.press("ControlOrMeta+a");
  await input.press("Backspace");
  await expect(input.locator("img[data-custom-emoji]")).toHaveCount(0);

  // Re-enter and send: it must serialize to `:shortcode:` and re-render as an
  // <img> in the timeline (remarkCustomEmoji), not as raw text.
  await input.pressSequentially(`:${SHORTCODE}:`);
  await expect(input.locator("img[data-custom-emoji]")).toHaveCount(1);
  await page.getByTestId("send-message").click();

  const sentEmoji = page
    .getByTestId("message-timeline")
    .locator(`img[data-custom-emoji][alt=":${SHORTCODE}:"]`);
  await expect(sentEmoji.last()).toBeVisible();
  await sentEmoji.last().hover();
  await expect(page.getByText(`:${SHORTCODE}:`).last()).toBeVisible();
  // The composer clears after send.
  await expect(input.locator("img[data-custom-emoji]")).toHaveCount(0);
});

test("native emoji-only messages leave space below the author metadata", async ({
  page,
}) => {
  await openGeneral(page);

  const nativeEmoji = "😜";
  const input = page.getByTestId("message-input");
  await input.click();
  await page.keyboard.insertText(nativeEmoji);
  await page.getByTestId("send-message").click();

  const row = page
    .getByTestId("message-row")
    .filter({ hasText: nativeEmoji })
    .last();
  await expect(row).toBeVisible();

  const author = row.getByText("npub1mock...", { exact: true });
  const emojiBody = row.locator(".text-4xl").last();
  await expect(author).toBeVisible();
  await expect(emojiBody).toContainText(nativeEmoji);
  await expect(emojiBody).toBeVisible();

  await expect
    .poll(async () => {
      const authorBox = await author.boundingBox();
      const emojiBodyBox = await emojiBody.boundingBox();
      if (!authorBox || !emojiBodyBox) {
        return Number.NEGATIVE_INFINITY;
      }
      return emojiBodyBox.y - (authorBox.y + authorBox.height);
    })
    .toBeGreaterThanOrEqual(4);
});

// Regression guard for custom-emoji REACTIONS.
//
// The bug (shipped in the custom-emoji launch, PR #816): the reaction renderer
// put the relay emoji URL straight into <img src> without going through
// rewriteRelayUrl(). WKWebView bypasses WARP, so the direct relay URL gets a
// Cloudflare Access 403 and shows a broken image — even though the same emoji
// rendered fine inline in chat (that path rewrites). The chat path was covered
// by the tests above; the reaction path was not, which is why it slipped.
//
// This drives the real interactive react flow (hover -> Open reactions ->
// emoji-mart custom category) so it exercises the add_reaction Tauri command,
// then asserts the rendered reaction <img> src points at the loopback media
// proxy. On the pre-fix code the src would be the raw relay URL, so this test
// fails there — exactly the assertion that would have caught the bug.
//
// `:react:` is a relay-hosted fixture emoji (URL on the relay origin matching
// rewriteRelayUrl()'s /media/{64-hex}.{ext} pattern), and the mock bridge
// answers get_media_proxy_port with port 54321 so the rewrite resolves to a
// real 127.0.0.1 URL rather than the buzz-media:// fallback.

const REACTION_SHORTCODE = "react";
const MOCK_MEDIA_PROXY_PORT = 54321;
const SELECTED_ACTION_CLASS = /(^|\s)bg-secondary(\s|$)/;
// A seeded message in `general` with a real 64-hex id — the only reactable
// target in mock mode (getReactionTargetId() requires a 64-hex `e` tag, which
// user-sent mock messages don't have). Mirrors REACTION_TARGET_CONTENT in the
// bridge.
const REACTION_TARGET_CONTENT = "React to me with a custom emoji";

function reactionTargetRow(page: import("@playwright/test").Page) {
  return page
    .getByTestId("message-row")
    .filter({ hasText: REACTION_TARGET_CONTENT })
    .last();
}

function messageActionBar(row: import("@playwright/test").Locator) {
  return row.locator("[data-testid^='message-action-bar-']");
}

function messageReactionTrigger(row: import("@playwright/test").Locator) {
  return row.locator("[data-testid^='react-message-']");
}

async function quickReactionStorageContains(
  page: import("@playwright/test").Page,
  emoji: string,
) {
  return page.evaluate((selectedEmoji) => {
    for (let index = 0; index < window.localStorage.length; index += 1) {
      const key = window.localStorage.key(index);
      if (!key?.startsWith("buzz.quick-reaction-emojis.v1")) continue;
      if (window.localStorage.getItem(key)?.includes(selectedEmoji)) {
        return true;
      }
    }
    return false;
  }, emoji);
}

test("message quick reaction tray stays neutral after selecting a tray emoji", async ({
  page,
}) => {
  await openGeneral(page);

  const row = reactionTargetRow(page);
  await expect(row).toBeVisible();
  await row.hover();

  const quickReactionButton = row.getByRole("button", {
    name: "React with :+1:",
  });
  await expect(quickReactionButton).toBeVisible();
  await quickReactionButton.click();

  await expect(row.getByLabel("Toggle 👍 reaction")).toBeVisible();
  await row.hover();
  await expect(quickReactionButton).not.toHaveAttribute("aria-pressed", "true");
  await expect(quickReactionButton).not.toHaveClass(SELECTED_ACTION_CLASS);
  await expect(messageReactionTrigger(row)).not.toHaveClass(
    SELECTED_ACTION_CLASS,
  );
});

test("reacting with a custom emoji renders via the loopback media proxy", async ({
  page,
}) => {
  await openGeneral(page);

  // Reveal the hover action bar on the seeded reaction-target message, then
  // open the reaction picker.
  const row = reactionTargetRow(page);
  await expect(row).toBeVisible();
  await row.hover();
  await row.getByLabel("Open reactions").click();

  // emoji-mart renders inside a Shadow DOM web component. Search by shortcode
  // to surface the custom emoji, then click it.
  const picker = page.locator("em-emoji-picker");
  await picker.locator("input[type='search']").fill(REACTION_SHORTCODE);
  await picker
    .getByRole("button", { name: `:${REACTION_SHORTCODE}:` })
    .first()
    .click();

  // The reaction pill renders the custom emoji as an <img alt=":react:">. Its
  // src must be the loopback proxy URL — proving rewriteRelayUrl() ran. A raw
  // relay URL here is the bug.
  const reactionPill = row.getByLabel(
    `Toggle :${REACTION_SHORTCODE}: reaction`,
  );
  const reactionImg = reactionPill.locator(
    `img[alt=':${REACTION_SHORTCODE}:']`,
  );
  await expect(reactionImg).toBeVisible();
  await expect(reactionImg).toHaveAttribute(
    "src",
    new RegExp(
      `^http://127\\.0\\.0\\.1:${MOCK_MEDIA_PROXY_PORT}/media/[\\da-f]{64}\\.png$`,
    ),
  );

  await expect
    .poll(() => quickReactionStorageContains(page, `:${REACTION_SHORTCODE}:`))
    .toBe(true);
  await expect(
    messageActionBar(row).locator("button[title=':react:']"),
  ).toHaveCount(0);
  await expect(messageReactionTrigger(row)).not.toHaveClass(
    SELECTED_ACTION_CLASS,
  );

  const inlineAddReactionButton = row.getByLabel("Add reaction");
  await expect
    .poll(() =>
      inlineAddReactionButton.evaluate((button) => {
        return getComputedStyle(button).opacity;
      }),
    )
    .toBe("0");
  await expect
    .poll(() =>
      inlineAddReactionButton.evaluate((button) => {
        const rect = button.getBoundingClientRect();
        return `${Math.round(rect.width)}x${Math.round(rect.height)}`;
      }),
    )
    .toBe("40x28");
  await expect
    .poll(() =>
      inlineAddReactionButton.evaluate((button) => {
        return getComputedStyle(button).transitionProperty;
      }),
    )
    .not.toContain("width");
  await row.hover();
  await expect(inlineAddReactionButton).toBeVisible();
  await expect
    .poll(() =>
      inlineAddReactionButton.evaluate((button) => {
        const rect = button.getBoundingClientRect();
        return `${Math.round(rect.width)}x${Math.round(rect.height)}`;
      }),
    )
    .toBe("40x28");

  // Toggle the reaction back off: click the pill, which fires remove_reaction
  // -> emits a kind:5 deletion targeting the reaction event. The pill must
  // disappear. Guards the mock-bridge deletion path: the reaction event needs a
  // 64-hex id, because the timeline only honors deletions whose `e` tag is
  // 64-hex (getDeletionTargets). A 32-hex reaction id leaves a stale pill here.
  await reactionPill.click();
  await expect(reactionImg).toHaveCount(0);
});

// Edit-flow regression guards.
//
// Two bugs lived on the edit path and were invisible to the send-only specs
// above:
//   Bug 1 — opening a message that contains a custom emoji for editing showed
//     the literal `:shortcode:` text in the composer instead of the inline
//     image. The node only materialized via the live input rule; loading via
//     `setContent` (how edit-open seeds the composer) left it as text because
//     the customEmoji node had no markdown parse rule.
//   Bug 2 — adding a custom emoji while editing, then saving, shipped a bare
//     `:shortcode:` because the edit-save path didn't attach NIP-30 emoji tags
//     (the send path does). Without those tags the renderer can't resolve the
//     shortcode → literal text in the timeline.
//
// These drive the real interactive edit flow (More actions → Edit message →
// save) so they exercise the `edit_message` Tauri command end to end. The mock
// bridge mirrors the real relay: it emits a kind:40003 edit event carrying the
// emoji tags, and the timeline overlays it via applyEditTagOverlay.

async function openMessageEditor(
  page: import("@playwright/test").Page,
  rowText: string,
) {
  const row = page
    .getByTestId("message-row")
    .filter({ hasText: rowText })
    .last();
  await expect(row).toBeVisible();
  await row.hover();
  await row.getByLabel("More actions").click();
  await page.getByRole("menuitem", { name: "Edit message" }).click();
  // The composer enters edit mode (shows the edit-target banner).
  await expect(page.getByTestId("edit-target")).toBeVisible();
}

test("editing a message with a custom emoji shows the image, not the shortcode (Bug 1)", async ({
  page,
}) => {
  await openGeneral(page);

  // Send our own message containing a custom emoji so it is editable.
  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially(`edit-bug1 :${SHORTCODE}:`);
  await expect(input.locator("img[data-custom-emoji]")).toHaveCount(1);
  await page.getByTestId("send-message").click();
  await expect(
    page
      .getByTestId("message-timeline")
      .locator(`img[data-custom-emoji][alt=":${SHORTCODE}:"]`)
      .last(),
  ).toBeVisible();

  // Open it for editing. The composer loads via setContent — the path the
  // markdown parse rule fixes. The known shortcode must render as the inline
  // node, NOT as literal `:buzz:` text.
  await openMessageEditor(page, "edit-bug1");
  await expect(input.locator("img[data-custom-emoji]")).toHaveCount(1);
  await expect(input.locator("img[data-custom-emoji]")).toHaveAttribute(
    "alt",
    `:${SHORTCODE}:`,
  );
  // The raw shortcode text must NOT linger in the editor.
  await expect(input).not.toContainText(`:${SHORTCODE}:`);
});

test("adding a custom emoji while editing keeps the image after save (Bug 2)", async ({
  page,
}) => {
  await openGeneral(page);

  // Send a plain message we'll edit to add an emoji to.
  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("edit-bug2 plain");
  await page.getByTestId("send-message").click();
  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "edit-bug2" })
    .last();
  await expect(row).toBeVisible();
  // No emoji yet.
  await expect(row.locator("img[data-custom-emoji]")).toHaveCount(0);

  // Edit it: append a custom emoji, then save.
  await openMessageEditor(page, "edit-bug2");
  await input.click();
  await input.pressSequentially(` :${SHORTCODE}:`);
  await expect(input.locator("img[data-custom-emoji]")).toHaveCount(1);
  await page.getByTestId("send-message").click();

  // After the edit round-trips through edit_message → kind:40003 (with emoji
  // tags) → applyEditTagOverlay, the timeline must render the emoji as an
  // <img>, not a bare `:buzz:`. The pre-fix edit path shipped no emoji tags,
  // so this row would show literal text and fail here.
  await expect(
    row.locator(`img[data-custom-emoji][alt=":${SHORTCODE}:"]`),
  ).toBeVisible();
  await expect(row).not.toContainText(`:${SHORTCODE}:`);
});

// System-message reaction guard. The original bug in this PR: system messages
// (joins, topic changes, etc.) couldn't take reactions. The seeded kind:40099
// join event renders via SystemMessageRow, which now carries the reaction
// affordance. This drives the real react flow on a system row and asserts the
// pill appears — the surface the fix targeted.
test("a system message accepts a custom-emoji reaction", async ({ page }) => {
  await openGeneral(page);

  const row = page.getByTestId("system-message-row").first();
  await expect(row).toBeVisible();
  await row.hover();
  await row.getByLabel("Open reactions").click();

  const picker = page.locator("em-emoji-picker");
  await picker.locator("input[type='search']").fill(REACTION_SHORTCODE);
  await picker
    .getByRole("button", { name: `:${REACTION_SHORTCODE}:` })
    .first()
    .click();

  const reactionImg = row
    .getByLabel(`Toggle :${REACTION_SHORTCODE}: reaction`)
    .locator(`img[alt=':${REACTION_SHORTCODE}:']`);
  await expect(reactionImg).toBeVisible();
});

test("emoji picker search input has spellcheck, autocorrect, and autocapitalize disabled", async ({
  page,
}) => {
  await openGeneral(page);

  // Open the reaction picker on the seeded reactable message.
  const row = reactionTargetRow(page);
  await expect(row).toBeVisible();
  await row.hover();
  await row.getByLabel("Open reactions").click();

  // Wait for the picker to be visible, then read the shadow-root input attributes.
  const picker = page.locator("em-emoji-picker");
  await expect(picker.locator("input[type='search']")).toBeVisible();

  const attrs = await picker.evaluate((el: Element) => {
    const input = (
      el as HTMLElement & { shadowRoot: ShadowRoot }
    ).shadowRoot?.querySelector<HTMLInputElement>('input[type="search"]');
    if (!input) throw new Error("search input not found in shadow root");
    return {
      spellcheck: input.spellcheck,
      autocorrect: input.getAttribute("autocorrect"),
      autocapitalize: input.getAttribute("autocapitalize"),
    };
  });

  expect(attrs.spellcheck).toBe(false);
  expect(attrs.autocorrect).toBe("off");
  expect(attrs.autocapitalize).toBe("off");
});

// Regression guard for PR #1438: after the spellcheck fix, the shadow-DOM
// traversal must also own autofocus so Radix's focus-scope can't steal it.
// Will reported that opening the picker required a manual click before typing.
test("emoji picker search input is focused immediately on open (no manual click needed)", async ({
  page,
}) => {
  await openGeneral(page);

  // Open the reaction picker — it passes autoFocus, so the input must receive
  // focus via our shadow traversal before the user interacts.
  const row = reactionTargetRow(page);
  await expect(row).toBeVisible();
  await row.hover();
  await row.getByLabel("Open reactions").click();

  const picker = page.locator("em-emoji-picker");
  await expect(picker.locator("input[type='search']")).toBeVisible();

  // The search input must be the active element inside the shadow root.
  // If Radix's focus-scope wins instead, shadowRoot.activeElement is the
  // popover host (or null), not the input — this assertion catches that.
  const isFocused = await picker.evaluate((el: Element) => {
    const host = el as HTMLElement & { shadowRoot: ShadowRoot };
    const input = host.shadowRoot?.querySelector<HTMLInputElement>(
      'input[type="search"]',
    );
    if (!input) return false;
    return host.shadowRoot?.activeElement === input;
  });
  expect(isFocused).toBe(true);
});

test("composer emoji picker search input is focused immediately on open", async ({
  page,
}) => {
  await openGeneral(page);

  // Open the composer emoji picker via the toolbar button.
  await page.getByTestId("composer-emoji-button").click();

  const picker = page.locator("em-emoji-picker");
  await expect(picker.locator("input[type='search']")).toBeVisible();

  // The search input must be the active element inside the shadow root so
  // the user can type immediately without a manual click.
  const isFocused = await picker.evaluate((el: Element) => {
    const host = el as HTMLElement & { shadowRoot: ShadowRoot };
    const input = host.shadowRoot?.querySelector<HTMLInputElement>(
      'input[type="search"]',
    );
    if (!input) return false;
    return host.shadowRoot?.activeElement === input;
  });
  expect(isFocused).toBe(true);

  // Pick an emoji and verify focus returns to the composer message input,
  // not left stranded in the now-closed popover. The insertEmoji handler
  // calls editor.chain().focus() before setIsEmojiPickerOpen(false), so
  // the Tiptap editor owns focus before Radix's onCloseAutoFocus fires.
  await picker.getByRole("button", { name: "😀" }).first().click();
  await expect(page.getByTestId("message-input")).toBeFocused();
});

test("composer emoji picker restores editor focus on Escape dismiss", async ({
  page,
}) => {
  await openGeneral(page);

  // Open the composer emoji picker via the toolbar button.
  await page.getByTestId("composer-emoji-button").click();

  const picker = page.locator("em-emoji-picker");
  await expect(picker.locator("input[type='search']")).toBeVisible();

  // Dismiss via Escape without selecting an emoji. The onClose callback
  // in ComposerEmojiPicker must restore focus to the editor so the user
  // can keep typing without an extra click.
  await page.keyboard.press("Escape");

  await expect(page.getByTestId("message-input")).toBeFocused();
});
