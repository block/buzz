import { expect, test, type Locator, type Page } from "@playwright/test";

import { installMockBridge, TEST_IDENTITIES } from "../../helpers/bridge";
import {
  forwardTraceStep,
  installNativeProjectionTraceAdapter,
  loadCurrentBindingStatusTrace,
  traceStep,
  type NativeCurrentProjection,
  waitForNativeProjectionTraceAdapter,
} from "./currentBindingStatusTrace";

const trace = loadCurrentBindingStatusTrace();
const LEGACY_VERIFIED_NAME_MARKER = "legacy-verified-name-must-not-authorize";
const LEGACY_ALIAS_MARKER = "legacy-relay-alias-must-not-authorize";
const PROJECTION_SETUP_HEADROOM_SECONDS = 60;

async function waitForMockLiveSubscription(page: Page) {
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
            channelName: "general",
          }) ?? false,
      ),
    )
    .toBe(true);
}

function otherSyntheticAuthor(projectedAuthors: ReadonlySet<string>): string {
  for (const identity of [TEST_IDENTITIES.bob, TEST_IDENTITIES.charlie]) {
    if (!projectedAuthors.has(identity.pubkey)) return identity.pubkey;
  }
  throw new Error(
    "Native trace unexpectedly contains both comparison authors.",
  );
}

async function emitMessage(
  page: Page,
  input: { content: string; pubkey: string; createdAt: number },
) {
  await page.evaluate((message) => {
    const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
    if (!emit) throw new Error("Mock live-message bridge is not installed.");
    emit({ channelName: "general", ...message });
  }, input);
}

async function expectOnlyAuthorBadge(
  page: Page,
  rows: ReadonlyMap<string, Locator>,
  projection: NativeCurrentProjection,
) {
  const matchingRow = rows.get(projection.eventAuthorPubkey);
  if (!matchingRow) {
    throw new Error("No message row was created for the projected author.");
  }

  const badge = matchingRow.getByTestId("current-relay-binding");
  await expect(badge).toHaveCount(1);
  await expect(badge).toHaveAccessibleName("Current relay binding");
  await expect(page.getByTestId("current-relay-binding")).toHaveCount(1);

  for (const [author, row] of rows) {
    if (author !== projection.eventAuthorPubkey) {
      await expect(row.getByTestId("current-relay-binding")).toHaveCount(0);
    }
  }

  const badgeMarkup = (
    await badge.evaluate((element) => element.outerHTML)
  ).toLowerCase();
  for (const hiddenValue of [
    projection.eventAuthorPubkey,
    String(projection.freshUntil),
    projection.connectionEpoch,
    LEGACY_VERIFIED_NAME_MARKER,
    LEGACY_ALIAS_MARKER,
    "eventauthorpubkey",
    "freshuntil",
    "connectionepoch",
  ]) {
    expect(badgeMarkup).not.toContain(hiddenValue.toLowerCase());
  }
}

async function expectNoLegacyTrustPresentation(page: Page) {
  await expect(page.getByTestId("relay-verified-identity")).toHaveCount(0);
  await expect(
    page.locator('[aria-label^="Relay-verified identity"]'),
  ).toHaveCount(0);
  await expect(page.getByText("Binding active", { exact: false })).toHaveCount(
    0,
  );
  await expect(page.getByText("Verified as", { exact: false })).toHaveCount(0);
}

test("Rust native-flow trace drives exact-author lifecycle presentation", async ({
  page,
}) => {
  const currentProjections = trace.steps.flatMap((step) =>
    step.projection === null ? [] : [step.projection],
  );
  const projectedAuthors = new Set(
    currentProjections.map((projection) => projection.eventAuthorPubkey),
  );
  const expiryProjection = currentProjections.reduce((earliest, projection) =>
    projection.freshUntil < earliest.freshUntil ? projection : earliest,
  );
  const activationStep = traceStep(trace, "current");
  const activationProjection = activationStep.projection;
  if (activationProjection === null) {
    throw new Error("Native current trace step must contain a projection.");
  }
  const clockStartSeconds =
    expiryProjection.freshUntil - PROJECTION_SETUP_HEADROOM_SECONDS;
  if (clockStartSeconds <= 0) {
    throw new Error(
      "Native trace freshUntil is too small for expiry coverage.",
    );
  }

  await page.clock.install({ time: clockStartSeconds * 1_000 });
  await installMockBridge(page, {
    searchProfiles: [...projectedAuthors].map((pubkey, index) => ({
      pubkey,
      displayName: `${LEGACY_VERIFIED_NAME_MARKER}-${index}`,
      nip05Handle: `${LEGACY_ALIAS_MARKER}-${index}@example.invalid`,
    })),
  });
  await installNativeProjectionTraceAdapter(page);
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await waitForMockLiveSubscription(page);
  await waitForNativeProjectionTraceAdapter(page);

  const rows = new Map<string, Locator>();
  let createdAt = clockStartSeconds - projectedAuthors.size - 1;
  for (const [index, pubkey] of [...projectedAuthors].entries()) {
    const content = `Native projection author ${index}`;
    await emitMessage(page, { content, pubkey, createdAt: createdAt++ });
    const row = page.getByTestId("message-row").filter({ hasText: content });
    await expect(row).toBeVisible();
    await expect(row.getByTestId("message-author")).toContainText(
      `${LEGACY_VERIFIED_NAME_MARKER}-${index}`,
    );
    rows.set(pubkey, row);
  }

  const otherAuthor = otherSyntheticAuthor(projectedAuthors);
  const otherContent = "Non-projected comparison author";
  await emitMessage(page, {
    content: otherContent,
    pubkey: otherAuthor,
    createdAt,
  });
  const otherRow = page
    .getByTestId("message-row")
    .filter({ hasText: otherContent });
  await expect(otherRow).toBeVisible();
  rows.set(otherAuthor, otherRow);

  for (const step of trace.steps) {
    if (step.case === "passive-expiry") continue;

    await test.step(`${step.case} projects its retained browser state`, async () => {
      if (step.projection === null) {
        // Every clear transition starts from a visible Rust-produced current
        // projection so a pre-cleared store can never make the assertion pass.
        await forwardTraceStep(page, activationStep);
        await expectOnlyAuthorBadge(page, rows, activationProjection);
      }

      await forwardTraceStep(page, step);
      if (step.projection === null) {
        await expect(page.getByTestId("current-relay-binding")).toHaveCount(0);
      } else {
        await expectOnlyAuthorBadge(page, rows, step.projection);
      }
    });
  }

  // The existing mock profile seed reaches ordinary displayName/NIP-05 fields
  // but intentionally exposes no dormant verifiedName field. Prove those
  // legacy-looking markers cannot enter trust-specific badge or panel UI.
  const activationRow = rows.get(activationProjection.eventAuthorPubkey);
  if (!activationRow) throw new Error("Activation author row is absent.");
  await activationRow.getByRole("button").first().click();
  const profilePanel = page.getByTestId("user-profile-panel");
  await expect(profilePanel).toBeVisible();
  await expect(profilePanel).toContainText(LEGACY_VERIFIED_NAME_MARKER);
  await expect(profilePanel).toContainText(LEGACY_ALIAS_MARKER);
  await expect(profilePanel.getByTestId("relay-verified-identity")).toHaveCount(
    0,
  );
  await expect(
    profilePanel.locator('[aria-label^="Relay-verified identity"]'),
  ).toHaveCount(0);
  await expect(
    profilePanel.getByText("Binding active", { exact: false }),
  ).toHaveCount(0);
  await expect(
    profilePanel.getByText("Verified as", { exact: false }),
  ).toHaveCount(0);
  await page.getByTestId("auxiliary-panel-close").click();
  await expect(profilePanel).toHaveCount(0);

  // These native lifecycle outputs must all clear presentation. Naming them
  // explicitly keeps this browser layer non-vacuous if the trace grows later.
  for (const caseName of [
    "withdrawal",
    "passive-expiry",
    "disconnect",
    "logout",
    "restart",
    "relay-scope-change",
    "signer-scope-change",
    "author-scope-change",
    "domain-scope-change",
    "epoch-scope-change",
    "profile-spoof",
    "nip85-no-fallback",
  ] as const) {
    expect(traceStep(trace, caseName).projection).toBeNull();
  }
  expect(traceStep(trace, "reconnect").projection).not.toBeNull();
  await expectNoLegacyTrustPresentation(page);

  // Deliver an unchanged DTO produced by Rust while the browser clock is
  // before its deadline, then advance to the exclusive boundary. No later
  // trace event, render fixture, or navigation clears it.
  const expiryStep = trace.steps.find(
    (step) => step.projection === expiryProjection,
  );
  if (!expiryStep) throw new Error("Expiry projection is absent from trace.");
  await forwardTraceStep(page, expiryStep);
  await expectOnlyAuthorBadge(page, rows, expiryProjection);
  await page.clock.fastForward(
    (expiryProjection.freshUntil - clockStartSeconds) * 1_000,
  );
  await expect(page.getByTestId("current-relay-binding")).toHaveCount(0);
  await expectNoLegacyTrustPresentation(page);
});
