/**
 * Screenshot + regression spec: the Goose harness drives the LLM-provider
 * control from the harness's own provider inventory.
 *
 * Before this change the provider field was a short built-in list (Anthropic,
 * OpenAI, OpenRouter, Databricks…) plus a "Custom provider…" free-text box, so
 * most of what Goose actually supports could only be selected by typing a raw
 * provider id. The catalog entry now advertises `provider_inventory`, the
 * dialog queries the harness, and the control becomes a searchable combobox
 * over the harness's list.
 *
 * The assertions pin the two halves that regress independently:
 *   - the option list comes from the harness (Ollama / Amazon Bedrock / Together
 *     AI are not in the built-in catalog), and
 *   - the list is searchable (a query filters rows out).
 */
import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const SHOTS = "test-results/screenshots-goose-provider";

async function openCreateDialog(page: import("@playwright/test").Page) {
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("open-agents-view").click();
  await page.getByTestId("new-agent-card").click();
  const dialog = page.getByTestId("persona-dialog");
  await expect(dialog).toBeVisible({ timeout: 10_000 });
  await dialog.getByRole("tab", { name: "Customize for this agent" }).click();
  return dialog;
}

test.describe("goose provider picker", () => {
  test.use({ viewport: { width: 1280, height: 900 } });

  test("lists the harness's providers in a searchable provider control", async ({
    page,
  }) => {
    await installMockBridge(page);
    const dialog = await openCreateDialog(page);

    await dialog.locator("#persona-runtime").click();
    await page.getByRole("menuitemradio", { name: "Goose" }).click();
    await waitForAnimations(page);

    const provider = dialog.locator("#persona-llm-provider");
    await expect(provider).toBeVisible({ timeout: 10_000 });
    await provider.click();

    // Harness-published providers the built-in catalog does not list.
    await expect(
      page.getByTestId("persona-llm-provider-option-ollama"),
    ).toBeVisible({ timeout: 10_000 });
    await expect(
      page.getByTestId("persona-llm-provider-option-together"),
    ).toBeVisible();
    await expect(
      page.getByTestId("persona-llm-provider-option-aws_bedrock"),
    ).toBeVisible();

    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/01-goose-provider-list.png` });

    // The harness's own rows follow the built-in ones, so scroll to the tail to
    // show them (plus the "Configured" marker on providers goose has creds for).
    await page
      .getByTestId("persona-llm-provider-option-aws_bedrock")
      .scrollIntoViewIfNeeded();
    await waitForAnimations(page);
    await page.screenshot({
      path: `${SHOTS}/02-goose-provider-harness-rows.png`,
    });

    // The control is searchable: typing filters the rows.
    await page.getByLabel("Search providers").fill("oll");
    await expect(
      page.getByTestId("persona-llm-provider-option-ollama"),
    ).toBeVisible();
    await expect(
      page.getByTestId("persona-llm-provider-option-together"),
    ).toHaveCount(0);

    await waitForAnimations(page);
    await page.screenshot({ path: `${SHOTS}/03-goose-provider-search.png` });
  });

  test("keeps the compact menu for harnesses without an inventory", async ({
    page,
  }) => {
    // buzz-agent keeps the built-in provider menu: it publishes no provider
    // inventory, so the field must NOT turn into a combobox (and must not spend
    // a subprocess spawn learning that).
    await installMockBridge(page, {
      personas: [
        {
          displayName: "Buzz Agent Definition",
          systemPrompt: "A buzz-agent-backed definition.",
          runtime: "buzz-agent",
        },
      ],
    });
    await page.goto("/");
    await page.getByTestId("open-agents-view").click();
    await page
      .getByRole("button", { name: "Open actions for Buzz Agent Definition" })
      .click();
    await page.getByRole("menuitem", { name: "Edit" }).click();

    const dialog = page.getByTestId("persona-dialog");
    await expect(dialog).toBeVisible({ timeout: 10_000 });
    await dialog.getByRole("tab", { name: "Customize for this agent" }).click();

    const provider = dialog.locator("#persona-llm-provider");
    await expect(provider).toBeVisible({ timeout: 10_000 });
    await provider.click();
    // A menu-style field renders radio rows; the combobox would render none.
    await expect(page.getByRole("menuitemradio").first()).toBeVisible();
    await expect(page.getByLabel("Search providers")).toHaveCount(0);
    await page.keyboard.press("Escape");
  });
});
