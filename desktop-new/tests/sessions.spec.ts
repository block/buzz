import { expect, test } from "@playwright/test";

test("opens a real channel and keeps protocol metadata out of the transcript", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  await page.getByRole("button", { name: "design", exact: true }).click();
  await expect(page.getByRole("heading", { name: "#design" })).toBeVisible();
  await expect(
    page.getByText(
      "The session should begin as simply as a thought: just type.",
    ),
  ).toBeVisible();
  await expect(page.getByText(/has_more|channel_created/)).toHaveCount(0);

  // The panel is the Dockview content surface. If it shrink-wraps to the
  // transcript, the composer escapes onto the app backdrop instead of staying
  // anchored inside the conversation panel.
  const panelHeight = await page
    .locator(".dv-active-group .panel")
    .evaluate((element) => Math.round(element.getBoundingClientRect().height));
  expect(panelHeight).toBeGreaterThan(500);

  // Dockview leaves a visible resize gutter between these regions. They must
  // therefore remain independent panels, each with a complete silhouette—not
  // retain a zero-radius edge from an old fixed connected-pair layout.
  const panelCorners = await page
    .locator(".buzz-dockview .panel")
    .evaluateAll((panels) =>
      panels.map((panel) => {
        const style = getComputedStyle(panel);
        return [
          style.borderTopLeftRadius,
          style.borderTopRightRadius,
          style.borderBottomRightRadius,
          style.borderBottomLeftRadius,
        ];
      }),
    );
  expect(panelCorners).toEqual([
    ["24px", "24px", "24px", "24px"],
    ["24px", "24px", "24px", "24px"],
  ]);
});

test("sends through the Conversation controller", async ({ page }) => {
  await page.goto("/?mock=1");
  const composer = page.getByRole("textbox", { name: "Message the session" });
  await composer.fill("A real contribution from the controller");
  await composer.press("Enter");

  // This binds the temporary conversation composition to the controller's
  // optimistic → accepted lifecycle. The message must settle in the transcript
  // instead of silently clearing out of the composer.
  await expect(
    page.getByText("A real contribution from the controller"),
  ).toBeVisible();
  await expect(composer).toHaveValue("");
});

test("keeps a deliberate newline in the composer until it is sent", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  const composer = page.getByRole("textbox", { name: "Message the session" });
  await composer.fill("First line");
  await composer.press("Shift+Enter");
  await composer.pressSequentially("Second line");

  await expect(composer).toHaveValue("First line\nSecond line");

  await composer.press("Enter");
  await expect(page.getByText("First line\nSecond line")).toBeVisible();
  await expect(composer).toHaveValue("");
});

test("keeps a conversation draft when a person leaves and returns", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  const composer = page.getByRole("textbox", { name: "Message the session" });
  await composer.fill("Keep this thought while I check another channel");

  await page
    .getByRole("button", { name: "buzz-interface", exact: true })
    .click();
  await page.getByRole("button", { name: "design", exact: true }).click();

  // A draft belongs to the conversation, not the mounted textarea. Removing
  // this controller makes the value disappear when the view changes.
  await expect(composer).toHaveValue(
    "Keep this thought while I check another channel",
  );
});

test("keeps a new-Session draft when a person leaves and returns", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  await page.getByRole("button", { name: "New Session in design" }).click();
  const composer = page.getByRole("textbox", { name: "Start a Session" });
  await composer.fill("Keep this Session thought");

  await page.getByRole("button", { name: "Back" }).click();
  await page.getByRole("button", { name: "New Session in design" }).click();

  await expect(composer).toHaveValue("Keep this Session thought");
});

test("starts a Session from its channel without naming ceremony", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  await page.getByRole("button", { name: "New Session in design" }).click();
  await expect(page.getByText("From design")).toBeVisible();
  const composer = page.getByRole("textbox", { name: "Start a Session" });
  await composer.fill("Explore a calmer multi-agent conversation");
  await composer.press("Enter");
  await expect(
    page.getByRole("heading", {
      name: "Explore a calmer multi-agent conversation",
    }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", {
      name: "Explore a calmer multi-agent conversation",
    }),
  ).toBeVisible();
});

test("keeps the composer usable above live agent activity", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  const dock = page.locator(".conversation-composer-dock");
  const composer = page.getByRole("textbox", { name: "Message the session" });

  await expect(dock).toHaveAttribute("data-activity", "true");
  await expect(dock.locator(".conversation-activity-rail")).toContainText(
    "Vogue is working",
  );
  await composer.fill("Keep writing while Vogue works");
  await expect(composer).toHaveValue("Keep writing while Vogue works");

  const positions = await page.evaluate(() => {
    const composer = document.querySelector(".message-composer");
    const rail = document.querySelector(".conversation-activity-rail");
    if (!composer || !rail) throw new Error("Composer dock is incomplete");
    return {
      composerBottom: composer.getBoundingClientRect().bottom,
      railTop: rail.getBoundingClientRect().top,
    };
  });
  expect(positions.composerBottom).toBeLessThanOrEqual(positions.railTop);
  // Browser percentage layout can resolve to fractional device pixels; the
  // designed dock separation is the 4px spacing step, within one CSS pixel.
  expect(positions.railTop - positions.composerBottom).toBeCloseTo(4, 0);
});

test("keeps owned agent activity inline and collapses earlier steps", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  await page.getByRole("button", { name: "design", exact: true }).click();
  await expect(
    page.getByRole("region", { name: "Vogue activity" }),
  ).toBeVisible();
  const disclosure = page.getByRole("button", {
    name: /Vogue activity, 5 steps/,
  });
  await disclosure.click();
  await expect(page.getByText("Read the interaction plan")).toBeVisible();
});

test("exposes the focused Agents destination without expanding the app scope", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  await page.getByRole("button", { name: "Agents", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Vogue" })).toBeVisible();
  await expect(
    page.getByText("Shape product interfaces with clarity and taste."),
  ).toBeVisible();
  await page.getByRole("button", { name: "New agent" }).click();
  await expect(page.getByRole("heading", { name: "New agent" })).toBeVisible();
  await page.getByRole("textbox", { name: "Name" }).fill("Scout");
  await page
    .getByRole("textbox", { name: "Instructions" })
    .fill("Help with focused product research.");
  await page.getByRole("button", { name: "Create and start" }).click();
  await expect(page.getByRole("heading", { name: "Scout" })).toBeVisible();
  await expect(
    page.getByText("Help with focused product research."),
  ).toBeVisible();
});

test("keeps the appearance choice after a reload", async ({ page }) => {
  await page.goto("/?mock=1");
  await page.evaluate(() => localStorage.removeItem("buzz-next-color-scheme"));
  await page.reload();

  await page.getByRole("button", { name: "Use dark mode" }).click();
  await expect(page.locator("html")).toHaveClass(/dark/);

  // The shell used to hold its own `dark` boolean beside the colour-scheme
  // fact, so the choice was lost on reload and two owners disagreed.
  await page.reload();
  await expect(page.locator("html")).toHaveClass(/dark/);
  await expect(
    page.getByRole("button", { name: "Use light mode" }),
  ).toBeVisible();
});

test("inserts emoji at the authored caret and returns to writing", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  const composer = page.getByRole("textbox", { name: "Message the session" });
  await composer.fill("Hello world");
  await composer.press("ArrowLeft");
  await page.getByRole("button", { name: "Choose emoji" }).click();
  await page.getByRole("button", { name: "Insert 🎉" }).click();

  await expect(composer).toHaveValue("Hello worl🎉d");
  await expect(composer).toBeFocused();
  await expect(page.getByRole("button", { name: "Insert 🎉" })).toHaveCount(0);
});

test("dismisses the emoji picker without losing the composer focus", async ({
  page,
}) => {
  await page.goto("/?mock=1");
  const composer = page.getByRole("textbox", { name: "Message the session" });
  await composer.fill("Keep writing");
  await page.getByRole("button", { name: "Choose emoji" }).click();
  await expect(page.getByRole("button", { name: "Insert 😀" })).toBeVisible();
  await page.keyboard.press("Escape");

  await expect(page.getByRole("button", { name: "Insert 😀" })).toHaveCount(0);
  await expect(composer).toBeFocused();
  await expect(composer).toHaveValue("Keep writing");
});

test("shows focus rings only for keyboard navigation", async ({ page }) => {
  await page.goto("/?mock=1");
  const composer = page.getByRole("textbox", { name: "Message the session" });
  const search = page.getByRole("searchbox", {
    name: "Find a Room, person, or Session",
  });

  await composer.click();
  await expect(page.locator("html")).not.toHaveAttribute(
    "data-keyboard-navigation",
  );
  await expect(composer).toHaveCSS("outline-style", "none");

  await search.focus();
  await page.keyboard.press("Tab");
  await expect(page.locator("html")).toHaveAttribute(
    "data-keyboard-navigation",
  );
  await expect(
    page.getByRole("button", { name: "design", exact: true }),
  ).toHaveCSS("outline-style", "solid");
});
