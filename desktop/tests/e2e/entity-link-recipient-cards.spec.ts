import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";

const SHOTS = "test-results/entity-link-recipient-cards";

// Regression coverage for buzz:// entity links posted WITHOUT sender
// snapshot tags (CLI / agent senders): #3818 moved external-link previews to
// sender-authored snapshots, which silently killed the recipient-side
// buzz://pr|issue|repo cards from #4695. These cards resolve their titles
// from the active relay itself, so they must render for recipients even when
// the message carries no link-preview tags.

const ALICE_PUBKEY = TEST_IDENTITIES.alice.pubkey;
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const REPO_ADDRESS = `30617:${ALICE_PUBKEY}:relay-tools`;
const PR_ID = `e0${"ca4d".repeat(15)}ff`; // 64-hex event id
const PR_SUBJECT = "Restore recipient-side entity cards";
const ISSUE_ID = `f0${"1a2b".repeat(15)}ee`; // 64-hex event id
const ISSUE_SUBJECT = "Reopen identical issue links";

test("agent-style message with bare buzz:// links renders entity cards without snapshot tags", async ({
  page,
}) => {
  await page.addInitScript(
    ({ repoAddress, prId, alicePubkey, subject }) => {
      window.__BUZZ_E2E_EXTRA_PROJECT_EVENTS__ = [
        {
          id: prId,
          kind: 1618, // KIND_GIT_PULL_REQUEST
          pubkey: alicePubkey,
          created_at: Math.floor(Date.now() / 1000) - 60,
          content: "PR body",
          tags: [
            ["a", repoAddress],
            ["subject", subject],
            ["c", "abc123".padEnd(40, "0")],
            ["branch-name", "fix/entity-cards"],
            ["clone", "https://github.com/block/relay-tools.git"],
          ],
        },
      ];
    },
    {
      repoAddress: REPO_ADDRESS,
      prId: PR_ID,
      alicePubkey: ALICE_PUBKEY,
      subject: PR_SUBJECT,
    },
  );
  await installMockBridge(page);
  await page.setViewportSize({ width: 900, height: 800 });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  // Simulate an agent/CLI sender: plain kind-9 message with bare buzz://
  // URLs in the content and NO link-preview snapshot tags.
  await page.evaluate(
    ({ prId, alicePubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        pubkey: alicePubkey,
        content: [
          "PR is up — review when you can:",
          `buzz://pr?id=${prId}&owner=${alicePubkey}&d=relay-tools`,
          `Repo: buzz://repo?owner=${alicePubkey}&d=relay-tools`,
        ].join("\n"),
      });
    },
    { prId: PR_ID, alicePubkey: ALICE_PUBKEY },
  );

  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "PR is up" })
    .last();
  await expect(row).toBeVisible();

  // The PR card resolves builder metadata from the signed root and repository.
  const prCard = row.locator('[data-link-preview="buzz-pull-request"]');
  await expect(prCard).toBeVisible();
  await expect(prCard).toContainText("relay-tools");
  await expect(prCard).toContainText(PR_SUBJECT);
  await expect(prCard).toContainText(
    "Open · fix/entity-cards → main · abc1230",
  );
  await expect(prCard).toHaveAttribute("data-image-state", "none");
  await expect(prCard.locator("[data-link-preview-thumbnail]")).toHaveCount(0);
  await expect(
    prCard.locator("[data-link-preview-hostname-buzz-mark]"),
  ).toBeVisible();
  await expect(
    prCard.locator("[data-link-preview-hostname-favicon]"),
  ).toHaveCount(0);

  // The repository card uses its signed announcement metadata and remains
  // image-less.
  const repoCard = row.locator('[data-link-preview="buzz-repository"]');
  await expect(repoCard).toBeVisible();
  await expect(repoCard).toContainText("relay-tools");
  await expect(repoCard).toContainText(
    "Operator tooling and admin CLI for relay deployments.",
  );
  await expect(repoCard).toContainText("active · default: main");
  await expect(repoCard).toHaveAttribute("data-image-state", "none");
  await expect(repoCard.locator("[data-link-preview-thumbnail]")).toHaveCount(
    0,
  );
  await expect(
    repoCard.locator("[data-link-preview-hostname-buzz-mark]"),
  ).toBeVisible();
  await expect(
    repoCard.locator("[data-link-preview-hostname-favicon]"),
  ).toHaveCount(0);
  // Default typography is 14px; keep the image-less card compact while
  // allowing fractional line-height rounding across rendering platforms.
  expect(
    await repoCard.evaluate((card) => card.getBoundingClientRect().height),
  ).toBeLessThan(90);

  await waitForAnimations(page);
  await page.screenshot({
    animations: "disabled",
    path: `${SHOTS}/01-recipient-entity-cards.png`,
  });
});

test("desktop composer shows entity card and send is not blocked by missing snapshot", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.setViewportSize({ width: 900, height: 800 });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();

  const repoLink = `buzz://repo?owner=${ALICE_PUBKEY}&d=relay-tools`;
  await page.getByTestId("message-input").fill(`Check out ${repoLink}`);

  // buzz:// links never produce snapshot tags, so the composer card must
  // show as done (not stuck "processing") with zero ready snapshots.
  const composerCard = page
    .locator("[data-composer-link-previews]")
    .locator('[data-link-preview="buzz-repository"]');
  await expect(composerCard).toBeVisible();
  await expect(page.locator("[data-composer-link-previews]")).toHaveAttribute(
    "data-ready-snapshot-count",
    "0",
  );

  await waitForAnimations(page);
  await page.screenshot({
    animations: "disabled",
    path: `${SHOTS}/02-composer-entity-card.png`,
  });

  await page.getByTestId("send-message").click();

  const row = page.getByTestId("message-row").last();
  const repoCard = row.locator('[data-link-preview="buzz-repository"]');
  await expect(repoCard).toBeVisible();
  await expect(repoCard).toContainText("relay-tools");

  await waitForAnimations(page);
  await page.screenshot({
    animations: "disabled",
    path: `${SHOTS}/03-sent-entity-card.png`,
  });
});

test("reopening the same entity link reapplies its workspace state", async ({
  page,
}) => {
  const repoAddress = `30617:${DEFAULT_MOCK_PUBKEY}:buzz`;
  await page.addInitScript(
    ({ issueId, issueSubject, prId, prSubject, repoAddress, owner }) => {
      const createdAt = Math.floor(Date.now() / 1000) - 60;
      window.__BUZZ_E2E_EXTRA_PROJECT_EVENTS__ = [
        {
          id: prId,
          kind: 1618, // KIND_GIT_PULL_REQUEST
          pubkey: owner,
          created_at: createdAt,
          content: "PR body",
          tags: [
            ["a", repoAddress],
            ["subject", prSubject],
            ["c", "abc123".padEnd(40, "0")],
            ["branch-name", "fix/reopen-entity-link"],
          ],
        },
        {
          id: issueId,
          kind: 1621, // KIND_GIT_ISSUE
          pubkey: owner,
          created_at: createdAt,
          content: "Issue body",
          tags: [
            ["a", repoAddress],
            ["subject", issueSubject],
          ],
        },
      ];
    },
    {
      issueId: ISSUE_ID,
      issueSubject: ISSUE_SUBJECT,
      prId: PR_ID,
      prSubject: PR_SUBJECT,
      repoAddress,
      owner: DEFAULT_MOCK_PUBKEY,
    },
  );
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("open-projects-view")).toBeVisible();
  const repoLink = `buzz://repo?owner=${DEFAULT_MOCK_PUBKEY}&d=buzz&tab=prs`;
  const prLink = `buzz://pr?id=${PR_ID}&owner=${DEFAULT_MOCK_PUBKEY}&d=buzz`;
  const issueLink = `buzz://issue?id=${ISSUE_ID}&owner=${DEFAULT_MOCK_PUBKEY}&d=buzz`;
  const emitEntityLink = async (link: string) => {
    await page.waitForFunction(
      () => typeof window.__TAURI_INTERNALS__?.invoke === "function",
    );
    await page.evaluate(
      (href) =>
        window.__TAURI_INTERNALS__?.invoke?.("plugin:event|emit", {
          event: "deep-link-entity",
          payload: href,
        }),
      link,
    );
  };

  await emitEntityLink(repoLink);
  const pullRequestsTab = page.getByRole("tab", {
    name: "Review",
    exact: true,
  });
  await expect(pullRequestsTab).toHaveAttribute("aria-selected", "true");

  const breadcrumb = page.getByRole("navigation", {
    name: "Project breadcrumb",
  });
  await breadcrumb.getByRole("button").nth(1).click();
  await expect(page.getByRole("tab", { name: "Overview" })).toHaveAttribute(
    "aria-selected",
    "true",
  );

  await emitEntityLink(repoLink);
  await expect(pullRequestsTab).toHaveAttribute("aria-selected", "true");

  const filesTab = page.getByRole("tab", { name: "Files", exact: true });
  await filesTab.click();
  await expect(filesTab).toHaveAttribute("aria-selected", "true");

  await emitEntityLink(repoLink);
  await expect(pullRequestsTab).toHaveAttribute("aria-selected", "true");

  await emitEntityLink(prLink);
  const prHeading = page.getByRole("heading", { name: PR_SUBJECT });
  await expect(prHeading).toBeVisible();
  await breadcrumb.getByRole("button", { name: "Review", exact: true }).click();
  await expect(prHeading).toHaveCount(0);
  await emitEntityLink(prLink);
  await expect(prHeading).toBeVisible();

  await emitEntityLink(issueLink);
  const issueHeading = page.getByRole("heading", { name: ISSUE_SUBJECT });
  await expect(issueHeading).toBeVisible();
  await breadcrumb.getByRole("button", { name: "Tasks", exact: true }).click();
  await expect(issueHeading).toHaveCount(0);
  await emitEntityLink(issueLink);
  await expect(issueHeading).toBeVisible();
});

test("cold-start entity links drain after the React listener mounts", async ({
  page,
}) => {
  const href = `buzz://repo?owner=${DEFAULT_MOCK_PUBKEY}&d=buzz&tab=prs`;
  await installMockBridge(page, {
    pendingEntityDeepLinks: [{ id: "cold-start-project", href }],
  });

  await page.goto("/", { waitUntil: "domcontentloaded" });

  await expect(
    page.getByRole("tab", { name: "Review", exact: true }),
  ).toHaveAttribute("aria-selected", "true");
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__TAURI_INTERNALS__?.invoke?.(
          "take_pending_entity_deep_link",
          {},
        ),
      ),
    )
    .toBeNull();
});
