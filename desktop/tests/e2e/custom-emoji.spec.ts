import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

// Custom-emoji end-to-end guard.
//
// The composer renders a known `:shortcode:` as a real inline atom node
// (`img[data-custom-emoji]`) that selects/copies/deletes as one unit, while
// still serializing to `:shortcode:` on send. The message timeline renders the
// same shortcode as `img[data-custom-emoji]` via remarkCustomEmoji.
//
// The `:sprout:` shortcode lives in a member-authored kind:30030 set
// (d=`sprout:custom-emoji`) served by the mock bridge from two distinct
// pubkeys. `listCustomEmoji` reads every member's set over the relay WS and
// unions them (deduped by shortcode+url) into the workspace palette — which is
// live even in mock-bridge mode (the mock only intercepts Tauri commands), so
// this spec uses the simpler mock-bridge setup like messaging.spec.ts.
const SHORTCODE = "sprout";

async function openGeneral(page: import("@playwright/test").Page) {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
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
  // The composer clears after send.
  await expect(input.locator("img[data-custom-emoji]")).toHaveCount(0);
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
// then asserts the rendered reaction <img> src points at the localhost media
// proxy. On the pre-fix code the src would be the raw relay URL, so this test
// fails there — exactly the assertion that would have caught the bug.
//
// `:react:` is a relay-hosted fixture emoji (URL on the relay origin matching
// rewriteRelayUrl()'s /media/{64-hex}.{ext} pattern), and the mock bridge
// answers get_media_proxy_port with port 54321 so the rewrite resolves to a
// real localhost URL rather than the sprout-media:// fallback.

const REACTION_SHORTCODE = "react";
const MOCK_MEDIA_PROXY_PORT = 54321;
// A seeded message in `general` with a real 64-hex id — the only reactable
// target in mock mode (getReactionTargetId() requires a 64-hex `e` tag, which
// user-sent mock messages don't have). Mirrors REACTION_TARGET_CONTENT in the
// bridge.
const REACTION_TARGET_CONTENT = "React to me with a custom emoji";

test("reacting with a custom emoji renders via the localhost media proxy", async ({
  page,
}) => {
  await openGeneral(page);

  // Reveal the hover action bar on the seeded reaction-target message, then
  // open the reaction picker.
  const row = page
    .getByTestId("message-row")
    .filter({ hasText: REACTION_TARGET_CONTENT })
    .last();
  await expect(row).toBeVisible();
  await row.hover();
  await row.getByLabel("Open reactions").click();

  // emoji-mart renders inside a Shadow DOM web component. Search by shortcode
  // to surface the custom emoji, then click it.
  const picker = page.locator("em-emoji-picker");
  await picker.locator("input[type='search']").fill(REACTION_SHORTCODE);
  // Custom emoji buttons carry the shortcode as their `title` (no aria-label).
  await picker.locator(`button[title='${REACTION_SHORTCODE}']`).first().click();

  // The reaction pill renders the custom emoji as an <img alt=":react:">. Its
  // src must be the localhost proxy URL — proving rewriteRelayUrl() ran. A raw
  // relay URL here is the bug.
  const reactionImg = row.locator(`img[alt=':${REACTION_SHORTCODE}:']`);
  await expect(reactionImg).toBeVisible();
  await expect(reactionImg).toHaveAttribute(
    "src",
    new RegExp(
      `^http://localhost:${MOCK_MEDIA_PROXY_PORT}/media/[\\da-f]{64}\\.png$`,
    ),
  );

  // Toggle the reaction back off: click the pill, which fires remove_reaction
  // -> emits a kind:5 deletion targeting the reaction event. The pill must
  // disappear. Guards the mock-bridge deletion path: the reaction event needs a
  // 64-hex id, because the timeline only honors deletions whose `e` tag is
  // 64-hex (getDeletionTargets). A 32-hex reaction id leaves a stale pill here.
  await row.getByLabel(`Toggle :${REACTION_SHORTCODE}: reaction`).click();
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
  // node, NOT as literal `:sprout:` text.
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
  // <img>, not a bare `:sprout:`. The pre-fix edit path shipped no emoji tags,
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
  await picker.locator(`button[title='${REACTION_SHORTCODE}']`).first().click();

  const reactionImg = row.locator(`img[alt=':${REACTION_SHORTCODE}:']`);
  await expect(reactionImg).toBeVisible();
});
