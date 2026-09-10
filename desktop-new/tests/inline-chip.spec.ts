import { expect, test, type Page } from "@playwright/test";

/**
 * InlineChip's rendered contract, asserted against the real specimen page so
 * these bind the shipped component rather than a test-only copy.
 */

const SPECIMEN = "/design/components/inline-chip";

async function open(page: Page) {
  await page.goto(SPECIMEN);
  await expect(
    page.getByRole("heading", { name: "InlineChip", level: 1 }),
  ).toBeVisible();
}

function group(page: Page, label: string) {
  return page
    .locator(".component-specimen-group")
    .filter({ has: page.getByRole("heading", { name: label, exact: true }) });
}

test("draws every kind with its sigil, and an icon only where needed", async ({
  page,
}) => {
  await open(page);
  const kinds = group(page, "Kinds");

  await expect(kinds.locator(".inline-chip-label")).toHaveText([
    "@Morgan Martin",
    "@Morgarita",
    "#buzz-team",
    "buzz-team",
    "github.com/block/buzz",
  ]);

  // A person, agent, and channel are identified by their sigil, so adding a
  // glyph would say the same thing twice. A message and link have no sigil.
  const iconed = kinds.locator(".inline-chip", {
    has: page.locator(".inline-chip-icon"),
  });
  await expect(iconed).toHaveCount(2);
  await expect(iconed.first()).toHaveAttribute("data-kind", "message");
  await expect(iconed.last()).toHaveAttribute("data-kind", "link");
});

test("names itself for assistive technology without speaking an identity", async ({
  page,
}) => {
  await open(page);

  await expect(
    group(page, "Kinds").locator(".inline-chip").first(),
  ).toHaveAttribute("aria-label", "Person Morgan Martin");

  // An abbreviated identity read aloud tells a screen-reader user nothing.
  const unresolved = group(page, "Unresolved reference").locator(
    ".inline-chip",
  );
  await expect(unresolved).toHaveAttribute("aria-label", "Unresolved person");
  const label = await unresolved.locator(".inline-chip-label").innerText();
  expect(label).not.toContain("9f2c4a1b7e5d8306");
});

test("an unresolved reference is never a control", async ({ page }) => {
  await open(page);

  // Nothing to open, so it must not look or behave actionable however it was
  // asked for. A chip that invites a click and does nothing is worse than one
  // that reads as inert.
  const unresolved = group(page, "Unresolved reference").locator(
    ".inline-chip",
  );
  await expect(unresolved).toHaveAttribute("data-state", "unresolved");
  await expect(unresolved).toHaveRole("img");
});

test("inert chips claim no interactive stop", async ({ page }) => {
  await open(page);

  const inert = group(page, "Inert rendering").locator(".inline-chip");
  await expect(inert).toHaveRole("img");
  await expect(inert.locator("xpath=self::button")).toHaveCount(0);
});

test("a resolved chip previews on pointer and keyboard focus", async ({
  page,
}) => {
  await open(page);
  const resolved = group(page, "Resolved preview").locator(".inline-chip");
  const preview = page.getByRole("tooltip");

  await expect(preview).toHaveCount(0);
  await resolved.hover();
  await expect(preview).toBeVisible();
  await expect(preview).toContainText("Person");
  await expect(preview).toContainText("Morgan Martin");

  await page.mouse.move(0, 0);
  await resolved.focus();
  await expect(preview).toBeVisible();
});

test("an unresolved chip has no preview", async ({ page }) => {
  await open(page);
  await expect(
    group(page, "Unresolved reference").getByRole("tooltip"),
  ).toHaveCount(0);
});

test("activating a chip reports its address, not its label", async ({
  page,
}) => {
  await open(page);
  const activation = group(page, "Activation");

  await expect(activation.getByText("Not activated")).toBeVisible();
  await activation.locator(".inline-chip").click();
  await expect(activation.getByText("Opened pk-morgan")).toBeVisible();
});

test("a long label truncates instead of stretching the line", async ({
  page,
}) => {
  await open(page);
  const chip = group(page, "Long label truncates").locator(".inline-chip");

  const box = await chip.boundingBox();
  if (!box) throw new Error("Chip is not laid out");
  // Bounded by the component's max-width rather than growing to fit.
  expect(box.width).toBeLessThan(280);

  const label = chip.locator(".inline-chip-label");
  const clipped = await label.evaluate(
    (el) => el.scrollWidth > el.clientWidth + 1,
  );
  expect(clipped).toBe(true);
});

test("a chip never breaks across a line", async ({ page }) => {
  await open(page);
  const chips = group(page, "Wrapping across lines").locator(".inline-chip");

  // Each chip occupies exactly one client rect: a break falls before or after
  // it, never through its middle.
  for (const chip of await chips.all()) {
    const rects = await chip.evaluate((el) => el.getClientRects().length);
    expect(rects).toBe(1);
  }
});

test("hover deepens the tint rather than changing the text", async ({
  page,
}) => {
  await open(page);
  const chip = group(page, "Activation").locator(".inline-chip");

  const read = () =>
    chip.evaluate((el) => {
      const s = getComputedStyle(el);
      return { bg: s.backgroundColor, color: s.color };
    });

  const rest = await read();
  await chip.hover();
  await page.waitForFunction(
    ([el, was]) => getComputedStyle(el as Element).backgroundColor !== was,
    [await chip.elementHandle(), rest.bg] as const,
  );
  const hovered = await read();

  expect(hovered.bg).not.toBe(rest.bg);
  expect(hovered.color).toBe(rest.color);
});
