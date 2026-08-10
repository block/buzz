import type { Page } from "@playwright/test";

/**
 * Simulate a native file drag entering the window. The backup test flow
 * listens for window-level dragenter with a "Files" payload and swaps in a
 * composer-style drop overlay over its host surface.
 */
export async function startWindowFileDrag(page: Page): Promise<void> {
  await page.evaluate(() => {
    const dataTransfer = new DataTransfer();
    dataTransfer.items.add(
      new File(["x"], "identity.ncryptsec", { type: "text/plain" }),
    );
    window.dispatchEvent(
      new DragEvent("dragenter", { dataTransfer, bubbles: true }),
    );
  });
}

/** Simulate the file drag leaving the window without dropping. */
export async function endWindowFileDrag(page: Page): Promise<void> {
  await page.evaluate(() => {
    window.dispatchEvent(new DragEvent("dragend", { bubbles: true }));
  });
}

/** Drop a text file with the given contents onto the element with `testId`. */
export async function dropFileOnTestId(
  page: Page,
  testId: string,
  contents: string,
  name = "identity.ncryptsec",
): Promise<void> {
  await page.evaluate(
    ({ testId, contents, name }) => {
      const dataTransfer = new DataTransfer();
      dataTransfer.items.add(
        new File([contents], name, { type: "text/plain" }),
      );
      const target = document.querySelector(`[data-testid="${testId}"]`);
      if (!target) throw new Error(`drop target ${testId} not found`);
      target.dispatchEvent(
        new DragEvent("drop", {
          dataTransfer,
          bubbles: true,
          cancelable: true,
        }),
      );
    },
    { testId, contents, name },
  );
}
