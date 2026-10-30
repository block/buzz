import { expect, test, type Locator, type Page } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { openGeneral, pressRowAction } from "../helpers/channelHeadReady";
import { clearComposer } from "../helpers/composer";

const SHRUG = "¯\\_(ツ)_/¯";
const SCREENSHOT_DIR = "test-results/slash-commands-core";

function mainComposer(page: Page) {
  return page
    .getByTestId("message-composer")
    .filter({ hasNot: page.getByTestId("edit-target") })
    .first();
}

async function typeIntoComposer(composer: Locator, text: string) {
  const input = composer.getByTestId("message-input");
  await input.click();
  await input.pressSequentially(text);
  return input;
}

async function pickShrugWithKeyboard(
  composer: Locator,
  page: Page,
  screenshotPath?: string,
) {
  const input = await typeIntoComposer(composer, "/sh");
  const list = composer.getByTestId("slash-command-autocomplete");
  await expect(list).toBeVisible();
  await expect(list.getByTestId("slash-command-shrug")).toHaveAccessibleName(
    /^\/shrug \[text\]/,
  );
  if (screenshotPath) {
    await waitForAnimations(page);
    await page.screenshot({
      path: screenshotPath,
      clip: { x: 300, y: 300, width: 980, height: 420 },
    });
  }
  await page.keyboard.press("Enter");
  await expect(list).toBeHidden();
  await expect(input).toHaveText("/shrug");
  return input;
}

async function openChannel(
  page: Page,
  options?: Parameters<typeof installMockBridge>[1],
) {
  await installMockBridge(page, options);
  await page.goto("/");
  await openGeneral(page);
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

test("channel composer lists commands, picks /shrug with the keyboard and sends the rendered shrug", async ({
  page,
}) => {
  await openChannel(page);
  const timeline = page.getByTestId("message-timeline");
  const composer = mainComposer(page);

  const input = await pickShrugWithKeyboard(
    composer,
    page,
    `${SCREENSHOT_DIR}/autocomplete-open.png`,
  );
  await input.pressSequentially("well");
  await input.press("Enter");

  await expect(
    timeline.getByTestId("message-body").filter({ hasText: SHRUG }).last(),
  ).toHaveText(`well ${SHRUG}`);
  await expect(input).toBeEmpty();

  await input.pressSequentially("/shrug");
  await expect(
    composer.getByTestId("slash-command-autocomplete"),
  ).toBeVisible();
  await input.press("Enter");
  await expect(input).toHaveText("/shrug");
  await input.press("Enter");
  await expect(
    timeline.getByTestId("message-body").filter({ hasText: SHRUG }),
  ).toHaveCount(2);
  await expect(input).toBeEmpty();
});

test("an unknown command sends as a normal message, and Escape closes the list", async ({
  page,
}) => {
  await openChannel(page);
  const timeline = page.getByTestId("message-timeline");
  const composer = mainComposer(page);
  const list = composer.getByTestId("slash-command-autocomplete");

  const input = await typeIntoComposer(composer, "/s");
  await expect(list).toBeVisible();
  await input.press("Escape");
  await expect(list).toBeHidden();
  await clearComposer(input);

  await input.pressSequentially("/review hello");
  await expect(list).toBeHidden();
  await input.press("Enter");
  await expect(
    timeline.getByTestId("message-body").filter({ hasText: "/review hello" }),
  ).toHaveCount(1);
  await expect(input).toBeEmpty();

  await input.pressSequentially("/usr/bin/foo");
  await expect(list).toBeHidden();
  await input.press("Enter");
  await expect(
    timeline.getByTestId("message-body").filter({ hasText: "/usr/bin/foo" }),
  ).toHaveCount(1);
});

test("an attachment keeps a command draft and shows an error instead of publishing", async ({
  page,
}) => {
  await openChannel(page, { deferredComposerUploads: true });
  const composer = mainComposer(page);
  const input = await typeIntoComposer(composer, "/shrug nope");
  const [chooser] = await Promise.all([
    page.waitForEvent("filechooser"),
    page.getByRole("button", { name: "Attach file" }).click(),
  ]);
  await chooser.setFiles({
    buffer: Buffer.alloc(16 * 1024 * 1024, 1),
    mimeType: "video/mp4",
    name: "large-video.mp4",
  });
  const queued = composer.getByTestId("composer-queued-media-attachment");
  await expect(queued).toBeVisible();
  await input.press("Enter");
  await expect(
    page.getByText("Commands can't be sent with attachments", {
      exact: false,
    }),
  ).toBeVisible();
  await expect(input).toHaveText("/shrug nope");
  await expect(queued).toBeVisible();
  await expect(
    page
      .getByTestId("message-timeline")
      .getByTestId("message-body")
      .filter({ hasText: SHRUG }),
  ).toHaveCount(0);
});

test("the edit composer shows no command list and never runs a command", async ({
  page,
}) => {
  await openChannel(page);
  const emitted = await page.evaluate(() =>
    window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
      channelName: "general",
      content: "Edit me",
    }),
  );
  if (!emitted?.id) throw new Error("mock message emit did not return an id");
  await expect(page.locator(`[data-message-id="${emitted.id}"]`)).toBeVisible();

  await pressRowAction(page.getByTestId(`more-actions-${emitted.id}`));
  await page.getByTestId(`edit-message-${emitted.id}`).click();
  await expect(page.getByTestId("edit-target")).toBeVisible();

  const input = page.getByTestId("message-input");
  await expect(input).toHaveText("Edit me");
  await clearComposer(input);
  await input.pressSequentially("/");
  await expect(page.getByTestId("slash-command-autocomplete")).toHaveCount(0);
  await input.pressSequentially("shrug");
  await input.press("Enter");
  await expect(page.getByTestId("edit-target")).toBeHidden();
  await expect(page.locator(`[data-message-id="${emitted.id}"]`)).toContainText(
    "/shrug",
  );
});

test("the thread reply composer runs /shrug as a thread reply", async ({
  page,
}) => {
  await openChannel(page);
  const timeline = page.getByTestId("message-timeline");
  const threadPanel = page.getByTestId("message-thread-panel");
  const rootMessage = timeline.getByTestId("message-row").first();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();
  await expect(threadPanel).toBeVisible();

  const composer = threadPanel.getByTestId("message-composer");
  const input = await pickShrugWithKeyboard(composer, page);
  await input.pressSequentially("in thread");
  await input.press("Enter");

  await expect(
    threadPanel
      .getByTestId("message-thread-replies")
      .getByTestId("message-body")
      .filter({ hasText: SHRUG }),
  ).toHaveText(`in thread ${SHRUG}`);
  await expect(input).toBeEmpty();
  await expect(
    timeline.getByTestId("message-body").filter({ hasText: SHRUG }),
  ).toHaveCount(0);

  await input.pressSequentially("/review thread");
  await input.press("Enter");
  await expect(
    threadPanel
      .getByTestId("message-thread-replies")
      .getByTestId("message-body")
      .filter({ hasText: "/review thread" }),
  ).toHaveCount(1);
});
