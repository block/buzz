import { expect, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import {
  installBridge,
  installMockBridge,
  TEST_IDENTITIES,
} from "../helpers/bridge";

const SHOTS = "test-results/entity-link-recipient-cards";

// Regression coverage for Buzz-native entity links after standalone cards were
// removed. Raw and authored entity links keep their inline navigation and
// relay-backed metadata tooltips, while external sender snapshots still render.

const ALICE_PUBKEY = TEST_IDENTITIES.alice.pubkey;
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
const REPO_ADDRESS = `30617:${ALICE_PUBKEY}:relay-tools`;
const PR_ID = `e0${"ca4d".repeat(15)}ff`; // 64-hex event id
const PR_SUBJECT = "Restore recipient-side entity cards";
const ISSUE_ID = `f0${"1a2b".repeat(15)}ee`; // 64-hex event id
const ISSUE_SUBJECT =
  "Smoke test: issue tracking on relay-tools with a deliberately long title";
const EXTERNAL_HREF = "https://example.com/entity-chip-control";
const RELAY_ORIGIN = "http://localhost:3000";
const CLONE_HREF = `${RELAY_ORIGIN}/git/${ALICE_PUBKEY}/relay-tools.git`;

test("agent-style Buzz links stay chip-only with metadata tooltips", async ({
  page,
}) => {
  await page.addInitScript(
    ({ repoAddress, prId, issueId, alicePubkey, prSubject, issueSubject }) => {
      const createdAt = Math.floor(Date.now() / 1000) - 60;
      window.__BUZZ_E2E_EXTRA_PROJECT_EVENTS__ = [
        {
          id: prId,
          kind: 1618, // KIND_GIT_PULL_REQUEST
          pubkey: alicePubkey,
          created_at: createdAt,
          content: "PR body",
          tags: [
            ["a", repoAddress],
            ["subject", prSubject],
            ["c", "abc123".padEnd(40, "0")],
            ["branch-name", "fix/entity-cards"],
            ["clone", "https://github.com/block/relay-tools.git"],
          ],
        },
        {
          id: issueId,
          kind: 1621, // KIND_GIT_ISSUE
          pubkey: alicePubkey,
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
      repoAddress: REPO_ADDRESS,
      prId: PR_ID,
      issueId: ISSUE_ID,
      alicePubkey: ALICE_PUBKEY,
      prSubject: PR_SUBJECT,
      issueSubject: ISSUE_SUBJECT,
    },
  );
  await installMockBridge(page);
  await page.setViewportSize({ width: 900, height: 800 });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  // Simulate an agent/CLI sender: plain kind-9 message with angle-bracket
  // buzz:// URLs in a Markdown list and NO link-preview snapshot tags.
  await page.evaluate(
    ({ prId, issueId, alicePubkey, externalHref, cloneHref }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        pubkey: alicePubkey,
        content: [
          "PR is up — review when you can:",
          "",
          `- Pull request with enough leading context to wrap: <buzz://pr?id=${prId}&owner=${alicePubkey}&d=relay-tools>`,
          `- Issue: <buzz://issue?id=${issueId}&owner=${alicePubkey}&d=relay-tools>`,
          `- Repository: <buzz://repo?owner=${alicePubkey}&d=relay-tools>`,
          `- Labeled issue: [triage this issue](buzz://issue?id=${issueId}&owner=${alicePubkey}&d=relay-tools)`,
          `- Labeled clone: [clone relay-tools](${cloneHref})`,
          `- Missing repo: <buzz://repo?owner=${alicePubkey}&d=missing-repo>`,
          `- External control: <${externalHref}>`,
        ].join("\n"),
        extraTags: [
          [
            "link-preview",
            "snapshot",
            "1",
            externalHref,
            "External preview survives",
            "Example",
            "Sender-authored external metadata",
            "",
            "",
            "",
            "",
          ],
        ],
      });
    },
    {
      prId: PR_ID,
      issueId: ISSUE_ID,
      alicePubkey: ALICE_PUBKEY,
      externalHref: EXTERNAL_HREF,
      cloneHref: CLONE_HREF,
    },
  );

  const row = page
    .getByTestId("message-row")
    .filter({ hasText: "PR is up" })
    .last();
  await expect(row).toBeVisible();

  await expect(row.locator('[data-link-preview^="buzz-"]')).toHaveCount(0);
  const externalCard = row.locator('[data-link-preview="generic-link"]');
  await expect(externalCard).toBeVisible();
  await expect(externalCard).toContainText("External preview survives");
  const prChip = row.getByRole("button", {
    name: /Open pull request .* in repository relay-tools/,
  });
  await expect(prChip).toHaveAccessibleName(
    `Open pull request ${PR_ID.slice(0, 8)} in repository relay-tools: relay-tools · ${PR_SUBJECT}`,
  );
  await expect(prChip).not.toHaveAttribute("title");
  await expect(prChip).toHaveClass(/wrapping-inline-chip/);
  await expect(prChip).toHaveCSS("display", "inline");
  await expect(prChip.locator(".truncate")).toHaveCount(0);
  await expect(prChip).toHaveText("relay-tools");
  await expect(prChip).not.toContainText(PR_SUBJECT);
  await expect(prChip).not.toContainText(PR_ID.slice(0, 8));
  // Fetched subjects stay out of the inline chip, so metadata resolution cannot
  // resize the surrounding message.
  await prChip.hover();
  const prTooltip = page.getByRole("tooltip");
  const prContext = prTooltip.locator(
    '[data-buzz-tooltip-metadata-content=""]',
  );
  await expect(prContext).toHaveText(PR_SUBJECT);
  await expect(prContext).toHaveClass(/line-clamp-3/);
  await expect(prContext).toHaveCSS("overflow-wrap", "anywhere");
  const prFooter = prTooltip.locator('[data-buzz-tooltip-metadata-type=""]');
  await expect(prFooter).toHaveText("Pull request · relay-tools");
  await expect(prFooter).toHaveCSS("overflow-wrap", "anywhere");
  await expect(prFooter).toHaveCSS("white-space", "normal");
  const tooltipSemanticColors = await prTooltip.evaluate((element) => {
    const styles = getComputedStyle(element);
    const probe = document.createElement("span");
    probe.style.backgroundColor = "hsl(var(--popover))";
    probe.style.color = "hsl(var(--popover-foreground))";
    document.body.append(probe);
    const semanticStyles = getComputedStyle(probe);
    const result = {
      actual: [styles.backgroundColor, styles.color],
      expected: [semanticStyles.backgroundColor, semanticStyles.color],
    };
    probe.remove();
    return result;
  });
  expect(tooltipSemanticColors.actual).toEqual(tooltipSemanticColors.expected);
  await expect(prChip).toHaveText("relay-tools");

  const issueChip = row.locator('[data-buzz-link-kind="issue"]');
  // The issue chip is the repository name alone — resolved metadata never
  // reaches the inline label, so it neither absorbs the title nor falls back
  // to the event hash.
  await expect(issueChip).toHaveText("relay-tools");
  await expect(issueChip).toHaveAccessibleName(
    `Open issue ${ISSUE_ID.slice(0, 8)} in repository relay-tools: relay-tools · ${ISSUE_SUBJECT}`,
  );
  await expect(issueChip).not.toContainText(ISSUE_SUBJECT);
  await expect(issueChip).not.toContainText(ISSUE_ID.slice(0, 8));
  await expect(issueChip).toHaveClass(/wrapping-inline-chip/);
  await expect(issueChip).toHaveCSS("display", "inline");
  await expect(issueChip.locator(".truncate")).toHaveCount(0);
  await issueChip.hover();
  const issueTooltip = page.getByRole("tooltip");
  const issueContext = issueTooltip.locator(
    '[data-buzz-tooltip-metadata-content=""]',
  );
  await expect(issueContext).toHaveText(ISSUE_SUBJECT);
  await expect(issueContext).toHaveClass(/line-clamp-3/);
  await expect(issueContext).toHaveCSS("overflow-wrap", "anywhere");
  await expect(issueContext).toHaveCSS("white-space", "normal");
  await expect
    .poll(() =>
      issueContext.evaluate(
        (element) => element.scrollHeight <= element.clientHeight,
      ),
    )
    .toBe(true);
  await expect(
    issueTooltip.locator('[data-buzz-tooltip-metadata-type=""]'),
  ).toHaveText("Issue · relay-tools");

  const labeledIssue = row
    .getByRole("button", { name: /Open issue .* in repository relay-tools:/ })
    .filter({ hasText: "triage this issue" });
  await expect(labeledIssue).toBeVisible();
  await labeledIssue.hover();
  await expect(
    page
      .getByRole("tooltip")
      .locator('[data-buzz-tooltip-metadata-content=""]'),
  ).toHaveText(ISSUE_SUBJECT);

  // Repository metadata remains available from its inline chip.
  const repoChip = row.getByRole("button", {
    name: "Open repository relay-tools",
    exact: true,
  });
  await repoChip.hover();
  const repoTooltip = page.getByRole("tooltip");
  await expect(
    repoTooltip.locator('[data-buzz-tooltip-metadata-content=""]'),
  ).toContainText("Operator tooling and admin CLI for relay deployments.");
  await expect(
    repoTooltip.locator('[data-buzz-tooltip-metadata-type=""]'),
  ).toHaveText("Repository");

  const labeledClone = row
    .getByRole("button", { name: "Open repository relay-tools" })
    .filter({ hasText: "clone relay-tools" });
  await expect(labeledClone).toBeVisible();
  await expect(labeledClone).toHaveClass(/underline/);
  await expect(labeledClone).not.toHaveClass(/wrapping-inline-chip/);
  await expect(labeledClone).toHaveAttribute("title", CLONE_HREF);
  await labeledClone.hover();
  const cloneTooltip = page.getByRole("tooltip");
  await expect(
    cloneTooltip.locator('[data-buzz-tooltip-metadata-content=""]'),
  ).toContainText("Operator tooling and admin CLI for relay deployments.");
  await labeledClone.click();
  await expect(page.locator("[data-project-detail-screen]")).toBeVisible();
  await page.getByTestId("channel-general").click();

  const missingRepoChip = row.getByRole("button", {
    name: "Open repository missing-repo",
  });
  await expect(missingRepoChip).not.toHaveClass(/buzz-link-unavailable/);
  const missingRepoColors = await missingRepoChip.evaluate((element) => {
    const styles = getComputedStyle(element);
    const probe = document.createElement("span");
    probe.style.backgroundColor = "hsl(var(--primary) / 0.15)";
    probe.style.color = "hsl(var(--primary))";
    document.body.append(probe);
    const semanticStyles = getComputedStyle(probe);
    const result = {
      actual: [styles.backgroundColor, styles.color],
      expected: [semanticStyles.backgroundColor, semanticStyles.color],
    };
    probe.remove();
    return result;
  });
  expect(missingRepoColors.actual).toEqual(missingRepoColors.expected);
  // Definitive metadata misses keep the tooltip's byline without repeating
  // the chip's stable identity.
  await missingRepoChip.hover();
  const missingRepoTooltip = page.getByRole("tooltip");
  await expect(
    missingRepoTooltip.locator('[data-buzz-tooltip-metadata-content=""]'),
  ).toHaveCount(0);
  const missingRepoFooter = missingRepoTooltip.locator(
    '[data-buzz-tooltip-metadata-type=""]',
  );
  await expect(missingRepoFooter).toHaveText("Repository");
  await expect(missingRepoFooter).toHaveClass(/text-secondary-foreground\/80/);
  await expect(missingRepoFooter).not.toHaveClass(/text-primary-foreground/);

  await waitForAnimations(page);
  await page.screenshot({
    animations: "disabled",
    path: `${SHOTS}/01-recipient-entity-chips.png`,
  });
});

test("issue chip width is metadata-independent while the title loads", async ({
  page,
}) => {
  await page.addInitScript(
    ({ repoAddress, issueId, alicePubkey, issueSubject }) => {
      window.__BUZZ_E2E_EXTRA_PROJECT_EVENTS__ = [
        {
          id: issueId,
          kind: 1621, // KIND_GIT_ISSUE
          pubkey: alicePubkey,
          created_at: Math.floor(Date.now() / 1000) - 60,
          content: "Issue body",
          tags: [
            ["a", repoAddress],
            ["subject", issueSubject],
          ],
        },
      ];
    },
    {
      repoAddress: REPO_ADDRESS,
      issueId: ISSUE_ID,
      alicePubkey: ALICE_PUBKEY,
      issueSubject: ISSUE_SUBJECT,
    },
  );
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();
  // No relay rate limit here: a rate-limited entity fetch caches a negative
  // result and never recovers, which would hide the resolved-title half of
  // this invariant. Natural relay latency supplies the pending window.
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );
  await page.evaluate(
    ({ issueId, alicePubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        pubkey: alicePubkey,
        content: `Issue link: buzz://issue?id=${issueId}&owner=${alicePubkey}&d=relay-tools`,
      });
    },
    { issueId: ISSUE_ID, alicePubkey: ALICE_PUBKEY },
  );

  const issueChip = page.getByRole("button", {
    name: /Open issue .* in repository relay-tools/,
  });
  await expect(issueChip).toHaveText("relay-tools");
  await issueChip.hover();

  const tooltipContent = page
    .getByRole("tooltip")
    .locator('[data-buzz-tooltip-metadata-content=""]');
  const widths = new Set<number>();
  const tooltipSamples: string[] = [];
  await expect
    .poll(
      async () => {
        const box = await issueChip.boundingBox();
        if (box) widths.add(Math.round(box.width));
        const text = (await tooltipContent.count())
          ? ((await tooltipContent.textContent()) ?? "")
          : "";
        tooltipSamples.push(text);
        return text;
      },
      { timeout: 15_000 },
    )
    .toBe(ISSUE_SUBJECT);

  // At least one sample predates the resolved title, so the widths below span
  // the load transition rather than only its settled end.
  expect(tooltipSamples.length).toBeGreaterThan(1);
  expect(tooltipSamples[0]).not.toBe(ISSUE_SUBJECT);
  // One width the whole way, and the label never left the repository name.
  expect(Array.from(widths)).toHaveLength(1);
  await expect(issueChip).toHaveText("relay-tools");
});

test("entity tooltip uses project context while relay metadata is delayed", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();
  await page.evaluate(() =>
    window.__BUZZ_E2E_ACTIVATE_RELAY_RATE_LIMIT__?.(300),
  );
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );
  await page.evaluate(
    ({ issueId, owner }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: `Delayed issue: buzz://issue?id=${issueId}&owner=${owner}&d=buzz`,
      });
    },
    { issueId: ISSUE_ID, owner: DEFAULT_MOCK_PUBKEY },
  );

  const issueChip = page.getByRole("button", {
    name: /Open issue .* in repository buzz/,
  });
  await issueChip.hover();
  await expect(
    page
      .getByRole("tooltip")
      .locator('[data-buzz-tooltip-metadata-content=""]'),
  ).toHaveText("buzz · The complete Buzz community platform.");
});

test("desktop composer and sent message keep Buzz entities chip-only", async ({
  page,
}) => {
  await installMockBridge(page);
  await page.setViewportSize({ width: 900, height: 800 });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();

  const repoLink = `buzz://repo?owner=${ALICE_PUBKEY}&d=relay-tools`;
  await page.getByTestId("message-input").fill(`Check out ${repoLink}`);

  // Buzz-native links do not enter the standalone composer-preview surface.
  await expect(page.locator("[data-composer-link-previews]")).toHaveCount(0);

  await waitForAnimations(page);
  await page.screenshot({
    animations: "disabled",
    path: `${SHOTS}/02-composer-entity-chip-only.png`,
  });

  await page.getByTestId("send-message").click();

  const row = page.getByTestId("message-row").last();
  await expect(row.locator('[data-link-preview^="buzz-"]')).toHaveCount(0);
  const repoChip = row.getByRole("button", {
    name: "Open repository relay-tools",
  });
  await expect(repoChip).toBeVisible();
  await repoChip.hover();
  await expect(
    page
      .getByRole("tooltip")
      .locator('[data-buzz-tooltip-metadata-content=""]'),
  ).toContainText("Operator tooling and admin CLI for relay deployments.");

  await waitForAnimations(page);
  await page.screenshot({
    animations: "disabled",
    path: `${SHOTS}/03-sent-entity-chip-tooltip.png`,
  });
});

// The composer and the renderer must classify a same-relay clone URL
// identically. Both read the active relay origin: without it the composer
// treats the URL as an external link, shows a loading standalone card, and
// enters sender-snapshot fetching — a preview the sent message then
// contradicts by rendering the same URL as a repository chip.
test("composer classifies a same-relay clone URL as a repository chip, not a card", async ({
  page,
}) => {
  // The default mock relay is http://localhost:3000, and generic external
  // previews are HTTPS-only — an http clone URL is unclassifiable either way,
  // so the mismatch cannot appear. Point the bridge at an https relay so the
  // composer's classification is the only variable.
  const relayHttpUrl = "https://relay.e2e.example";
  const cloneHref = `${relayHttpUrl}/git/${DEFAULT_MOCK_PUBKEY}/buzz.git`;
  await installBridge(page, {
    mode: "mock",
    relayHttpUrl,
    relayWsUrl: relayHttpUrl.replace(/^https/, "wss"),
    // Serve external metadata so a misclassified clone URL produces a
    // persistent standalone card rather than a card that loads and then
    // disappears on a null result — the assertions below would otherwise pass
    // by racing the loading window instead of by correct classification.
    mock: {
      linkPreviewMetadata: {
        title: "Misclassified as external",
        siteName: "relay.e2e.example",
        description: "Sender-snapshot metadata for an external link",
        imageDataUrl: null,
        imageDomain: null,
      },
    },
  });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();

  await page.getByTestId("message-input").fill(`Clone it: ${cloneHref}`);
  // Past the composer's 350ms resolution debounce: a same-relay clone URL must
  // never reach the standalone preview surface, loading card included.
  await expect(page.locator("[data-composer-link-previews]")).toHaveCount(0);
  await page.waitForTimeout(600);
  await expect(page.locator("[data-composer-link-previews]")).toHaveCount(0);

  await page.getByTestId("send-message").click();

  const row = page.getByTestId("message-row").last();
  await expect(row.locator("[data-link-preview]")).toHaveCount(0);
  const repoChip = row.getByRole("button", { name: "Open repository buzz" });
  await expect(repoChip).toBeVisible();
  await repoChip.hover();
  await expect(
    page
      .getByRole("tooltip")
      .locator('[data-buzz-tooltip-metadata-content=""]'),
  ).toContainText("Relay, desktop, and mobile clients");

  // The chip navigates in-app, proving the clone URL resolved onto the
  // canonical buzz://repo target rather than being handed to the OS.
  await repoChip.click();
  await expect(page.locator("[data-project-detail-screen]")).toBeVisible();
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
  await expect(page.getByTestId("project-channel-home")).toBeVisible();

  await emitEntityLink(repoLink);
  await expect(pullRequestsTab).toHaveAttribute("aria-selected", "true");

  const filesTab = page.getByRole("tab", { name: "Files", exact: true });
  await filesTab.click();
  await expect(filesTab).toHaveAttribute("aria-selected", "true");

  await emitEntityLink(repoLink);
  await expect(pullRequestsTab).toHaveAttribute("aria-selected", "true");

  await emitEntityLink(prLink);
  const prHeading = page
    .getByTestId("project-pull-request-detail")
    .getByRole("heading", { name: PR_SUBJECT });
  await expect(prHeading).toBeVisible();
  await breadcrumb.getByRole("button", { name: "Review", exact: true }).click();
  await expect(prHeading).toHaveCount(0);
  await emitEntityLink(prLink);
  await expect(prHeading).toBeVisible();

  await emitEntityLink(issueLink);
  const issueHeading = page
    .getByTestId("project-issue-detail")
    .getByRole("heading", { name: ISSUE_SUBJECT });
  await expect(issueHeading).toBeVisible();
  await breadcrumb.getByRole("button", { name: "Tasks", exact: true }).click();
  await expect(issueHeading).toHaveCount(0);
  await emitEntityLink(issueLink);
  await expect(issueHeading).toBeVisible();
});

test("deleted reply links identify deletion and fall back to their thread root", async ({
  page,
}) => {
  const deletedReplyId = "c".repeat(64);
  const threadRootId = "b".repeat(64);
  const channelId = "9dae0116-799b-5071-a0a8-fdd30a91a35d";
  const link = `buzz://message?channel=${channelId}&id=${deletedReplyId}&thread=${threadRootId}`;
  await installMockBridge(page, { deletedEventIds: [deletedReplyId] });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );
  await page.evaluate(
    ({ id }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "random",
        content: "Surviving thread root",
        id,
      });
    },
    { id: threadRootId },
  );
  await page.getByTestId("channel-general").click();
  await page.getByTestId("message-input").fill(`Deleted reply ${link}`);
  await page.getByTestId("send-message").click();

  const deletedLink = page
    .getByTestId("message-row")
    .filter({ hasText: "Deleted reply" })
    .last()
    .getByRole("button", {
      name: "Open thread in channel random; linked message was deleted",
    });
  await expect(deletedLink).toHaveAttribute(
    "data-message-link-state",
    "deleted",
  );
  await deletedLink.click();

  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(page).toHaveURL(new RegExp(`thread=${threadRootId}`));
  await expect(page.getByRole("heading", { name: "Thread" })).toBeVisible();
});

test("deleted top-level message links identify deletion and fall back to channel navigation", async ({
  page,
}) => {
  await page.addInitScript(() => {
    window.localStorage.setItem("buzz-theme", "buzz-dark");
    window.localStorage.setItem("buzz-accent-color", "#c0a2f1");
  });
  const missingMessageId = "d".repeat(64);
  const channelId = "9dae0116-799b-5071-a0a8-fdd30a91a35d";
  const link = `buzz://message?channel=${channelId}&id=${missingMessageId}`;
  await installMockBridge(page, { deletedEventIds: [missingMessageId] });
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await page.getByTestId("channel-general").click();
  await page.getByTestId("message-input").fill(`Missing link ${link}`);
  await page.getByTestId("send-message").click();

  const linkMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Missing link" })
    .last();
  const deletedLink = linkMessage.getByRole("button", {
    name: "Open channel random; linked message was deleted",
  });
  await expect(deletedLink).toHaveText("random");
  await expect(deletedLink).toHaveAttribute(
    "data-message-link-state",
    "deleted",
  );
  await expect(deletedLink).toHaveClass(/buzz-link-deleted/);
  await expect(deletedLink).not.toHaveClass(/buzz-link-unavailable/);
  const unavailableColors = await linkMessage.evaluate((message) => {
    const probe = document.createElement("span");
    probe.className = "mention-chip buzz-link-unavailable";
    message.querySelector(".message-markdown")?.append(probe);
    const styles = getComputedStyle(probe);
    const colors = {
      backgroundColor: styles.backgroundColor,
      color: styles.color,
    };
    probe.remove();
    return colors;
  });
  await expect
    .poll(() =>
      deletedLink.evaluate((element) => {
        const styles = getComputedStyle(element);
        return {
          backgroundColor: styles.backgroundColor,
          color: styles.color,
        };
      }),
    )
    .toEqual(unavailableColors);
  await waitForAnimations(page);
  await linkMessage.screenshot({
    path: `${SHOTS}/deleted-message-link-after.png`,
  });
  await deletedLink.hover();
  await expect(page.getByRole("tooltip")).toHaveText("Message deleted");

  await deletedLink.click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(page).toHaveURL(new RegExp(`#/channels/${channelId}$`));
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
