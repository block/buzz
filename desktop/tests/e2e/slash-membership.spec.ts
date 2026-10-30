import { expect, test, type Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { openGeneral } from "../helpers/channelHeadReady";

async function openChannel(
  page: Page,
  name: string,
  options?: Parameters<typeof installMockBridge>[1],
) {
  await installMockBridge(page, options);
  await page.goto("/");
  if (name === "general") {
    await openGeneral(page);
  } else {
    await page.getByTestId(`channel-${name}`).click();
  }
  await expect(page.getByTestId("chat-title")).toHaveText(name);
  return page.getByTestId("message-input");
}

async function runCommand(page: Page, text: string) {
  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially(text);
  await page.keyboard.press("Escape");
  await input.press("Enter");
  return input;
}

async function typeMention(page: Page, command: string, name: string) {
  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially(`${command} @${name}`);
  await page
    .getByTestId("mention-autocomplete")
    .getByText(name, { exact: true })
    .click();
  return input;
}

function commandCalls(page: Page, command: string) {
  return page.evaluate(
    (command) =>
      (window.__BUZZ_E2E_COMMAND_LOG__ ?? [])
        .filter((call) => call.command === command)
        .map((call) => call.payload as Record<string, unknown>),
    command,
  );
}

async function listedCommands(page: Page, prefix = "/") {
  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially(prefix);
  const list = page.getByTestId("slash-command-autocomplete");
  await expect(list).toBeVisible();
  const names = await list
    .locator("[data-testid^='slash-command-']")
    .evaluateAll((rows) =>
      rows.map((row) => row.getAttribute("data-testid")?.slice(14) ?? ""),
    );
  await input.press("Escape");
  await expect(list).toBeHidden();
  await input.press("ControlOrMeta+a");
  await input.press("Delete");
  return names;
}

test("/invite adds a mentioned person, and /kick then /remove take them out again", async ({
  page,
}) => {
  const input = await openChannel(page, "general");
  const { pubkey } = TEST_IDENTITIES.charlie;

  await typeMention(page, "/invite", "charlie");
  await page.keyboard.press("Escape");
  await input.press("Enter");
  await expect(page.getByText("Added 1", { exact: true })).toBeVisible();
  await expect(input).toBeEmpty();
  const added = await commandCalls(page, "add_channel_members");
  expect(added).toHaveLength(1);
  expect(added[0]).toMatchObject({ pubkeys: [pubkey], role: "bot" });

  await input.press("ControlOrMeta+a");
  await typeMention(page, "/invite", "charlie");
  await page.keyboard.press("Escape");
  await input.press("Enter");
  await expect(
    page.getByText("Already in this channel", { exact: true }),
  ).toBeVisible();
  await expect(input).toContainText("/invite");
  await input.press("ControlOrMeta+a");
  await input.press("Delete");

  await typeMention(page, "/kick", "charlie");
  await page.keyboard.press("Escape");
  await input.press("Enter");
  await expect(page.getByText("Removed 1", { exact: true })).toBeVisible();
  await expect(input).toBeEmpty();
  expect(await commandCalls(page, "remove_channel_member")).toEqual([
    expect.objectContaining({ pubkey }),
  ]);

  await typeMention(page, "/remove", "charlie");
  await page.keyboard.press("Escape");
  await input.press("Enter");
  await expect(
    page.getByText("Not in this channel", { exact: true }),
  ).toBeVisible();
  await expect(input).toContainText("/remove");
  expect(await commandCalls(page, "remove_channel_member")).toHaveLength(1);
});

test("a relay rejection of /invite shows the reason and keeps the draft", async ({
  page,
}) => {
  const input = await openChannel(page, "general", {
    addChannelMembersErrors: ["Only admins can add people here."],
  });

  await typeMention(page, "/invite", "charlie");
  await page.keyboard.press("Escape");
  await input.press("Enter");
  await expect(
    page.getByText("Only admins can add people here.", { exact: true }),
  ).toBeVisible();
  await expect(input).toContainText("/invite");
  await expect(input).toContainText("charlie");
});

test("/who opens the member list, and /join opens a channel the user is not in", async ({
  page,
}) => {
  const input = await openChannel(page, "general");

  await runCommand(page, "/who");
  await expect(page.getByTestId("members-sidebar")).toBeVisible();
  await expect(input).toBeEmpty();
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("members-sidebar")).toBeHidden();

  await runCommand(page, "/join #nowhere");
  await expect(page.getByText("No channel named #nowhere")).toBeVisible();
  await expect(input).toHaveText("/join #nowhere");
  await input.press("ControlOrMeta+a");
  await input.press("Delete");

  await runCommand(page, "/join #design");
  await expect(page.getByTestId("chat-title")).toHaveText("design");
  expect(await commandCalls(page, "join_channel")).toHaveLength(1);

  await page.getByTestId("message-input").click();
  await runCommand(page, "/join #random");
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  expect(await commandCalls(page, "join_channel")).toHaveLength(1);
});

test("/leave asks first: cancelling keeps the draft, confirming leaves and goes home", async ({
  page,
}) => {
  const input = await openChannel(page, "random");

  await runCommand(page, "/part");
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toContainText('Leave "random"?');
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toBeHidden();
  await expect(input).toHaveText("/part");
  expect(await commandCalls(page, "leave_channel")).toHaveLength(0);

  await input.press("Enter");
  await expect(dialog).toBeVisible();
  await dialog.getByRole("button", { name: "Leave" }).click();
  await expect(dialog).toBeHidden();
  await expect(page.getByTestId("chat-title")).toHaveCount(0);
  expect(await commandCalls(page, "leave_channel")).toEqual([
    expect.objectContaining({
      channelId: expect.any(String),
    }),
  ]);
  await expect(page.getByTestId("channel-random")).toHaveCount(0);
});

test("channel commands are refused in a DM and left out of its list", async ({
  page,
}) => {
  const input = await openChannel(page, "alice-tyler");

  const names = await listedCommands(page);
  expect(names).toEqual(expect.arrayContaining(["join", "who"]));
  for (const hidden of ["invite", "remove", "leave"]) {
    expect(names).not.toContain(hidden);
  }

  await runCommand(page, "/leave");
  await expect(
    page.getByText("/leave only works in channels.", { exact: true }),
  ).toBeVisible();
  await expect(input).toHaveText("/leave");
  await expect(page.getByRole("alertdialog")).toHaveCount(0);
  expect(await commandCalls(page, "leave_channel")).toHaveLength(0);
});

test("/remove is only listed for owners and admins", async ({ page }) => {
  await openChannel(page, "general");
  expect(await listedCommands(page)).toEqual(
    expect.arrayContaining(["invite", "remove", "leave"]),
  );

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(async () => {
    const names = await listedCommands(page);
    expect(names).toContain("invite");
    expect(names).not.toContain("remove");
  }).toPass();
});
