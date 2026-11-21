import { expect, test } from "@playwright/test";
import {
  conversationAppLink,
  opensAppOnLoad,
} from "../../src/features/conversation-link/conversation-link";

const event = "a".repeat(64);
const root = "b".repeat(64);

test("conversation links hand off only to canonical Buzz links", () => {
  expect(conversationAppLink({ channelId: "general" })).toBe(
    "buzz://channel/general",
  );
  expect(
    conversationAppLink({
      channelId: "general",
      messageId: event.toUpperCase(),
    }),
  ).toBe(`buzz://message?channel=general&id=${event}`);
  expect(
    conversationAppLink({
      channelId: "general",
      messageId: event,
      search: `?thread=${root}`,
    }),
  ).toBe(`buzz://message?channel=general&id=${event}&thread=${root}`);

  // Anything that is not a channel id or an event id hands off nowhere.
  for (const channelId of [
    "",
    "javascript:alert(1)",
    "general%2F..",
    "../general",
    "gen eral",
    "-general",
    "a".repeat(257),
  ])
    expect(conversationAppLink({ channelId })).toBeNull();
  for (const messageId of ["", "abc123", `${event}0`, "g".repeat(64)])
    expect(conversationAppLink({ channelId: "general", messageId })).toBeNull();
  // A thread needs a message, a valid root, and only one of them.
  expect(
    conversationAppLink({ channelId: "general", search: `?thread=${root}` }),
  ).toBeNull();
  expect(
    conversationAppLink({
      channelId: "general",
      messageId: event,
      search: "?thread=javascript:alert(1)",
    }),
  ).toBeNull();
  expect(
    conversationAppLink({
      channelId: "general",
      messageId: event,
      search: `?thread=${root}&thread=${event}`,
    }),
  ).toBeNull();
});

test("conversation links open the app on load only on desktop", () => {
  for (const os of ["macos", "windows", "linux"])
    expect(opensAppOnLoad(os)).toBe(true);
  for (const os of ["ios", "android", "chromeos", "unknown"])
    expect(opensAppOnLoad(os)).toBe(false);
});

test("a conversation link on a phone waits for a tap to open Buzz", async ({
  browser,
}) => {
  const context = await browser.newContext({
    userAgent:
      "Mozilla/5.0 (iPhone; CPU iPhone OS 18_5 like Mac OS X) AppleWebKit/605.1.15",
  });
  await context.addInitScript(() => {
    Object.defineProperties(navigator, {
      platform: { configurable: true, value: "iPhone" },
      maxTouchPoints: { configurable: true, value: 5 },
    });
  });
  const page = await context.newPage();
  await page.route("https://api.github.com/**", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: "[]" }),
  );

  await page.goto(`/channel/general/${event}?thread=${root}`);
  await expect(
    page.getByRole("heading", { name: "Open in Buzz" }),
  ).toBeVisible();
  await expect(
    page.getByText("This message is in a Buzz community."),
  ).toBeVisible();
  await expect(
    page.getByRole("link", { name: "Open in Buzz" }),
  ).toHaveAttribute(
    "href",
    `buzz://message?channel=general&id=${event}&thread=${root}`,
  );
  await expect(page).toHaveURL(new RegExp(`/channel/general/${event}`));

  await page.goto("/channel/general");
  await expect(
    page.getByRole("link", { name: "Open in Buzz" }),
  ).toHaveAttribute("href", "buzz://channel/general");

  await page.goto(`/channel/general/${event}?thread=${root}&thread=${event}`);
  await expect(
    page.getByRole("heading", { name: "This link isn't valid" }),
  ).toBeVisible();
  await expect(page.getByRole("link", { name: "Open in Buzz" })).toHaveCount(0);
  await expect(page.getByRole("link", { name: "Download Buzz" })).toBeVisible();
  await context.close();
});
