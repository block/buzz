import { expect, type Locator, type Page, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";
import { openGeneral } from "../helpers/channelHeadReady";

function mainComposer(page: Page) {
  return page
    .getByTestId("message-composer")
    .filter({ hasNot: page.getByTestId("edit-target") })
    .first();
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

async function typeCommand(page: Page, text: string) {
  const input = mainComposer(page).getByTestId("message-input");
  await input.click();
  await input.pressSequentially(text);
  return input;
}

async function listedCommands(composer: Locator) {
  const input = composer.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("/");
  const list = composer.getByTestId("slash-command-autocomplete");
  await expect(list).toBeVisible();
  const names = await list
    .locator("[data-testid^='slash-command-']")
    .evaluateAll((rows) =>
      rows.map((row) => row.getAttribute("data-testid")?.slice(14)),
    );
  await input.press("Escape");
  await expect(list).toBeHidden();
  await input.press("Backspace");
  return names;
}

async function openRowMenu(page: Page, channelName: string) {
  await page.getByTestId(`channel-${channelName}`).click({ button: "right" });
  return page.getByRole("menu");
}

test("/rename renames the channel, and a rejection or same name keeps the draft", async ({
  page,
}) => {
  await openChannel(page, {
    updateChannelErrors: ["Relay refused the rename"],
  });
  const title = page.getByTestId("chat-title");

  const input = await typeCommand(page, "/rename ops");
  await input.press("Enter");
  await expect(page.getByText("Relay refused the rename")).toBeVisible();
  await expect(input).toHaveText("/rename ops");
  await expect(title).toHaveText("general");

  await input.press("Enter");
  await expect(page.getByText("Renamed to #ops")).toBeVisible();
  await expect(title).toHaveText("ops");
  await expect(page.getByTestId("channel-ops")).toBeVisible();
  await expect(input).toBeEmpty();

  await input.pressSequentially("/rename  ops ");
  await input.press("Enter");
  await expect(
    page.getByText("This channel is already named #ops"),
  ).toBeVisible();
  await expect(input).toHaveText("/rename  ops");

  await input.press("ControlOrMeta+a");
  await input.pressSequentially("/rename");
  await input.press("Enter");
  await expect(page.getByText("Usage: /rename <name>")).toBeVisible();
  await expect(input).toHaveText("/rename");
  await expect(title).toHaveText("ops");
});

test("/topic sets the topic shown on the channel header and needs text", async ({
  page,
}) => {
  await openChannel(page);
  const title = page.getByTestId("chat-title");

  const input = await typeCommand(page, "/topic");
  await input.press("Enter");
  await expect(page.getByText("Usage: /topic <text>")).toBeVisible();
  await expect(input).toHaveText("/topic");

  await input.pressSequentially(" Release planning");
  await input.press("Enter");
  await expect(page.getByText("Topic set", { exact: true })).toBeVisible();
  await expect(title).toHaveAttribute("title", "Release planning");
  await expect(input).toBeEmpty();
});

test("/mute toggles the sidebar mute state on each run", async ({ page }) => {
  await openChannel(page);
  const input = await typeCommand(page, "/mute");

  await input.press("Enter");
  await expect(page.getByText("Muted #general", { exact: true })).toBeVisible();
  await expect(input).toBeEmpty();
  let menu = await openRowMenu(page, "general");
  await expect(menu.getByText("Unmute channel")).toBeVisible();
  await page.keyboard.press("Escape");

  await input.pressSequentially("/mute");
  await input.press("Enter");
  await expect(
    page.getByText("Unmuted #general", { exact: true }),
  ).toBeVisible();
  menu = await openRowMenu(page, "general");
  await expect(menu.getByText("Mute channel", { exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
});

test("/archive archives the channel straight away, as the channel menu does", async ({
  page,
}) => {
  await openChannel(page);
  const input = await typeCommand(page, "/archive");
  await input.press("Enter");

  await expect(
    page.getByText("Archived #general", { exact: true }),
  ).toBeVisible();
  await expect(page.getByTestId("stream-list")).not.toContainText("general");
  await expect(page.getByTestId("message-input")).toHaveAttribute(
    "contenteditable",
    "false",
  );
});

test("a member who cannot manage the channel is not offered /rename or /archive", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  const composer = mainComposer(page);

  await expect
    .poll(() => listedCommands(composer))
    .toEqual(expect.arrayContaining(["mute", "topic"]));
  const names = await listedCommands(composer);
  expect(names).not.toContain("rename");
  expect(names).not.toContain("archive");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect
    .poll(() => listedCommands(mainComposer(page)))
    .toEqual(expect.arrayContaining(["archive", "mute", "rename", "topic"]));
});

test("channel commands are hidden in a DM and refused when typed there", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  const composer = mainComposer(page);

  expect(await listedCommands(composer)).toEqual(
    expect.not.arrayContaining(["rename", "archive", "topic"]),
  );

  const input = await typeCommand(page, "/rename secret");
  await input.press("Enter");
  await expect(page.getByText("/rename only works in channels.")).toBeVisible();
  await expect(input).toHaveText("/rename secret");

  await input.press("ControlOrMeta+a");
  await input.pressSequentially("/mute");
  await input.press("Enter");
  await expect(page.getByText(/^Muted /)).toBeVisible();
  await expect(input).toBeEmpty();
});
