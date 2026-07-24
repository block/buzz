import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/mention-descriptions";

// Distinct pubkeys — no overlap with bridge fixtures or other specs.
const FIZZ_PUBKEY = "3a".repeat(32);
const HONEY_PUBKEY = "3b".repeat(32);
const BUMBLE_PUBKEY = "3c".repeat(32);
const BUZZY_PUBKEY = "3d".repeat(32);
const ATLAS_PUBKEY = "3e".repeat(32);

const FIZZ_ABOUT = "Builder — implements features and fixes bugs";
const HONEY_ABOUT = "Writer — drafts docs, posts, and summaries";
const BUMBLE_ABOUT = "Researcher — deep dives, sourcing, and citations";
const ATLAS_LONG_ABOUT =
  "Operations copilot for the whole hive: triages incoming requests, " +
  "routes work to the right specialist agent, keeps the runbook current, " +
  "and escalates anything ambiguous to a human before acting on it";

/** Locator scoped to the mention autocomplete dropdown inside the composer. */
function autocomplete(page: import("@playwright/test").Page) {
  return page
    .getByTestId("message-composer")
    .getByTestId("mention-autocomplete");
}

/** Full-page clip spanning the open dropdown down to the composer bottom. */
async function shootComposerWithDropdown(
  page: import("@playwright/test").Page,
  path: string,
) {
  await waitForAnimations(page);
  const dropdownBox = await autocomplete(page).boundingBox();
  const composerBox = await page
    .getByTestId("message-composer")
    .boundingBox();
  if (!dropdownBox || !composerBox) {
    throw new Error("composer or dropdown not visible for screenshot");
  }
  const top = Math.max(0, dropdownBox.y - 8);
  await page.screenshot({
    path,
    clip: {
      x: Math.max(0, composerBox.x - 8),
      y: top,
      width: composerBox.width + 16,
      height: composerBox.y + composerBox.height + 8 - top,
    },
  });
}

test("mention selector shows each agent's kind-0 about as a role line", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: FIZZ_PUBKEY,
        name: "Fizz",
        status: "stopped",
        channelNames: ["general"],
      },
      {
        pubkey: HONEY_PUBKEY,
        name: "Honey",
        status: "stopped",
        channelNames: ["general"],
      },
      {
        pubkey: BUMBLE_PUBKEY,
        name: "Bumble",
        status: "stopped",
        channelNames: ["general"],
      },
      {
        pubkey: BUZZY_PUBKEY,
        name: "Buzzy",
        status: "stopped",
        channelNames: ["general"],
      },
    ],
    searchProfiles: [
      { pubkey: FIZZ_PUBKEY, displayName: "Fizz", about: FIZZ_ABOUT },
      { pubkey: HONEY_PUBKEY, displayName: "Honey", about: HONEY_ABOUT },
      { pubkey: BUMBLE_PUBKEY, displayName: "Bumble", about: BUMBLE_ABOUT },
      // Buzzy has no `about` — exercises the name-only fallback row.
      // bob is human — an `about` on a person must NOT render a role line.
      {
        pubkey: TEST_IDENTITIES.bob.pubkey,
        displayName: "bob",
        about: "A human bio that stays off the mention row",
      },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill("@");

  const dropdown = autocomplete(page);
  await expect(dropdown).toBeVisible();

  for (const [name, about] of [
    ["Fizz", FIZZ_ABOUT],
    ["Honey", HONEY_ABOUT],
    ["Bumble", BUMBLE_ABOUT],
  ] as const) {
    const row = dropdown.locator("button", { hasText: name });
    await expect(row.getByTestId("mention-agent-icon")).toBeVisible();
    await expect(row.getByTestId("mention-agent-description")).toHaveText(
      about,
    );
    await expect(row.getByText("managed by you")).toBeVisible();
  }

  // No `about` → today's exact row: bot icon + literal "agent" label.
  const buzzyRow = dropdown.locator("button", { hasText: "Buzzy" });
  await expect(buzzyRow.getByTestId("mention-agent-icon")).toBeVisible();
  await expect(buzzyRow.getByText("agent", { exact: true })).toBeVisible();
  await expect(buzzyRow.getByTestId("mention-agent-description")).toHaveCount(
    0,
  );

  // Humans never get a role line, even with an `about` on their profile.
  const bobRow = dropdown.locator("button", { hasText: "bob" });
  await expect(bobRow).toBeVisible();
  await expect(bobRow.getByTestId("mention-agent-description")).toHaveCount(0);

  // Scroll the agent rows into frame — the list opens scrolled to the top
  // where the viewer/member rows sit.
  await dropdown.evaluate((el) => {
    el.scrollTop = el.scrollHeight;
  });
  await shootComposerWithDropdown(page, `${SHOTS}/01-agent-role-lines.png`);
});

test("long about truncates to a single line beside the managed-by label", async ({
  page,
}) => {
  await installMockBridge(page, {
    managedAgents: [
      {
        pubkey: ATLAS_PUBKEY,
        name: "Atlas",
        status: "stopped",
        channelNames: ["general"],
      },
    ],
    searchProfiles: [
      { pubkey: ATLAS_PUBKEY, displayName: "Atlas", about: ATLAS_LONG_ABOUT },
    ],
  });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill("@Atlas");

  const dropdown = autocomplete(page);
  const atlasRow = dropdown.locator("button", { hasText: "Atlas" });
  const description = atlasRow.getByTestId("mention-agent-description");
  await expect(description).toBeVisible();
  await expect(description).toHaveAttribute("title", ATLAS_LONG_ABOUT);
  await expect(atlasRow.getByText("managed by you")).toBeVisible();

  // The full text must overflow its one-line box — proof it truncates
  // instead of wrapping or pushing the managed-by label out of the row.
  const truncates = await description.evaluate(
    (el) => el.scrollWidth > el.clientWidth,
  );
  expect(truncates).toBe(true);

  await shootComposerWithDropdown(page, `${SHOTS}/02-long-about-truncates.png`);
});
