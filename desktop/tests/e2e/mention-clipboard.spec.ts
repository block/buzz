import { expect, test, type Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

/**
 * Copying a mention and pasting it back must preserve the identity.
 *
 * The reported failure is specific to a **multi-word, non-member** display
 * name: the rendered chip drops the `@`, so a plain copy yields "John Smith",
 * and nothing downstream can tell that from two ordinary words. These tests
 * bind the production copy/paste seams — real `copy`/`cut`/`paste` DOM events
 * against the timeline and the composer — and assert both clipboard flavors:
 * a readable plain flavor with no pubkey in it, and an HTML sidecar that
 * carries one.
 */

/** `mockDisplayNames` maps this to "John Smith"; it joins no mock channel. */
const JOHN_SMITH_PUBKEY =
  "7c1f2ad0b4e93856a1d0c2f4e6b8093a5d7f1c3e5a79b1d3f5072a4c6e80931b";
/**
 * A second account this community also names "John Smith".
 *
 * Seeded per-test, and no more forged than the first: two people really can
 * share a display name, so a paste of one is not evidence about the other.
 */
const NAMESAKE_PUBKEY =
  "5e2c9b710f4a836d2b5c0e7d8a1f4c63b9d0725e1a8c3f6407d9b2e5c8a1f403";
const MENTION_LABEL = "John Smith";
const MENTION_SIGIL = `@${MENTION_LABEL}`;
const MESSAGE_BODY = "@John Smith fixed the bug";
/** A copied sentence with words either side of its mention, as most have. */
const SURROUNDED_MENTION_BODY = "Hello @John Smith fixed the bug";
/** A second copied sentence naming the same person, appended to the first. */
const SECOND_MENTION_BODY = " and @John Smith agrees";
const FORUM_REPLY_BODY = "Agreed, @John Smith should confirm";
/** The mock `general` channel — a feed item needs its UUID, not its name. */
const GENERAL_CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";
/** A pubkey must never reach the flavor an external app pastes. */
const ANY_64_HEX = /[0-9a-f]{64}/i;
/** Nobody's key — what a crafted clipboard sidecar would name instead. */
const IMPOSTOR_PUBKEY =
  "1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f";
/**
 * What a pasteboard round trip can swap a copied chip's spaces for.
 *
 * Built rather than written as an escape: a U+00A0 that decays into an
 * invisible literal is unreadable in a diff and silently changes the fixture.
 */
const NBSP = String.fromCharCode(0xa0);

type ClipboardFlavors = {
  defaultPrevented: boolean;
  html: string;
  text: string;
};

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

async function waitForMockLiveSubscription(page: Page, channelName: string) {
  await expect
    .poll(() =>
      page.evaluate(
        (currentChannelName) =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: currentChannelName,
          }) ?? false,
        channelName,
      ),
    )
    .toBe(true);
}

// The timeline renders off a `useDeferredValue` snapshot; the list wrapper
// carries `data-render-pending` until that commit lands.
async function waitForTimelineSettled(page: Page) {
  await expect(page.locator("[data-render-pending]")).toHaveCount(0);
}

async function emitMentionMessage(page: Page, channelName: string) {
  const event = await page.evaluate(
    ({ channel, content, mentionPubkey, pubkey }) =>
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: channel,
        content,
        mentionPubkeys: [mentionPubkey],
        pubkey,
      }),
    {
      channel: channelName,
      content: MESSAGE_BODY,
      mentionPubkey: JOHN_SMITH_PUBKEY,
      pubkey: TEST_IDENTITIES.alice.pubkey,
    },
  );
  if (!event) throw new Error("Mock message emitter is not installed");
  // The chip is what every copy in this file selects against, so wait for the
  // resolved identity rather than for the row. A non-member's profile is not
  // in the channel roster, so this waits on a profile round trip — give it
  // room beyond the default, since this is setup and not the assertion.
  const chip = page
    .getByTestId("message-body")
    .locator(`[data-mention-pubkey="${JOHN_SMITH_PUBKEY}"]`);
  await expect(chip).toHaveText("John Smith", { timeout: 15_000 });
  await waitForTimelineSettled(page);
  return event;
}

/**
 * Copy a range of the rendered timeline through the real `copy` event.
 *
 * `selectChip` narrows the range to the first four characters *inside* the
 * mention chip, which is how a user drags across half a name.
 */
async function copyFromTimeline(
  page: Page,
  { partialChip = false }: { partialChip?: boolean } = {},
): Promise<ClipboardFlavors> {
  return page.evaluate(
    ({ pubkey, selectPartialChip }) => {
      // Anchor on the chip, not on the first message body: `general` is seeded
      // with unrelated messages that would otherwise win the query.
      const chip = document.querySelector<HTMLElement>(
        `[data-testid="message-body"] [data-mention-pubkey="${pubkey}"]`,
      );
      if (!chip) throw new Error("Message body rendered no mention chip.");
      const body = chip.closest<HTMLElement>(".message-markdown");
      if (!body) throw new Error("Mention chip is outside a rendered body.");

      const selection = window.getSelection();
      if (!selection) throw new Error("Selection API unavailable.");
      selection.removeAllRanges();
      const range = document.createRange();
      if (selectPartialChip) {
        const walker = document.createTreeWalker(chip, NodeFilter.SHOW_TEXT);
        const label = walker.nextNode();
        if (!label?.nodeValue?.startsWith("John")) {
          throw new Error("Mention chip has no leading John text node.");
        }
        range.setStart(label, 0);
        range.setEnd(label, 4);
      } else {
        range.selectNodeContents(body);
      }
      selection.addRange(range);

      const clipboardData = new DataTransfer();
      const event = new ClipboardEvent("copy", {
        bubbles: true,
        cancelable: true,
        clipboardData,
      });
      (selectPartialChip ? chip : body).dispatchEvent(event);
      return {
        defaultPrevented: event.defaultPrevented,
        html: clipboardData.getData("text/html"),
        text: clipboardData.getData("text/plain"),
      };
    },
    { pubkey: JOHN_SMITH_PUBKEY, selectPartialChip: partialChip },
  );
}

/**
 * Seed the home inbox with a mention message and open it.
 *
 * The message mentions the viewer, which is what routes it to the inbox, and
 * John Smith, whose identity every assertion here is about. The feed item is
 * pushed separately: a live channel message alone does not enter the feed the
 * home surface reads.
 */
async function openInboxMentionItem(page: Page) {
  const item = await page.evaluate(
    ({ channelId, content, mentionPubkey, pubkey, viewerPubkey }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      const push = window.__BUZZ_E2E_PUSH_MOCK_FEED_ITEM__;
      if (!emit || !push)
        throw new Error("Mock feed helpers are not installed");
      const event = emit({
        channelName: "general",
        content,
        mentionPubkeys: [viewerPubkey, mentionPubkey],
        pubkey,
      });
      push({
        category: "mention",
        channel_id: channelId,
        channel_name: "general",
        content: event.content,
        created_at: event.created_at,
        id: event.id,
        kind: event.kind,
        pubkey: event.pubkey,
        tags: event.tags,
      });
      return event;
    },
    {
      channelId: GENERAL_CHANNEL_ID,
      content: MESSAGE_BODY,
      mentionPubkey: JOHN_SMITH_PUBKEY,
      pubkey: TEST_IDENTITIES.alice.pubkey,
      viewerPubkey: TEST_IDENTITIES.tyler.pubkey,
    },
  );

  const preview = page
    .getByTestId(`home-inbox-item-${item.id}`)
    .locator("[data-mention]");
  await expect(preview).toHaveText("John Smith");
  expect(await preview.ariaSnapshot()).toContain("John");
  await page.getByTestId(`home-inbox-item-${item.id}`).click();
  // The inbox resolves a non-member's display name off its own profile batch,
  // so wait for the chip's identity rather than for the row — same setup
  // allowance the channel-timeline seed makes.
  const chip = page
    .getByTestId("home-inbox-detail-scroll")
    .locator(`[data-mention-pubkey="${JOHN_SMITH_PUBKEY}"]`);
  await expect(chip).toHaveText("John Smith", { timeout: 15_000 });
  expect(await chip.ariaSnapshot()).toContain("John");
  return item;
}

/**
 * Copy a whole rendered forum body — post card or thread reply — through the
 * real `copy` event. The event bubbles to the surface container, which is
 * where the forum wires `handleTimelineMentionCopy`.
 */
async function copyFromForumMarkdown(
  page: Page,
  marker: string,
): Promise<ClipboardFlavors> {
  return page.evaluate((bodyMarker) => {
    const body = [
      ...document.querySelectorAll<HTMLElement>(".message-markdown"),
    ].find((candidate) => candidate.textContent?.includes(bodyMarker));
    if (!body)
      throw new Error(`No rendered forum body contains: ${bodyMarker}`);

    const selection = window.getSelection();
    if (!selection) throw new Error("Selection API unavailable.");
    selection.removeAllRanges();
    const range = document.createRange();
    range.selectNodeContents(body);
    selection.addRange(range);

    const clipboardData = new DataTransfer();
    const event = new ClipboardEvent("copy", {
      bubbles: true,
      cancelable: true,
      clipboardData,
    });
    body.dispatchEvent(event);
    return {
      defaultPrevented: event.defaultPrevented,
      html: clipboardData.getData("text/html"),
      text: clipboardData.getData("text/plain"),
    };
  }, marker);
}

/**
 * Copy the rendered body holding the mention chip inside `containerTestId`.
 *
 * The event is dispatched on the body and bubbles to that container, which is
 * where a surface wires `handleTimelineMentionCopy` — so a missing wiring shows
 * up as a declined copy rather than as a lookup failure.
 */
async function copyMentionBodyWithin(
  page: Page,
  containerTestId: string,
): Promise<ClipboardFlavors> {
  return page.evaluate(
    ({ pubkey, testId }) => {
      const container = document.querySelector<HTMLElement>(
        `[data-testid="${testId}"]`,
      );
      if (!container) throw new Error(`No container: ${testId}`);
      const chip = container.querySelector<HTMLElement>(
        `[data-mention-pubkey="${pubkey}"]`,
      );
      if (!chip) throw new Error(`${testId} rendered no mention chip.`);
      const body = chip.closest<HTMLElement>(".message-markdown");
      if (!body) throw new Error("Mention chip is outside a rendered body.");

      const selection = window.getSelection();
      if (!selection) throw new Error("Selection API unavailable.");
      selection.removeAllRanges();
      const range = document.createRange();
      range.selectNodeContents(body);
      selection.addRange(range);

      const clipboardData = new DataTransfer();
      const event = new ClipboardEvent("copy", {
        bubbles: true,
        cancelable: true,
        clipboardData,
      });
      body.dispatchEvent(event);
      return {
        defaultPrevented: event.defaultPrevented,
        html: clipboardData.getData("text/html"),
        text: clipboardData.getData("text/plain"),
      };
    },
    { pubkey: JOHN_SMITH_PUBKEY, testId: containerTestId },
  );
}

/** Copy or cut the composer's current selection through the real DOM event. */
async function copyFromComposer(
  page: Page,
  type: "copy" | "cut",
): Promise<ClipboardFlavors> {
  return page.getByTestId("message-input").evaluate((element, eventType) => {
    const clipboardData = new DataTransfer();
    const event = new ClipboardEvent(eventType, {
      bubbles: true,
      cancelable: true,
      clipboardData,
    });
    element.dispatchEvent(event);
    return {
      defaultPrevented: event.defaultPrevented,
      html: clipboardData.getData("text/html"),
      text: clipboardData.getData("text/plain"),
    };
  }, type);
}

async function pasteIntoComposer(
  page: Page,
  flavors: { html: string; text: string },
) {
  const input = page.getByTestId("message-input");
  await input.click();
  await input.evaluate((element, { html, text }) => {
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
}

/** Paste at the end of whatever the composer already holds. */
async function pasteAfterComposerText(
  page: Page,
  flavors: { html: string; text: string },
) {
  const input = page.getByTestId("message-input");
  await input.click();
  // A click lands the caret wherever it hit; the append is the point here.
  await input.press("End");
  await input.evaluate((element, { html, text }) => {
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
}

/** The flavors a Buzz copy of `body` writes for a "John Smith" chip. */
function mentionFlavors(pubkey: string, body: string) {
  const at = body.indexOf(MENTION_SIGIL);
  if (at < 0) throw new Error(`Body names nobody: ${body}`);
  return {
    html:
      '<span data-buzz-copy="markdown">' +
      body.slice(0, at) +
      `<span data-mention="" data-mention-pubkey="${pubkey}" ` +
      `data-mention-label="${MENTION_LABEL}">${MENTION_SIGIL}</span>` +
      body.slice(at + MENTION_SIGIL.length) +
      "</span>",
    text: body,
  };
}

/**
 * Pin every relay profile lookup open, or let the held ones through.
 *
 * The identity check behind a pasted mention is exactly such a lookup when the
 * person is not a member of the channel being pasted into — the case the
 * feature exists for. Holding it is what makes "the paste is still deciding" a
 * state a test can act inside rather than race.
 */
async function holdRelayProfileLookups(page: Page, hold: boolean) {
  return page.evaluate(
    (next) => window.__BUZZ_E2E_HOLD_USERS_BATCH__?.(next) ?? 0,
    hold,
  );
}

/**
 * Select the first occurrence of `wanted` in the composer.
 *
 * The DOM range a mouse drag leaves behind, in one shot: ProseMirror adopts it
 * exactly as it adopts a real drag, and whatever key is pressed next still
 * goes through the editor's own keymap. A caret walked there with arrow keys
 * silently lost presses between runs and deleted the wrong words.
 */
async function selectComposerRange(page: Page, wanted: string) {
  const selected = await page
    .getByTestId("message-input")
    .evaluate((element, needle) => {
      const selection = window.getSelection();
      if (!selection) throw new Error("Selection API unavailable.");
      const walker = document.createTreeWalker(element, NodeFilter.SHOW_TEXT);
      for (let node = walker.nextNode(); node; node = walker.nextNode()) {
        const at = node.nodeValue?.indexOf(needle) ?? -1;
        if (at < 0) continue;
        const range = document.createRange();
        range.setStart(node, at);
        range.setEnd(node, at + needle.length);
        selection.removeAllRanges();
        selection.addRange(range);
        return selection.toString();
      }
      throw new Error(`Composer holds no run reading ${needle}`);
    }, wanted);
  expect(selected).toBe(wanted);
}

/** Wait until a lookup naming `pubkey` is provably pinned open. */
async function waitForHeldProfileLookup(page: Page, pubkey: string) {
  await expect.poll(() => askedRelayAboutProfile(page, pubkey)).toBe(true);
  await expect
    .poll(() =>
      page.evaluate(() => window.__BUZZ_E2E_USERS_BATCH_PENDING__?.() ?? 0),
    )
    .toBeGreaterThan(0);
}

/**
 * The `p` tags of the outgoing message whose body is `content`.
 *
 * A DM is signed client-side and published over the socket rather than through
 * `send_channel_message`, so read the event handed to the signer.
 */
async function readSentMentionPubkeys(page: Page, content: string) {
  return page.evaluate((expectedContent) => {
    for (const entry of window.__BUZZ_E2E_COMMAND_LOG__ ?? []) {
      if (entry.command === "send_channel_message") {
        const payload = entry.payload as
          | { content?: string; mentionPubkeys?: string[] | null }
          | undefined;
        if (payload?.content !== expectedContent) continue;
        return payload.mentionPubkeys ?? [];
      }
      if (entry.command !== "sign_event") continue;
      const unsigned = entry.payload as
        | { content?: string; tags?: string[][] }
        | undefined;
      if (unsigned?.content !== expectedContent) continue;
      return (unsigned.tags ?? [])
        .filter((tag) => tag[0] === "p" && tag[1])
        .map((tag) => tag[1]);
    }
    return null;
  }, content);
}

/** Whether a profile lookup naming `pubkey` reached the backend. */
async function askedRelayAboutProfile(page: Page, pubkey: string) {
  return page.evaluate(
    (wanted) =>
      (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).some(
        (entry) =>
          entry.command === "get_users_batch" &&
          (
            (entry.payload as { pubkeys?: string[] } | undefined)?.pubkeys ?? []
          ).includes(wanted),
      ),
    pubkey,
  );
}

function expectCarriesJohnSmith(flavors: ClipboardFlavors) {
  expect(flavors.defaultPrevented).toBe(true);
  // Readable anywhere, and safe to hand an external app: the sigil is back and
  // no identifier rode along.
  expect(flavors.text).toContain("@John Smith");
  expect(flavors.text).not.toMatch(ANY_64_HEX);
  // The identity travels in the sidecar flavor instead.
  expect(flavors.html).toContain(`data-mention-pubkey="${JOHN_SMITH_PUBKEY}"`);
  expect(flavors.html).toContain('data-mention-label="John Smith"');
}

async function expectComposerChip(page: Page) {
  const input = page.getByTestId("message-input");
  await expect(input).toHaveText(MESSAGE_BODY);
  await expect(input.locator(".mention-chip")).toHaveText("John Smith");
}

test("timeline selection copy carries a multi-word mention into another channel", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");
  await emitMentionMessage(page, "general");

  const chip = page
    .getByTestId("message-row")
    .filter({ hasText: "John Smith fixed the bug" })
    .locator("[data-mention]");
  await expect(chip).toHaveAttribute("data-mention-pubkey", JOHN_SMITH_PUBKEY);
  await expect(chip).toHaveText("John Smith");

  const flavors = await copyFromTimeline(page);
  expectCarriesJohnSmith(flavors);
  expect(flavors.text.trim()).toBe(MESSAGE_BODY);

  // A DM is the destination so the send is not intercepted by the non-member
  // invite prompt — the assertion under test is the recovered `p` tag.
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");
  await pasteIntoComposer(page, flavors);
  await expectComposerChip(page);

  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-input")).toHaveText("");
  await expect
    .poll(() => readSentMentionPubkeys(page, MESSAGE_BODY))
    .toContain(JOHN_SMITH_PUBKEY);
});

test("copy message writes the identity sidecar beside readable plain text", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");
  const message = await emitMentionMessage(page, "general");

  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "John Smith fixed the bug" });
  await row.hover();
  await row.getByTestId(`more-actions-${message.id}`).click({ force: true });
  await page.getByRole("menuitem", { name: "Copy message" }).click();

  const written = await page.evaluate(
    () => window.__BUZZ_E2E_LAST_CLIPBOARD__ ?? null,
  );
  expect(written?.text).toBe(MESSAGE_BODY);
  expect(written?.text).not.toMatch(ANY_64_HEX);
  expect(written?.html).toContain(`data-mention-pubkey="${JOHN_SMITH_PUBKEY}"`);
  // "Copy message" copies Markdown source, so paste must take the text path.
  expect(written?.html).toContain('data-buzz-copy="markdown"');

  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");
  await pasteIntoComposer(page, {
    html: written?.html ?? "",
    text: written?.text ?? "",
  });
  await expectComposerChip(page);
});

test("composer copy and cut round-trip the mention they were pasted with", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");
  await emitMentionMessage(page, "general");

  const source = await copyFromTimeline(page);
  await pasteIntoComposer(page, source);
  await expectComposerChip(page);

  const input = page.getByTestId("message-input");
  await input.press("ControlOrMeta+a");
  expectCarriesJohnSmith(await copyFromComposer(page, "copy"));
  await expect(input).toHaveText(MESSAGE_BODY);

  const cut = await copyFromComposer(page, "cut");
  expectCarriesJohnSmith(cut);
  await expect(input).toHaveText("");

  // The cut flavors are a complete round trip on their own.
  await pasteIntoComposer(page, cut);
  await expectComposerChip(page);
});

test("an identity the pasted content never shows binds no name", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");

  // Any copied page can carry this: an empty span claiming a real display
  // name against a key of its choosing. A registration outlives its paste, so
  // accepting one the user cannot see would rebind the name for the session.
  await pasteIntoComposer(page, {
    html:
      '<span data-buzz-copy="markdown">' +
      `<span data-mention="" data-mention-pubkey="${IMPOSTOR_PUBKEY}" ` +
      'data-mention-label="John Smith"></span>look at this</span>',
    text: "look at this",
  });
  const input = page.getByTestId("message-input");
  await expect(input).toHaveText("look at this");

  // The name that sidecar tried to claim, written afterwards by hand.
  await input.press("ControlOrMeta+a");
  await input.press("Backspace");
  await expect(input).toHaveText("");
  await pasteIntoComposer(page, { html: "", text: MESSAGE_BODY });
  await expect(input).toHaveText(MESSAGE_BODY);
  await expect(input.locator(".mention-chip")).toHaveCount(0);

  await page.getByTestId("send-message").click();
  await expect(input).toHaveText("");
  await expect
    .poll(() => readSentMentionPubkeys(page, MESSAGE_BODY))
    .not.toBeNull();
  expect(await readSentMentionPubkeys(page, MESSAGE_BODY)).not.toContain(
    IMPOSTOR_PUBKEY,
  );
});

test("an identity vouched for only by dropped markup binds no name", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");

  // The same claim as above, hidden where ProseMirror's parser throws it away
  // rather than in an empty span. No `data-buzz-copy` marker, so this takes
  // the rich HTML branch, where the gate reads the normalized markup's text.
  const input = page.getByTestId("message-input");
  await pasteIntoComposer(page, {
    html:
      "visible <style>@John Smith </style>" +
      `<span data-mention="" data-mention-pubkey="${IMPOSTOR_PUBKEY}" ` +
      'data-mention-label="John Smith"></span>',
    text: "visible",
  });
  // The premise the gate has to share: `<style>` text is never inserted.
  await expect(input).toHaveText("visible");

  // The name that sidecar tried to claim, written afterwards by hand.
  await input.press("ControlOrMeta+a");
  await input.press("Backspace");
  await expect(input).toHaveText("");
  await pasteIntoComposer(page, { html: "", text: MESSAGE_BODY });
  await expect(input).toHaveText(MESSAGE_BODY);
  await expect(input.locator(".mention-chip")).toHaveCount(0);

  await page.getByTestId("send-message").click();
  await expect(input).toHaveText("");
  await expect
    .poll(() => readSentMentionPubkeys(page, MESSAGE_BODY))
    .not.toBeNull();
  expect(await readSentMentionPubkeys(page, MESSAGE_BODY)).not.toContain(
    IMPOSTOR_PUBKEY,
  );
});

test("a visible pair no trusted state vouches for binds no name", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");

  // Nothing is concealed this time. The record claims a real display name
  // against a key of its choosing and writes that name where the paste puts
  // it on screen, so the visibility gate has no objection. What the user sees
  // is a plausible "@John Smith"; what nothing in this community says is that
  // the key beside it is his.
  const input = page.getByTestId("message-input");
  await pasteIntoComposer(page, {
    html:
      '<span data-buzz-copy="markdown">' +
      `<span data-mention="" data-mention-pubkey="${IMPOSTOR_PUBKEY}" ` +
      'data-mention-label="John Smith">@John Smith</span> fixed the bug' +
      "</span>",
    text: MESSAGE_BODY,
  });
  await expect(input).toHaveText(MESSAGE_BODY);

  // The paste did put the question: no local directory can speak for that
  // key, so it cost a profile lookup. Waiting on it is also what gives an
  // ungated build the time it needs to light the chip below.
  await expect
    .poll(() => askedRelayAboutProfile(page, IMPOSTOR_PUBKEY))
    .toBe(true);
  await expect(input.locator(".mention-chip")).toHaveCount(0);

  await page.getByTestId("send-message").click();
  await expect(input).toHaveText("");
  await expect
    .poll(() => readSentMentionPubkeys(page, MESSAGE_BODY))
    .not.toBeNull();
  expect(await readSentMentionPubkeys(page, MESSAGE_BODY)).not.toContain(
    IMPOSTOR_PUBKEY,
  );

  // And the refusal does not outlive the paste in the other direction: the
  // name stays unbound, so writing it by hand afterwards tags nobody either.
  await pasteIntoComposer(page, { html: "", text: MESSAGE_BODY });
  await expect(input).toHaveText(MESSAGE_BODY);
  await expect(input.locator(".mention-chip")).toHaveCount(0);
});

test("forum post and reply selection copies carry the mention", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("channel-watercooler")).toBeVisible();

  // Seeded straight into the mock store before the forum mounts: forum posts
  // are fetched via `get_forum_posts`, not received over a live subscription.
  const { postId, replyId } = await page.evaluate(
    ({ postBody, replyBody, mentionPubkey, pubkey }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) throw new Error("Mock message emitter is not installed");
      const post = emit({
        channelName: "watercooler",
        content: postBody,
        kind: 45001,
        mentionPubkeys: [mentionPubkey],
        pubkey,
      });
      const reply = emit({
        channelName: "watercooler",
        content: replyBody,
        kind: 45003,
        mentionPubkeys: [mentionPubkey],
        parentEventId: post.id,
        pubkey,
      });
      return { postId: post.id, replyId: reply.id };
    },
    {
      postBody: MESSAGE_BODY,
      replyBody: FORUM_REPLY_BODY,
      mentionPubkey: JOHN_SMITH_PUBKEY,
      pubkey: TEST_IDENTITIES.alice.pubkey,
    },
  );

  await page.getByTestId("channel-watercooler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("watercooler");

  // The chip resolves a non-member's name off a profile round trip — wait for
  // the identity, not the card.
  const cardChip = page.locator(
    `.message-markdown [data-mention-pubkey="${JOHN_SMITH_PUBKEY}"]`,
  );
  await expect(cardChip).toHaveText("John Smith", { timeout: 15_000 });

  const cardFlavors = await copyFromForumMarkdown(page, "fixed the bug");
  expectCarriesJohnSmith(cardFlavors);
  expect(cardFlavors.text.trim()).toBe(MESSAGE_BODY);

  // Open the thread; both the root post and the reply render in the panel.
  await page
    .locator('[role="button"]')
    .filter({ hasText: "fixed the bug" })
    .click({ position: { x: 8, y: 8 } });
  const replyChip = page.locator(
    `[data-forum-event-id="${replyId}"] [data-mention-pubkey="${JOHN_SMITH_PUBKEY}"]`,
  );
  await expect(replyChip).toHaveText("John Smith", { timeout: 15_000 });
  await expect(page.locator(`[data-forum-event-id="${postId}"]`)).toBeVisible();

  const replyFlavors = await copyFromForumMarkdown(page, "should confirm");
  expectCarriesJohnSmith(replyFlavors);
  expect(replyFlavors.text.trim()).toBe(FORUM_REPLY_BODY);

  // Close the loop on the forum's own composer: the pasted reply re-lights
  // the chip and the send recovers the identity in its `p` tag.
  await pasteIntoComposer(page, replyFlavors);
  const input = page.getByTestId("message-input");
  await expect(input).toHaveText(FORUM_REPLY_BODY);
  await expect(input.locator(".mention-chip")).toHaveText("John Smith");

  await page.getByTestId("send-message").click();
  await expect(input).toHaveText("");
  await expect
    .poll(() => readSentMentionPubkeys(page, FORUM_REPLY_BODY))
    .toContain(JOHN_SMITH_PUBKEY);
});

test("home inbox copy message carries the mention out of the detail pane", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  const item = await openInboxMentionItem(page);

  const row = page.getByTestId("home-inbox-selected-message");
  await row.hover();
  await row.getByTestId(`more-actions-${item.id}`).click({ force: true });
  await page.getByRole("menuitem", { name: "Copy message" }).click();

  const written = await page.evaluate(
    () => window.__BUZZ_E2E_LAST_CLIPBOARD__ ?? null,
  );
  expect(written?.text).toBe(MESSAGE_BODY);
  expect(written?.text).not.toMatch(ANY_64_HEX);
  // Without the row's `profiles`, the identities resolve empty and the copy
  // writes no HTML flavor at all.
  expect(written?.html).toContain(`data-mention-pubkey="${JOHN_SMITH_PUBKEY}"`);
  expect(written?.html).toContain('data-buzz-copy="markdown"');

  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");
  await pasteIntoComposer(page, {
    html: written?.html ?? "",
    text: written?.text ?? "",
  });
  await expectComposerChip(page);

  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-input")).toHaveText("");
  await expect
    .poll(() => readSentMentionPubkeys(page, MESSAGE_BODY))
    .toContain(JOHN_SMITH_PUBKEY);
});

test("home inbox selection copy carries the mention out of the detail pane", async ({
  page,
}) => {
  await page.goto("/");
  await expect(page.getByTestId("home-inbox-list")).toBeVisible();
  await openInboxMentionItem(page);

  // Unwired, this copy falls through to the browser's default: sigil-less
  // text, no identity, nothing for a paste to bind.
  const flavors = await copyMentionBodyWithin(page, "home-inbox-detail-scroll");
  expectCarriesJohnSmith(flavors);
  expect(flavors.text.trim()).toBe(MESSAGE_BODY);

  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");
  await pasteIntoComposer(page, flavors);
  await expectComposerChip(page);

  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-input")).toHaveText("");
  await expect
    .poll(() => readSentMentionPubkeys(page, MESSAGE_BODY))
    .toContain(JOHN_SMITH_PUBKEY);
});

test("a half-selected chip copies as plain text with no identity attached", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");
  await emitMentionMessage(page, "general");

  // Selecting "John" out of "John Smith" must not invent "@John": registering
  // a truncated label would bind the wrong name to a real pubkey.
  const flavors = await copyFromTimeline(page, { partialChip: true });
  expect(flavors.defaultPrevented).toBe(false);
  expect(flavors.html).toBe("");
  expect(flavors.text).toBe("");
});

test("a chip whose spaces became NBSP in transit still pastes as a mention", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");

  // A Buzz timeline copy as the pasteboard can hand it back: the chip's spaces
  // swapped for U+00A0. The paste normalizer tolerates that when judging the
  // chip whole, so it must insert the label the chip declares — the mention
  // decorations, the visibility gate, and the send-time extractor all want the
  // label's literal characters, and none of them would find "@John<NBSP>Smith".
  await pasteIntoComposer(page, {
    html:
      '<span data-buzz-copy="rich">' +
      `<span data-mention="" data-mention-pubkey="${JOHN_SMITH_PUBKEY}" ` +
      `data-mention-label="John Smith">@John${NBSP}Smith</span>` +
      " fixed the bug</span>",
    text: `@John${NBSP}Smith fixed the bug`,
  });
  await expectComposerChip(page);

  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-input")).toHaveText("");
  await expect
    .poll(() => readSentMentionPubkeys(page, MESSAGE_BODY))
    .toContain(JOHN_SMITH_PUBKEY);
});

test("a boundary-crossing default copy pastes its chip fragment without a sigil", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");
  await emitMentionMessage(page, "general");

  // A drag from inside the chip into the sentence after it covers no chip
  // fully, so Buzz's copy handler declines and the browser's default copy
  // runs — serializing the partially covered chip element with its full
  // identity attributes around only the covered slice of its text.
  const flavors = await page.evaluate((pubkey) => {
    const chip = document.querySelector<HTMLElement>(
      `[data-testid="message-body"] [data-mention-pubkey="${pubkey}"]`,
    );
    if (!chip) throw new Error("Message body rendered no mention chip.");
    const body = chip.closest<HTMLElement>(".message-markdown");
    if (!body) throw new Error("Mention chip is outside a rendered body.");

    const textNodesUnder = (root: Node) => {
      const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      const nodes: Text[] = [];
      while (walker.nextNode()) nodes.push(walker.currentNode as Text);
      return nodes;
    };
    const label = textNodesUnder(chip).find((node) =>
      node.nodeValue?.includes("Smith"),
    );
    const tail = textNodesUnder(body).find((node) =>
      node.nodeValue?.includes("fixed the bug"),
    );
    if (!label?.nodeValue || !tail?.nodeValue) {
      throw new Error("Timeline text nodes not found.");
    }

    const selection = window.getSelection();
    if (!selection) throw new Error("Selection API unavailable.");
    selection.removeAllRanges();
    const range = document.createRange();
    // "Smith fixed the" — starts mid-chip, ends mid-sentence.
    range.setStart(label, label.nodeValue.indexOf("Smith"));
    range.setEnd(tail, tail.nodeValue.indexOf("the bug") + "the".length);
    selection.addRange(range);

    const clipboardData = new DataTransfer();
    const event = new ClipboardEvent("copy", {
      bubbles: true,
      cancelable: true,
      clipboardData,
    });
    body.dispatchEvent(event);

    // A synthetic event cannot trigger the browser's real default copy, so
    // serialize the same range the way that default does: partially covered
    // elements are cloned whole, attributes intact.
    const host = document.createElement("div");
    host.append(range.cloneContents());
    return {
      defaultPrevented: event.defaultPrevented,
      html: host.innerHTML,
      text: selection.toString(),
    };
  }, JOHN_SMITH_PUBKEY);

  // The production handler really does decline this selection…
  expect(flavors.defaultPrevented).toBe(false);
  // …and what the default path writes still claims the full identity.
  expect(flavors.html).toContain(`data-mention-pubkey="${JOHN_SMITH_PUBKEY}"`);
  expect(flavors.html).toContain('data-mention-label="John Smith"');

  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");
  await pasteIntoComposer(page, flavors);

  // The fragment pastes as the words the user copied: no invented "@Smith",
  // no chip, and the identity record has nothing visible to bind to.
  const input = page.getByTestId("message-input");
  await expect(input).toHaveText("Smith fixed the");
  await expect(input.locator(".mention-chip")).toHaveCount(0);
});

test("sending on top of a paste still being verified keeps its identity", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");

  // Paste and send immediately — the whole verification happens inside the
  // window between the two, which is a window the user can genuinely hit: the
  // lookup crosses the network and the pasted words are already on screen.
  await holdRelayProfileLookups(page, true);
  await pasteIntoComposer(
    page,
    mentionFlavors(JOHN_SMITH_PUBKEY, MESSAGE_BODY),
  );
  const input = page.getByTestId("message-input");
  await expect(input).toHaveText(MESSAGE_BODY);
  await waitForHeldProfileLookup(page, JOHN_SMITH_PUBKEY);
  // Nothing has bound yet, so a send that read the map now would tag nobody.
  await expect(input.locator(".mention-chip")).toHaveCount(0);

  await page.getByTestId("send-message").click();
  // The send waits on the answer instead of publishing without it: the words
  // are still in the composer, and nothing has gone out.
  await expect(input).toHaveText(MESSAGE_BODY);
  expect(await readSentMentionPubkeys(page, MESSAGE_BODY)).toBeNull();

  expect(await holdRelayProfileLookups(page, false)).toBeGreaterThan(0);
  await expect(input).toHaveText("");
  await expect
    .poll(() => readSentMentionPubkeys(page, MESSAGE_BODY))
    .toContain(JOHN_SMITH_PUBKEY);
});

test("a slow paste settling last does not take a newer paste's name", async ({
  page,
}) => {
  // Two accounts share "John Smith", so the label alone cannot say which key
  // owns it. The namesake is seeded per-test, which needs its own install:
  // `mock` comes from the newest init script rather than merging.
  await installMockBridge(page, {
    searchProfiles: [{ pubkey: NAMESAKE_PUBKEY, displayName: MENTION_LABEL }],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page, "general");
  // Rendering his chip here resolves the first John Smith's profile, so a
  // paste of *that* key verifies from what this client already holds.
  await emitMentionMessage(page, "general");

  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");
  const input = page.getByTestId("message-input");

  // The namesake is nobody this client has looked up, so his paste has to ask
  // the relay — and that question stays open for the rest of the test.
  await holdRelayProfileLookups(page, true);
  await pasteIntoComposer(page, mentionFlavors(NAMESAKE_PUBKEY, MESSAGE_BODY));
  await expect(input).toHaveText(MESSAGE_BODY);
  await waitForHeldProfileLookup(page, NAMESAKE_PUBKEY);

  // The second paste of the same label answers from cache while the first is
  // still deciding — both chips light, and the name is now the newer key's.
  await pasteAfterComposerText(
    page,
    mentionFlavors(JOHN_SMITH_PUBKEY, SECOND_MENTION_BODY),
  );
  const wholeBody = MESSAGE_BODY + SECOND_MENTION_BODY;
  await expect(input).toHaveText(wholeBody);
  await expect(input.locator(".mention-chip")).toHaveCount(2);

  // Now let the older answer land. It is a true answer — the relay really does
  // name that key "John Smith" — and the label really is still on screen. What
  // it is not is the newest thing the user said about the name.
  expect(await holdRelayProfileLookups(page, false)).toBeGreaterThan(0);

  await page.getByTestId("send-message").click();
  await expect(input).toHaveText("");
  await expect
    .poll(() => readSentMentionPubkeys(page, wholeBody))
    .toContain(JOHN_SMITH_PUBKEY);
  expect(await readSentMentionPubkeys(page, wholeBody)).not.toContain(
    NAMESAKE_PUBKEY,
  );
});

test("retyping the pasted mention binds nothing to the typed words", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");

  await holdRelayProfileLookups(page, true);
  await pasteIntoComposer(
    page,
    mentionFlavors(JOHN_SMITH_PUBKEY, SURROUNDED_MENTION_BODY),
  );
  const input = page.getByTestId("message-input");
  await expect(input).toHaveText(SURROUNDED_MENTION_BODY);
  await waitForHeldProfileLookup(page, JOHN_SMITH_PUBKEY);

  // Second thoughts about the name specifically: select exactly the mention
  // and write it out again. The edit is strictly inside the paste, so the
  // sentence around it is untouched and the composer reads character for
  // character as it did — but the name is now the user's own words, and their
  // own candidates are what should decide who it means.
  await selectComposerRange(page, MENTION_SIGIL);
  await input.pressSequentially(MENTION_SIGIL);
  // Hand-typing `@` opens the picker; nothing here selects from it.
  await input.press("Escape");
  await expect(input).toHaveText(SURROUNDED_MENTION_BODY);

  expect(await holdRelayProfileLookups(page, false)).toBeGreaterThan(0);
  await expect(input.locator(".mention-chip")).toHaveCount(0);

  await page.getByTestId("send-message").click();
  await expect(input).toHaveText("");
  await expect
    .poll(() => readSentMentionPubkeys(page, SURROUNDED_MENTION_BODY))
    .not.toBeNull();
  expect(
    await readSentMentionPubkeys(page, SURROUNDED_MENTION_BODY),
  ).not.toContain(JOHN_SMITH_PUBKEY);
});

test("editing a word beside a pasted mention keeps its identity", async ({
  page,
}) => {
  // The other side of the same fence. A lookup that crosses the network is
  // exactly the window a user has time to tidy the sentence in, and tidying it
  // must not silently cost the mention its pubkey — that is the non-member
  // case this whole feature exists for.
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");

  await holdRelayProfileLookups(page, true);
  await pasteIntoComposer(
    page,
    mentionFlavors(JOHN_SMITH_PUBKEY, SURROUNDED_MENTION_BODY),
  );
  const input = page.getByTestId("message-input");
  await expect(input).toHaveText(SURROUNDED_MENTION_BODY);
  await waitForHeldProfileLookup(page, JOHN_SMITH_PUBKEY);

  await selectComposerRange(page, "fixed ");
  await input.press("Backspace");
  const edited = SURROUNDED_MENTION_BODY.replace("fixed ", "");
  await expect(input).toHaveText(edited);

  expect(await holdRelayProfileLookups(page, false)).toBeGreaterThan(0);
  await expect(input.locator(".mention-chip")).toHaveText("John Smith");

  await page.getByTestId("send-message").click();
  await expect(input).toHaveText("");
  await expect
    .poll(() => readSentMentionPubkeys(page, edited))
    .toContain(JOHN_SMITH_PUBKEY);
});

test("a deleted paste binds nothing to the same name typed after it", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-bob-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("bob-tyler");

  await holdRelayProfileLookups(page, true);
  await pasteIntoComposer(
    page,
    mentionFlavors(JOHN_SMITH_PUBKEY, MESSAGE_BODY),
  );
  const input = page.getByTestId("message-input");
  await expect(input).toHaveText(MESSAGE_BODY);
  await waitForHeldProfileLookup(page, JOHN_SMITH_PUBKEY);

  // Second thoughts, mid-lookup: the paste goes away and the same sentence is
  // written out by hand. Character for character the composer reads as it did,
  // so "is this label somewhere in the composer?" still says yes — but not one
  // word of it came off the clipboard, and its identity claim is over.
  await input.press("ControlOrMeta+a");
  await input.press("Backspace");
  await expect(input).toHaveText("");
  await pasteIntoComposer(page, { html: "", text: MESSAGE_BODY });
  await expect(input).toHaveText(MESSAGE_BODY);

  expect(await holdRelayProfileLookups(page, false)).toBeGreaterThan(0);
  await expect(input.locator(".mention-chip")).toHaveCount(0);

  await page.getByTestId("send-message").click();
  await expect(input).toHaveText("");
  await expect
    .poll(() => readSentMentionPubkeys(page, MESSAGE_BODY))
    .not.toBeNull();
  expect(await readSentMentionPubkeys(page, MESSAGE_BODY)).not.toContain(
    JOHN_SMITH_PUBKEY,
  );
});
