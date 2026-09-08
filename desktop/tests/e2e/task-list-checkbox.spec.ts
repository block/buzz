import { expect, test, type Page } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

const OWNER = "deadbeef".repeat(8);
const ALICE =
  "953d3363262e86b770419834c53d2446409db6d918a57f8f339d495d54ab001f";
const CHANNEL_ID = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

const TASKS = [
  "Pendientes del deploy:",
  "",
  "- [ ] revisar el relay",
  "- [x] cerrar el incidente",
  "- [ ] avisar al cliente",
].join("\n");

async function openGeneral(page: Page) {
  await page.goto(`/#/channels/${CHANNEL_ID}`, {
    waitUntil: "domcontentloaded",
  });
  await expect(page.getByTestId("chat-title")).toHaveText("general");
}

async function emitTaskList(page: Page, pubkey: string) {
  const event = await page.evaluate(
    ({ content, author }) =>
      (
        window as Window & {
          __BUZZ_E2E_EMIT_MOCK_MESSAGE__?: (input: {
            channelName: string;
            content: string;
            pubkey?: string;
          }) => { id: string };
        }
      ).__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content,
        pubkey: author,
      }),
    { content: TASKS, author: pubkey },
  );
  if (!event) throw new Error("Mock message emitter is not installed");
  return event;
}

/** The three GFM checkboxes rendered by the seeded task list. */
function taskBoxes(page: Page) {
  return page.locator('[role="checkbox"]');
}

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("clicking a task checkbox on your own message persists the flip", async ({
  page,
}) => {
  await openGeneral(page);
  await emitTaskList(page, OWNER);

  const boxes = taskBoxes(page);
  await expect(boxes).toHaveCount(3);

  // Seeded state: only the middle task is done.
  await expect(boxes.nth(0)).toHaveAttribute("data-state", "unchecked");
  await expect(boxes.nth(1)).toHaveAttribute("data-state", "checked");
  await expect(boxes.nth(2)).toHaveAttribute("data-state", "unchecked");

  // Owned message → the box is a real control, not the inert render.
  await expect(boxes.nth(0)).toBeEnabled();

  await boxes.nth(0).click();

  // The flip round-trips: the click publishes an edit, the timeline overlays
  // it, and the re-rendered Markdown comes back with the marker checked. A
  // purely local toggle would not survive that path.
  await expect(boxes.nth(0)).toHaveAttribute("data-state", "checked");
  // Its neighbours are untouched — the ordinal addressed exactly one marker.
  await expect(boxes.nth(1)).toHaveAttribute("data-state", "checked");
  await expect(boxes.nth(2)).toHaveAttribute("data-state", "unchecked");

  // And unchecking returns it.
  await boxes.nth(0).click();
  await expect(boxes.nth(0)).toHaveAttribute("data-state", "unchecked");
});

test("a task checkbox on someone else's message stays inert", async ({
  page,
}) => {
  await openGeneral(page);
  await emitTaskList(page, ALICE);

  const boxes = taskBoxes(page);
  await expect(boxes).toHaveCount(3);

  // Same authz as editing: not the author, not the agent's owner, no write.
  await expect(boxes.nth(0)).toBeDisabled();
  await expect(boxes.nth(0)).toHaveAttribute("data-state", "unchecked");
});

test("a task list typed in the composer comes back as real checkboxes", async ({
  page,
}) => {
  // Answers the practical question: there is no checklist button, so a task
  // list is typed as Markdown. TipTap turns the leading "- " into a list item
  // and `getMarkdownFromEditor` un-escapes the brackets, so `[ ]` survives
  // serialization as a GFM task marker rather than literal text.
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  await input.click();
  // pressSequentially, not fill: TipTap's list input rule only fires on real
  // keystrokes.
  await input.pressSequentially("- [ ] revisar el relay");
  await page.getByTestId("send-message").click();

  const boxes = taskBoxes(page);
  await expect(boxes).toHaveCount(1);
  await expect(boxes.nth(0)).toHaveAttribute("data-state", "unchecked");
  // Sent by the mock identity, so it is immediately checkable.
  await expect(boxes.nth(0)).toBeEnabled();

  await boxes.nth(0).click();
  await expect(boxes.nth(0)).toHaveAttribute("data-state", "checked");
});

test("the composer task button builds a checklist without typing syntax", async ({
  page,
}) => {
  await openGeneral(page);

  const input = page.getByTestId("message-input");
  const addTask = page.getByTestId("message-insert-task");

  await input.click();
  await addTask.click();
  await input.pressSequentially("revisar el relay");
  // Second press continues the list instead of splicing into the first item.
  await addTask.click();
  await input.pressSequentially("cerrar el incidente");
  await page.getByTestId("send-message").click();

  const boxes = taskBoxes(page);
  await expect(boxes).toHaveCount(2);
  await expect(boxes.nth(0)).toHaveAttribute("data-state", "unchecked");
  await expect(boxes.nth(1)).toHaveAttribute("data-state", "unchecked");

  // And they are live, not decorative.
  await boxes.nth(1).click();
  await expect(boxes.nth(1)).toHaveAttribute("data-state", "checked");
  await expect(boxes.nth(0)).toHaveAttribute("data-state", "unchecked");
});
