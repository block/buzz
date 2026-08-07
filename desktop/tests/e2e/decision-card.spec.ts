import { expect, test } from "@playwright/test";
import { sha256 } from "@noble/hashes/sha2.js";
import { bytesToHex } from "@noble/hashes/utils.js";

import { installMockBridge } from "../helpers/bridge";

const CARD_KIND = 40009;
const RESPONSE_KIND = 40010;
const CARD_ID = "550e8400-e29b-41d4-a716-446655440000";
const CARD_PAYLOAD = {
  schema_version: 1,
  card_id: CARD_ID,
  title: "Approve corrected redraft",
  situation: "Case #625 has a corrected draft.",
  recommendation: "Approve the corrected wording.",
  proposed_action: "Record shadow intent only.",
  risk: "No external send and no production write.",
  record_url: "https://stomaton.example/cases/625",
  choices: ["approve", "redraft", "escalate", "reject"],
  expires_at: 2_100_000_000,
  shadow: true,
};
const ENCODED_CARD = JSON.stringify(CARD_PAYLOAD);
const PAYLOAD_HASH = bytesToHex(sha256(new TextEncoder().encode(ENCODED_CARD)));

async function emitDecisionCard(page: Parameters<typeof test>[0]["page"]) {
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  await page.evaluate(
    ({ encodedCard, hash, kind }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content:
          "## Decision needed\nApprove corrected redraft\n\n**SHADOW — NOT DELIVERED**",
        kind,
        extraTags: [
          ["decision_card", encodedCard],
          ["payload_hash", hash],
          ["shadow", "1"],
        ],
      });
    },
    { encodedCard: ENCODED_CARD, hash: PAYLOAD_HASH, kind: CARD_KIND },
  );
}

test("renders and records a signed shadow decision inside the Buzz timeline", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/");
  await emitDecisionCard(page);

  await page.getByTestId("channel-general").click();
  const card = page.getByTestId("decision-card");
  await expect(card).toBeVisible();
  await expect(card.getByText("Approve corrected redraft")).toBeVisible();
  await expect(card.getByText("Shadow", { exact: true })).toBeVisible();

  await card.getByRole("button", { name: "Approve" }).click();

  await expect(page.getByTestId("decision-receipt-card")).toContainText(
    "Approved",
  );
  await expect(page.getByTestId("decision-receipt-card")).toContainText(
    "NOT DELIVERED",
  );

  const signed = await page.evaluate(
    () => window.__BUZZ_E2E_SIGNED_EVENTS__ ?? [],
  );
  const response = signed.find((event) => event.kind === RESPONSE_KIND);
  expect(response).toBeTruthy();
  expect(response?.tags).toContainEqual(["payload_hash", PAYLOAD_HASH]);
  expect(response?.tags).toContainEqual(["shadow", "1"]);
  expect(response?.tags.some((tag) => tag[0] === "decision_response")).toBe(
    true,
  );
});

test("adapts the native decision card to Buzz dark mode", async ({ page }) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("buzz-theme", "buzz-dark");
  });
  await installMockBridge(page);
  await page.goto("/");
  await emitDecisionCard(page);

  await page.getByTestId("channel-general").click();
  await expect(page.locator("html")).toHaveClass(/dark/);

  const card = page.getByTestId("decision-card");
  await expect(card).toBeVisible();
  await expect(card.getByText("Approve corrected redraft")).toBeVisible();
  await expect(card.getByRole("button", { name: "Approve" })).toBeVisible();

  const colors = await card.evaluate((element) => {
    const style = window.getComputedStyle(element);
    return { background: style.backgroundColor, foreground: style.color };
  });
  expect(colors.background).not.toBe("rgba(0, 0, 0, 0)");
  expect(colors.background).not.toBe(colors.foreground);

  await card.screenshot({ path: "test-results/decision-card/dark.png" });
});
