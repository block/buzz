import { expect, type Locator } from "@playwright/test";

/**
 * Replaces a composer's draft with `text`, for a composer the app may still be
 * focusing.
 *
 * Entering edit mode (and restoring focus after a mention dialog) focuses the
 * end of the draft in deferred steps: one animation frame sets the selection
 * to the end, a nested frame runs ProseMirror's `view.focus()`, and that focus
 * schedules a DOM-selection resync. `fill()` selects the contents and then
 * inserts text in separate protocol steps; a collapse that lands between them
 * appends the new text instead of replacing the draft. Clearing first makes
 * that collapse harmless: an empty document has a single caret position.
 */
export async function replaceComposerText(input: Locator, text: string) {
  await clearComposer(input);
  await input.fill(text);
}

/**
 * Clears a composer through the editor's own keymap, retrying until it is
 * empty. Use it on a composer the app focuses itself (edit mode, a closed
 * dialog); it waits for that focus first.
 *
 * `fill("")` selects the contents through the DOM and then presses Delete.
 * ProseMirror only learns about a DOM selection when `selectionchange` fires;
 * if the key arrives first, it still sees a caret at the end of the text and
 * blocks the Delete, leaving the draft intact. Select-all as a key press goes
 * through the keymap and updates the editor's selection synchronously. A
 * queued focus-to-end can still collapse it before the Delete, which then
 * deletes nothing, so the clear repeats until the document is empty.
 */
export async function clearComposer(input: Locator) {
  await expect(input).toBeFocused();
  await expect(async () => {
    await input.press("ControlOrMeta+a");
    await input.press("Delete");
    await expect(input).toBeEmpty({ timeout: 250 });
  }).toPass({ timeout: 5_000 });
}
