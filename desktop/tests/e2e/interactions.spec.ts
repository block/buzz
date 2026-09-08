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
  extraTags: string[][] = [],
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
    ({ relay, prompt, projection, question, type, fields, extraTags }) => {
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
          [
            "deadline",
            extraTags.find((t) => t[0] === "deadline")?.[1] ??
              String(Math.floor(Date.now() / 1000) + 3600),
          ],
          ...opts,
          ...fields,
          ...extraTags.filter((t) => t[0] !== "deadline"),
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
      extraTags,
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
  answer?: { choices: string[]; tally: Record<string, number> },
) {
  await page.evaluate(
    ({ prompt, revision, closed, signer, answer }) => {
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
          tally: answer?.tally ?? { approve: closed ? 1 : 0, deny: 0 },
          responders: answer
            ? [
                {
                  pubkey: "deadbeef".repeat(8),
                  event_id: "e".repeat(64),
                  created_at: Math.floor(Date.now() / 1000),
                  choices: answer.choices,
                },
              ]
            : [],
        }),
        extraTags: [["d", prompt]],
      });
    },
    { prompt: PROMPT, revision, closed, signer, answer },
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

// Controls are derived from the vendor behavior documented in
// docs/interaction-controls.md; the mock supplies transport, not the UI logic.
test("Discord control: a recorded poll vote can be changed while open", async ({
  page,
}) => {
  await seed(page, "poll");
  const card = page.getByTestId("interaction-card");
  await updateState(page, 1, false, RELAY, {
    choices: ["approve"],
    tally: { approve: 1, deny: 0 },
  });
  await expect(card.getByRole("radio", { name: /Approve/ })).toBeChecked();
  await expect(card.getByRole("status")).toContainText(
    "Your answer is recorded",
  );
  await card.getByRole("radio", { name: /Deny/ }).check();
  // An unrelated live tally must not erase a person's unsubmitted change.
  await updateState(page, 2, false, RELAY, {
    choices: ["approve"],
    tally: { approve: 2, deny: 3 },
  });
  await expect(card.getByRole("radio", { name: /Deny/ })).toBeChecked();
  await card.getByRole("button", { name: "Update answer" }).click();
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
  await updateState(page, 3, false, RELAY, {
    choices: ["deny"],
    tally: { approve: 1, deny: 4 },
  });
  await expect(card.getByRole("radio", { name: /Deny/ })).toBeChecked();
  await expect(card.getByRole("radio", { name: /Deny/ })).toHaveAccessibleName(
    "Deny 4",
  );
});

test("Discord select control: enforce min/max without trapping the current selection", async ({
  page,
}) => {
  await seed(page, "poll", [], true, [
    ["opt", "revise", "Revise"],
    ["min", "2"],
    ["max", "2"],
  ]);
  const card = page.getByTestId("interaction-card");
  const submit = card.getByRole("button", { name: "Submit answer" });
  await expect(submit).toBeDisabled();
  await card.getByRole("checkbox", { name: /Approve/ }).check();
  await expect(submit).toBeDisabled();
  await card.getByRole("checkbox", { name: /Deny/ }).check();
  await expect(submit).toBeEnabled();
  await expect(card.getByRole("checkbox", { name: /Revise/ })).toBeDisabled();
  await card.getByRole("checkbox", { name: /Deny/ }).uncheck();
  await card.getByRole("checkbox", { name: /Revise/ }).check();
  await submit.click();
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__BUZZ_E2E_SIGNED_EVENTS__
          ?.filter((e) => e.kind === 40011)
          .at(-1)
          ?.tags.filter((t) => t[0] === "choice"),
      ),
    )
    .toEqual([
      ["choice", "approve"],
      ["choice", "revise"],
    ]);
});

test("Teams close control: an idle card stops accepting input at the deadline without inventing a result", async ({
  page,
}) => {
  const deadline = Math.floor(Date.now() / 1000) + 3600;
  await page.clock.install();
  await seed(page, "poll", [], true, [["deadline", String(deadline)]]);
  const card = page.getByTestId("interaction-card");
  await expect(card.getByRole("radio", { name: /Approve/ })).toBeEnabled();
  await page.clock.fastForward(3_601_000);
  await expect(card.getByRole("status")).toContainText("Voting ended");
  await expect(card.getByRole("status")).toContainText("Waiting for the relay");
  await expect(card.getByRole("radio", { name: /Approve/ })).toBeDisabled();
  await updateState(page, 1, true);
  await expect(card.getByRole("status")).toContainText("Closed · Approve");
});

test("audience control: listed responders restrict answering while the channel can read", async ({
  page,
}) => {
  await seed(page, "buttons", [], true, [
    ["responders", "listed"],
    ["p", "f".repeat(64)],
  ]);
  const card = page.getByTestId("interaction-card");
  await expect(card.getByText(QUESTION)).toBeVisible();
  await expect(
    card.getByRole("button", { name: "Approve", exact: true }),
  ).toBeDisabled();
  await expect(
    card.getByText("This request is addressed to other members."),
  ).toBeVisible();
});

test("Slack form control: pending feedback, failure and retry preserve the user's input", async ({
  page,
}) => {
  await seed(page, "form", [["field", "note", "Note", "text", "required"]]);
  await page.evaluate(() => {
    const bridge = (
      window as unknown as {
        __TAURI_INTERNALS__: {
          invoke: (command: string, ...args: unknown[]) => Promise<unknown>;
        };
      }
    ).__TAURI_INTERNALS__;
    const invoke = bridge.invoke.bind(bridge);
    let fail = true;
    bridge.invoke = async (command, ...args) => {
      if (command === "sign_event" && fail) {
        fail = false;
        await new Promise((resolve) => setTimeout(resolve, 700));
        throw new Error("Control: signer temporarily unavailable");
      }
      return invoke(command, ...args);
    };
  });
  const card = page.getByTestId("interaction-card");
  await card.getByLabel("Note (required)").fill("Keep the second take");
  await card.getByRole("button", { name: "Submit answer" }).click();
  await expect(card.getByRole("status")).toHaveText("Sending answer…");
  await expect(
    card.getByRole("button", { name: "Submit answer" }),
  ).toBeDisabled();
  await expect(card.getByRole("alert")).toContainText(
    "signer temporarily unavailable",
  );
  await expect(card.getByLabel("Note (required)")).toHaveValue(
    "Keep the second take",
  );
  await expect(card.getByRole("status")).not.toContainText("recorded");
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
    .toContainEqual(["value", "note", "Keep the second take"]);
});

test("independent cards load their own prompt and pre-existing state in one channel", async ({
  page,
}) => {
  await seed(page);
  const second = "2".repeat(64);
  await page.evaluate(
    ({ relay, second }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      emit?.({
        channelName: "engineering",
        id: second,
        kind: 40010,
        content: "Second decision",
        extraTags: [
          ["itype", "buttons"],
          ["opt", "continue", "Continue"],
          ["closes", "first"],
          ["deadline", String(Math.floor(Date.now() / 1000) + 3600)],
        ],
      });
      emit?.({
        channelName: "engineering",
        id: "3".repeat(64),
        kind: 39010,
        pubkey: relay,
        content: JSON.stringify({
          version: 1,
          revision: 0,
          status: "open",
          close_reason: null,
          winner: null,
          tally: { continue: 0 },
          responders: [],
        }),
        extraTags: [["d", second]],
      });
      emit?.({
        channelName: "engineering",
        id: "4".repeat(64),
        kind: 9,
        pubkey: relay,
        content: "Second decision. Reply Continue.",
        extraTags: [["interaction", second]],
      });
    },
    { relay: RELAY, second },
  );
  const other = page
    .getByTestId("interaction-card")
    .filter({ hasText: "Second decision" });
  await expect(
    other.getByRole("button", { name: "Continue", exact: true }),
  ).toBeEnabled();
  await other.getByRole("button", { name: "Continue", exact: true }).click();
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_SIGNED_EVENTS__
            ?.filter((e) => e.kind === 40011)
            .at(-1)?.tags,
      ),
    )
    .toContainEqual(["e", second, "", "prompt"]);
  const first = page
    .getByTestId("interaction-card")
    .filter({ hasText: QUESTION });
  await expect(
    first.getByRole("button", { name: "Approve", exact: true }),
  ).toBeEnabled();
});
