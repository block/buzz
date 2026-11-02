import { expect, type Locator } from "@playwright/test";

/**
 * Adds a Send Message step to an open workflow dialog and fills its text.
 *
 * The inspector swaps from the trigger to the new step after the click; an
 * unscoped fill can land in the trigger's message-text condition instead.
 * The "Action" header switches at once, but the trigger body stays mounted
 * through its exit animation (AnimatePresence mode="wait"), so also wait
 * for a control only the step body renders.
 */
export async function addSendMessageStep(dialog: Locator, text: string) {
  await dialog.getByRole("button", { name: "Add step", exact: true }).click();
  await dialog.page().getByRole("menuitem", { name: "Send Message" }).click();
  const stepInspector = dialog.getByTestId("workflow-node-inspector");
  await expect(
    stepInspector.getByRole("button", { name: "Action", exact: true }),
  ).toBeVisible();
  await expect(
    stepInspector.getByRole("button", { name: "Step details" }),
  ).toBeVisible();
  await stepInspector.getByLabel("Message text").fill(text);
}
