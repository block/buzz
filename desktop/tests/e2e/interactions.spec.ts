import { expect, test, type Page } from "@playwright/test";
import { installMockBridge } from "../helpers/bridge";
import { waitForAnimations } from "../helpers/animations";

const RELAY = "a".repeat(64);
const PROMPT = "b".repeat(64);
const PROJECTION = "c".repeat(64);
const QUESTION = "Render The Door That Refused the Umbral Key?";

async function seed(
  page: Page,
  type = "buttons",
  fields: string[][] = [],
  enabled = true,
) {
  await installMockBridge(
    page,
    { relaySelf: RELAY },
    { seedPreviewFeatures: enabled },
  );
  await page.goto("/");
  await page.getByTestId("channel-engineering").click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "engineering",
          }) ?? false,
      ),
    )
    .toBe(true);
  await page.evaluate(
    ({ relay, prompt, projection, question, type, fields }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      const opts =
        type === "form"
          ? []
          : [
              ["opt", "approve", "Approve", "primary"],
              ["opt", "deny", "Deny", "danger"],
            ];
      emit?.({
        channelName: "engineering",
        id: prompt,
        kind: 40010,
        content: question,
        extraTags: [
          ["itype", type],
          ["closes", type === "poll" ? "expiry" : "first"],
          ["deadline", String(Math.floor(Date.now() / 1000) + 3600)],
          ...opts,
          ...fields,
        ],
      });
      emit?.({
        channelName: "engineering",
        id: projection,
        kind: 9,
        pubkey: relay,
        content: `${question}\nApprove / Deny\nReply with an option.`,
        extraTags: [["interaction", prompt]],
      });
    },
    {
      relay: RELAY,
      prompt: PROMPT,
      projection: PROJECTION,
      question: QUESTION,
      type,
      fields,
    },
  );
  if (enabled) {
    await expect(page.getByTestId("interaction-card")).toBeVisible();
    await expect
      .poll(() =>
        page.evaluate(
          () =>
            window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
              channelName: "engineering",
              kind: 39010,
            }) ?? false,
        ),
      )
      .toBe(true);
    await updateState(page, 0, false);
  }
}
async function updateState(
  page: Page,
  revision: number,
  closed: boolean,
  signer = RELAY,
) {
  await page.evaluate(
    ({ prompt, revision, closed, signer }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "engineering",
        id: String(revision + 1)
          .repeat(64)
          .slice(0, 64),
        kind: 39010,
        pubkey: signer,
        content: JSON.stringify({
          version: 1,
          revision,
          status: closed ? "closed" : "open",
          close_reason: closed ? "first" : null,
          winner: closed ? "approve" : null,
          tally: { approve: closed ? 1 : 0, deny: 0 },
          responders: [],
        }),
        extraTags: [["d", prompt]],
      });
    },
    { prompt: PROMPT, revision, closed, signer },
  );
}

test("buttons sign a decision and follow authoritative close; stale or foreign state is ignored", async ({
  page,
}) => {
  await seed(page);
  const card = page.getByTestId("interaction-card");
  const approve = card.getByRole("button", { name: "Approve", exact: true });
  await expect(approve).toBeEnabled();
  await updateState(page, 50, true, "f".repeat(64));
  await expect(approve).toBeEnabled();
  await approve.focus();
  await page.keyboard.press("Enter");
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_SIGNED_EVENTS__
            ?.filter((e) => e.kind === 40011)
            .at(-1)?.tags,
      ),
    )
    .toContainEqual(["choice", "approve"]);
  await updateState(page, 2, true);
  await expect(card.getByRole("status")).toContainText("Closed · Approve");
  await updateState(page, 1, false);
  await expect(approve).toBeDisabled();
  await waitForAnimations(page);
  await card.screenshot({ path: "test-results/interactions-closed.png" });
});

test("forms retain validation and send typed values", async ({ page }) => {
  await seed(page, "form", [
    ["field", "title", "Episode title", "text", "required"],
    ["field", "length", "Length", "select", "required"],
    ["optsel", "length", "60s", "60 seconds"],
    ["optsel", "length", "6min", "6 minutes"],
  ]);
  const card = page.getByTestId("interaction-card");
  await card.getByRole("button", { name: "Submit answer" }).click();
  expect(
    await page.evaluate(
      () =>
        window.__BUZZ_E2E_SIGNED_EVENTS__?.filter((e) => e.kind === 40011)
          .length ?? 0,
    ),
  ).toBe(0);
  await card.getByLabel("Episode title (required)").fill("The Door");
  await card.getByLabel("Length (required)").selectOption("6min");
  await waitForAnimations(page);
  await card.screenshot({ path: "test-results/interactions-form.png" });
  await card.getByRole("button", { name: "Submit answer" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_SIGNED_EVENTS__
            ?.filter((e) => e.kind === 40011)
            .at(-1)?.tags,
      ),
    )
    .toContainEqual(["value", "length", "6min"]);
});

test("polls show relay tallies and publish selected choices", async ({
  page,
}) => {
  await seed(page, "poll");
  const card = page.getByTestId("interaction-card");
  await card.getByRole("radio", { name: /Deny/ }).check();
  await card.getByRole("button", { name: "Submit answer" }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_SIGNED_EVENTS__
            ?.filter((e) => e.kind === 40011)
            .at(-1)?.tags,
      ),
    )
    .toContainEqual(["choice", "deny"]);
});

test("default-off clients retain the readable reply fallback", async ({
  page,
}) => {
  await seed(page, "buttons", [], false);
  await expect(page.getByTestId("interaction-card")).toHaveCount(0);
  await expect(
    page.getByText("Reply with an option.", { exact: false }),
  ).toBeVisible();
  await waitForAnimations(page);
  await page
    .getByTestId("message-row")
    .filter({ hasText: QUESTION })
    .screenshot({ path: "test-results/interactions-fallback.png" });
});
