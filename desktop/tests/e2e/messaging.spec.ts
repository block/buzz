import { readFileSync } from "node:fs";

import { expect, test, type Locator } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { expectCornerRadiusPx, expectSmoothCorners } from "../helpers/css";
import { openSettings } from "../helpers/settings";

const LINK_PREVIEW_IMAGE = readFileSync(
  new URL("../fixtures/github-pr-5629-og.png", import.meta.url),
);
const LINK_PREVIEW_IMAGE_DATA_URL = `data:image/png;base64,${LINK_PREVIEW_IMAGE.toString("base64")}`;

async function waitForReadyComposerSnapshots(
  page: import("@playwright/test").Page,
  count = 1,
) {
  await expect(page.locator("[data-composer-link-previews]")).toHaveAttribute(
    "data-ready-snapshot-count",
    String(count),
  );
}

async function expectThreadReplyUnobscured(row: Locator) {
  await expect
    .poll(async () =>
      row.evaluate((element) => {
        const threadBody = element.closest(
          '[data-testid="message-thread-body"]',
        ) as HTMLElement | null;
        const threadPanel = element.closest(
          '[data-testid="message-thread-panel"]',
        ) as HTMLElement | null;
        const composer = threadPanel?.querySelector<HTMLElement>(
          '[data-testid="message-input"]',
        );
        if (!threadBody || !composer) return false;

        const rowRect = element.getBoundingClientRect();
        const bodyRect = threadBody.getBoundingClientRect();
        const composerRect = composer.getBoundingClientRect();
        const visibleBottom = Math.min(bodyRect.bottom, composerRect.top);
        return (
          rowRect.top >= bodyRect.top - 1 && rowRect.bottom <= visibleBottom + 1
        );
      }),
    )
    .toBe(true);
}

async function measureThreadSummaryGeometry(summaryRow: Locator) {
  return summaryRow.evaluate((summaryButton) => {
    const summaryWrapper = summaryButton.parentElement;
    const container = summaryWrapper?.parentElement;
    const messageRow = container?.querySelector<HTMLElement>(
      '[data-testid="message-row"]',
    );
    const messageMarkdown =
      messageRow?.querySelector<HTMLElement>(".message-markdown");
    const messageAuthor = messageRow?.querySelector<HTMLElement>(
      '[data-testid="message-author"]',
    );
    const firstParticipant = summaryButton.querySelector<HTMLElement>(
      '[data-testid="message-thread-summary-participant"]',
    );
    const summarySurface = summaryButton.querySelector<HTMLElement>(
      '[data-testid="message-thread-summary-surface"]',
    );
    const firstAvatar = firstParticipant?.firstElementChild;

    if (
      !summaryWrapper ||
      !container ||
      !messageRow ||
      !messageAuthor ||
      !messageMarkdown ||
      !summarySurface ||
      !(firstAvatar instanceof HTMLElement)
    ) {
      throw new Error("Expected measurable thread summary geometry.");
    }

    const containerRect = container.getBoundingClientRect();
    const messageRowRect = messageRow.getBoundingClientRect();
    const messageAuthorRect = messageAuthor.getBoundingClientRect();
    const messageMarkdownRect = messageMarkdown.getBoundingClientRect();
    const summaryButtonRect = summaryButton.getBoundingClientRect();
    const summaryButtonStyle = getComputedStyle(summaryButton);
    const summaryButtonPaddingLeft = Number.parseFloat(
      summaryButtonStyle.paddingLeft,
    );
    const summaryWrapperRect = summaryWrapper.getBoundingClientRect();
    const firstAvatarRect = firstAvatar.getBoundingClientRect();
    const summarySurfaceRect = summarySurface.getBoundingClientRect();

    return {
      authorLeft: messageAuthorRect.left,
      avatarLeft: firstAvatarRect.left,
      bodyLeft: messageMarkdownRect.left,
      bottomPadding: containerRect.bottom - summaryWrapperRect.bottom,
      messageRowLeft: messageRowRect.left,
      summaryButtonContentLeft:
        summaryButtonRect.left + summaryButtonPaddingLeft,
      summaryButtonLeft: summaryButtonRect.left,
      summaryButtonPaddingLeft,
      summarySurfaceLeft: summarySurfaceRect.left,
      topPadding: messageRowRect.top - containerRect.top,
    };
  });
}

test.beforeEach(async ({ page }, testInfo) => {
  const baseMock = testInfo.title.includes("agent owner label")
    ? {
        searchProfiles: [
          {
            pubkey: TEST_IDENTITIES.alice.pubkey,
            displayName: "alice",
            ownerPubkey: TEST_IDENTITIES.bob.pubkey,
            isAgent: true,
          },
          {
            pubkey: TEST_IDENTITIES.bob.pubkey,
            displayName: "bob",
          },
        ],
      }
    : testInfo.title.includes("cardless short tweet preview")
      ? {
          linkPreviewMetadata: {
            title: "jack (@jack) on X",
            siteName: "X (formerly Twitter)",
            description: "just setting up my twttr",
            imageDataUrl: null,
            imageDomain: null,
          },
        }
      : testInfo.title.includes("cardless tweet preview")
        ? {
            linkPreviewMetadata: {
              title: "Buzz (@buzz) on X",
              siteName: "X (formerly Twitter)",
              description:
                "This is a real tweet-style description long enough to wrap across several lines while preserving the message-like treatment. It keeps going with enough distinct words to exceed five rendered lines at the preview width, proving that the overflow-aware control appears only when the content is genuinely clipped rather than relying on a brittle character-count guess.",
              imageDataUrl:
                "data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='1200' height='675'%3E%3Crect width='1200' height='675' fill='%231d9bf0'/%3E%3C/svg%3E",
              imageDomain: "pbs.twimg.com",
            },
          }
        : testInfo.title.includes("fragment link previews")
          ? {
              // Metadata is keyed by the canonical, fragment-less URL — the
              // shape a real OpenGraph/HTML fetch resolves against. A resolver
              // that fetches with the raw `#fragment` attached would miss these
              // keys and drop the card, which is exactly the bug under test.
              linkPreviewMetadataByHref: {
                "https://github.com/block/buzz/pull/3767": {
                  title: "Buzz pull request 3767",
                  siteName: "GitHub",
                  description: "Fragment-bearing PR link.",
                  imageDataUrl: null,
                  imageDomain: null,
                },
                "https://github.com/block/buzz/pull/3867": {
                  title: "Buzz pull request 3867",
                  siteName: "GitHub",
                  description: "Plain PR link.",
                  imageDataUrl: null,
                  imageDomain: null,
                },
              },
            }
          : testInfo.title.includes("mixed link preview image outcomes")
            ? {
                linkPreviewMetadataByHref: {
                  "https://github.com/block/buzz/pull/4001": {
                    title: "Loaded preview image",
                    siteName: "GitHub",
                    description: "The image request completed.",
                    imageDataUrl:
                      "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
                    imageDomain: "opengraph.githubassets.com",
                    imageFetchState: "image",
                    imageRetryAfterMs: null,
                  },
                  "https://github.com/block/buzz/pull/4002": {
                    title: "Rate-limited preview image",
                    siteName: "GitHub",
                    description: "Metadata remains available during cooldown.",
                    imageDataUrl: null,
                    imageDomain: null,
                    imageFetchState: "transient_failure",
                    imageRetryAfterMs: 900_000,
                  },
                },
              }
            : testInfo.title.includes("link preview browser image error")
              ? {
                  linkPreviewMetadata: {
                    title: "Invalid decoded preview image",
                    siteName: "GitHub",
                    description: "The browser should replace this image.",
                    imageDataUrl: null,
                    imageDomain: null,
                    imageFetchState: "rejected",
                    imageRetryAfterMs: null,
                  },
                }
              : testInfo.title.includes("link preview image geometry")
                ? {
                    linkPreviewMetadata: {
                      title:
                        "Ship a wider horizontal preview with a two-line title that wraps cleanly",
                      siteName: "GitHub",
                      description:
                        "A polished, stable preview for shared links.",
                      imageDataUrl: LINK_PREVIEW_IMAGE_DATA_URL,
                      imageDomain: "opengraph.githubassets.com",
                      faviconDataUrl:
                        "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
                    },
                    linkPreviewMetadataDelayMs: 800,
                  }
                : testInfo.title.includes("link preview no-image layout") ||
                    testInfo.title.includes("composer no-image link embeds")
                  ? {
                      linkPreviewMetadata: {
                        title: "Buzz",
                        siteName: "GitHub",
                        description:
                          "Open-source collaboration for the Buzz app.",
                        imageDataUrl: null,
                        imageDomain: null,
                        faviconDataUrl:
                          "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
                      },
                      linkPreviewMetadataDelayMs: 2_000,
                    }
                  : testInfo.title.includes(
                        "rich link preview preserves description newlines",
                      )
                    ? {
                        linkPreviewMetadata: {
                          title: "Buzz pull request",
                          siteName: "GitHub",
                          description:
                            "First paragraph line one.\nFirst paragraph line two.\n\nSecond paragraph.",
                          imageDataUrl: null,
                          imageDomain: null,
                        },
                      }
                    : testInfo.title.includes(
                          "Enter during an in-flight snapshot upload",
                        ) ||
                        testInfo.title.includes("Skip wins the upload race") ||
                        testInfo.title.includes(
                          "async metadata beyond old cutoff",
                        ) ||
                        testInfo.title.includes(
                          "async upload beyond metadata budget",
                        ) ||
                        testInfo.title.includes(
                          "immediately pressing Enter prepares",
                        )
                      ? {
                          linkPreviewMetadata: {
                            title: "Buzz pull request",
                            siteName: "GitHub",
                            description: "A sender-authored preview snapshot.",
                            imageDataUrl:
                              "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
                            imageDomain: "opengraph.githubassets.com",
                          },
                          linkPreviewMetadataDelayMs: testInfo.title.includes(
                            "async metadata beyond old cutoff",
                          )
                            ? 4_000
                            : 300,
                          linkPreviewUploadDelayMs: testInfo.title.includes(
                            "async upload beyond metadata budget",
                          )
                            ? 4_000
                            : 1_200,
                        }
                      : testInfo.title.includes(
                            "snapshot thumbnail upload failure",
                          ) ||
                          testInfo.title.includes(
                            "snapshot media upload failure",
                          )
                        ? {
                            linkPreviewMetadata: {
                              title: "Buzz pull request",
                              siteName: "GitHub",
                              description:
                                "A sender-authored preview snapshot.",
                              imageDataUrl:
                                "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
                              imageDomain: "opengraph.githubassets.com",
                              faviconDataUrl:
                                "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
                            },
                            // Fail only the thumbnail upload; the favicon survives,
                            // so the snapshot degrades to a favicon-only preview.
                            linkPreviewUploadErrorFilenames: [
                              "link-preview-image",
                            ],
                          }
                        : testInfo.title.includes(
                              "sent link preview media uses",
                            )
                          ? {
                              mediaProxyInitiallyUnavailable: true,
                              linkPreviewMetadata: {
                                title: "Buzz pull request",
                                siteName: "GitHub",
                                description:
                                  "A sender-authored preview snapshot.",
                                imageDataUrl:
                                  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
                                imageDomain: "opengraph.githubassets.com",
                                faviconDataUrl:
                                  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=",
                              },
                            }
                          : testInfo.title.includes("link preview") ||
                              testInfo.title.includes("supported Compact")
                            ? {
                                linkPreviewMetadata: {
                                  title: "Buzz pull request",
                                  siteName: "GitHub",
                                  description:
                                    "A sender-authored preview snapshot.",
                                  imageDataUrl: null,
                                  imageDomain: null,
                                },
                                linkPreviewMetadataDelayMs:
                                  testInfo.title.includes(
                                    "loading card before cold resolver work",
                                  )
                                    ? 10_000
                                    : testInfo.title.includes(
                                          "explicit cancellation suppresses a pending",
                                        ) ||
                                        testInfo.title.includes(
                                          "settled-empty promoted link preview",
                                        )
                                      ? 10_000
                                      : testInfo.title.includes(
                                            "draft auto-send",
                                          )
                                        ? 500
                                        : testInfo.title.includes(
                                              "style defaults",
                                            ) ||
                                            testInfo.title.includes(
                                              "attachment-sized",
                                            )
                                          ? 1_500
                                          : undefined,
                                linkPreviewMetadataStartBlockMs:
                                  testInfo.title.includes(
                                    "loading card before cold resolver work",
                                  )
                                    ? 150
                                    : undefined,
                              }
                            : undefined;
  const mock = testInfo.title.includes("unresolvable preview")
    ? { linkPreviewMetadata: null, linkPreviewMetadataDelayMs: 800 }
    : {
        ...baseMock,
        ...(testInfo.title.includes("clears Sending after")
          ? { sendMessageDelayMs: 800 }
          : {}),
      };
  await installMockBridge(page, mock);
});

test("agent owner label identifies the agent and owner", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();

  const aliceMessage = page
    .getByTestId("message-row")
    .filter({ hasText: "Hey team — checking in." });
  const ownerTreatment = aliceMessage.getByTestId("message-agent-owner");

  await expect(ownerTreatment.locator("svg")).toBeVisible();
  await expect(
    ownerTreatment.getByText("managed by", { exact: true }),
  ).toBeVisible();
  await expect(ownerTreatment.locator(".font-semibold")).toHaveText("bob");
  await expect(ownerTreatment.getByRole("button")).toHaveAccessibleName("bob");
  await expect(ownerTreatment.locator(".sr-only")).toHaveText(
    "Agent managed by",
  );
});

test("send a message and see it in timeline", async ({ page }) => {
  const message = `Hello timeline ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("message-timeline")).toContainText(message);
  await expect(page.getByTestId("message-row").last()).toContainText(
    "npub1mock...",
  );
});

test("long autolink wraps without widening the timeline", async ({ page }) => {
  await page.setViewportSize({ width: 800, height: 600 });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const longUrl = `https://blocked.teams.cloudflare.com/?${"dependencyconfusionnpm".repeat(18)}`;
  const message = `Step "adapter" failed: npm error invalid json response body at <${longUrl}> reason: Unexpected token '<'`;

  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();

  const timeline = page.getByTestId("message-timeline");
  await expect(timeline).toContainText('Step "adapter" failed');
  await expect
    .poll(() =>
      timeline.evaluate((element) => element.scrollWidth - element.clientWidth),
    )
    .toBeLessThanOrEqual(1);

  const row = page.getByTestId("message-row").last();
  await row.hover();

  const actionBar = page.locator('[data-testid^="message-action-bar-"]').last();
  await expect(actionBar).toHaveCSS("opacity", "1");
  await expect
    .poll(async () => {
      const [barBox, timelineBox] = await Promise.all([
        actionBar.boundingBox(),
        timeline.boundingBox(),
      ]);
      if (!barBox || !timelineBox) {
        return Number.POSITIVE_INFINITY;
      }
      return barBox.x + barBox.width - (timelineBox.x + timelineBox.width);
    })
    .toBeLessThanOrEqual(0);
});

test("markdown tables overflow wide content and fill the message when narrow", async ({
  page,
}) => {
  await page.setViewportSize({ width: 900, height: 600 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.waitForFunction(
    () => typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function",
  );

  const longCell = "WIDE TABLE COLUMN VALUE ".repeat(8);
  await page.evaluate(
    ({ wide, narrow }) => {
      const createdAt = Math.floor(Date.now() / 1000);
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: wide,
        createdAt,
      });
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content: narrow,
        createdAt: createdAt + 1,
      });
    },
    {
      wide: `| ${longCell} | ${longCell} | ${longCell} |\n| --- | --- | --- |\n| ${longCell} | ${longCell} | ${longCell} |`,
      narrow: "| NARROW TABLE | VALUE |\n| --- | --- |\n| alpha | beta |",
    },
  );

  const wideTable = page
    .getByTestId("message-row")
    .filter({ hasText: "WIDE TABLE COLUMN VALUE" })
    .locator("[data-table-block]");
  const narrowTable = page
    .getByTestId("message-row")
    .filter({ hasText: "NARROW TABLE" })
    .locator("[data-table-block]");
  await expect(wideTable).toBeVisible();
  await expect(narrowTable).toBeVisible();

  await expect
    .poll(() =>
      wideTable.evaluate(
        (element) => element.scrollWidth - element.clientWidth,
      ),
    )
    .toBeGreaterThan(1);
  await expect
    .poll(() =>
      narrowTable.evaluate((element) => {
        const table = element.querySelector("table");
        return table
          ? Math.abs(table.getBoundingClientRect().width - element.clientWidth)
          : Number.POSITIVE_INFINITY;
      }),
    )
    .toBeLessThanOrEqual(1);
});

test("sent link preview media uses the authenticated proxy in compact and rich cards", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?proxy=1";
  const fallbackMediaPattern =
    /^buzz-media:\/\/localhost\/media\/[\da-f]{64}\.png$/;
  const proxyMediaPattern =
    /^http:\/\/127\.0\.0\.1:54321\/media\/[\da-f]{64}\.png$/;
  await page.route("http://127.0.0.1:54321/media/**", (route) =>
    route.fulfill({
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#22c55e"/></svg>',
      contentType: "image/svg+xml",
    }),
  );

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.getByTestId("message-input").fill(previewUrl);
  await waitForReadyComposerSnapshots(page);
  await page.getByTestId("send-message").click();

  const row = page.getByTestId("message-row").last();
  const compactPreview = row.locator(
    '[data-link-preview="github-pull-request"]',
  );
  const compactThumbnail = compactPreview
    .locator("[data-link-preview-thumbnail] img")
    .first();
  const compactFavicon = compactPreview.locator(
    "img[data-link-preview-hostname-favicon]",
  );
  await expect(compactThumbnail).toHaveAttribute("src", fallbackMediaPattern);
  await expect(compactFavicon).toHaveAttribute("src", fallbackMediaPattern);

  const releasedPort = await page.evaluate(() =>
    window.__BUZZ_E2E_RELEASE_MEDIA_PROXY__?.(),
  );
  expect(releasedPort).toBe(54321);
  await expect(compactThumbnail).toHaveAttribute("src", proxyMediaPattern);
  await expect(compactFavicon).toHaveAttribute("src", proxyMediaPattern);
  await expect
    .poll(() => compactThumbnail.evaluate((image) => image.naturalWidth))
    .toBe(40);

  // The compact thumbnail sits flush against the card shell's left edge, so the
  // shell's smooth-corner clip carves those corners. It must therefore carry
  // the same treatment itself, or its right corners render as a plain arc
  // against the shell's smoothed left corners. See the invariant in
  // `shared/ui/smoothCorners.ts`.
  const compactThumbnailFrame = compactPreview
    .locator("[data-link-preview-thumbnail]")
    .first();
  await expectCornerRadiusPx(compactPreview, 16);
  await expectCornerRadiusPx(compactThumbnailFrame, 16);
  await expectSmoothCorners(compactThumbnailFrame);

  await openSettings(page, "appearance");
  await page.getByTestId("link-preview-style-rich").click();
  await expect(page.getByTestId("link-preview-style-rich")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await page.getByTestId("settings-back-to-app").click();

  const richPreview = row.locator(
    '[data-link-preview="github-pull-request"][data-link-preview-inline]',
  );
  const richThumbnail = richPreview
    .locator("[data-link-preview-thumbnail] img")
    .first();
  const richFavicon = richPreview.locator("img[data-link-preview-favicon]");
  await expect(richThumbnail).toHaveAttribute("src", proxyMediaPattern);
  await expect(richFavicon).toHaveAttribute("src", proxyMediaPattern);
  await expect
    .poll(() => richThumbnail.evaluate((image) => image.naturalWidth))
    .toBe(40);
});

test("link preview style defaults to compact and Rich unfurls descriptions", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?inline=1";
  await page.setViewportSize({ width: 800, height: 900 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.getByTestId("message-input").fill(previewUrl);
  const composerPreview = page
    .locator("[data-composer-link-previews]")
    .locator('[data-link-preview="github-pull-request"]');
  await expect(composerPreview).toHaveAttribute("data-image-state", "pending");
  if (process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR) {
    await waitForAnimations(page);
    await page.screenshot({
      animations: "disabled",
      path: `${process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR}/compact-composer-loading.png`,
    });
  }
  await waitForReadyComposerSnapshots(page);
  await expect(composerPreview).toHaveAttribute("data-image-state", "none");
  if (process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR) {
    await waitForAnimations(page);
    await page.screenshot({
      animations: "disabled",
      path: `${process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR}/compact-composer-ready.png`,
    });
  }
  await page.getByTestId("send-message").click();

  const row = page.getByTestId("message-row").last();
  const compactPreview = row.locator(
    '[data-link-preview="github-pull-request"]',
  );
  await expect(compactPreview).toHaveCSS("border-top-left-radius", "0px");
  await expect(compactPreview).toHaveCSS("border-left-width", "3px");
  if (process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR) {
    await waitForAnimations(page);
    await page.screenshot({
      animations: "disabled",
      path: `${process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR}/recipient-compact.png`,
    });
  }

  await openSettings(page, "appearance");
  await expect(page.getByTestId("link-preview-style-compact")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await page.getByTestId("link-preview-style-rich").click();
  await expect(page.getByTestId("link-preview-style-rich")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await expect
    .poll(() =>
      page.evaluate(() =>
        localStorage.getItem("buzz.appearance.linkPreviewStyle"),
      ),
    )
    .toBe("rich");

  await page.getByTestId("settings-back-to-app").click();
  const richPreview = row.locator(
    '[data-link-preview="github-pull-request"][data-link-preview-inline]',
  );
  await expect(richPreview).toBeVisible();
  const richHostname = richPreview.locator("[data-link-preview-hostname]");
  await expect(richHostname).toHaveText("github.com");
  await expect(richHostname).toHaveAttribute("href", previewUrl);
  if (process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR) {
    await waitForAnimations(page);
    await page.screenshot({
      animations: "disabled",
      path: `${process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR}/recipient-rich.png`,
    });
    const richComposerUrl = `${previewUrl}&composer=rich`;
    await page.getByTestId("message-input").fill(richComposerUrl);
    const richComposerPreview = page
      .locator("[data-composer-link-previews]")
      .locator('[data-link-preview="github-pull-request"]');
    await expect(richComposerPreview).toHaveAttribute(
      "data-image-state",
      "pending",
    );
    await waitForAnimations(page);
    await page.screenshot({
      animations: "disabled",
      path: `${process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR}/rich-composer-loading.png`,
    });
    await waitForReadyComposerSnapshots(page);
    await expect(richComposerPreview).toHaveAttribute(
      "data-image-state",
      "none",
    );
    await waitForAnimations(page);
    await page.screenshot({
      animations: "disabled",
      path: `${process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR}/rich-composer-ready.png`,
    });
    await page.getByTestId("message-input").fill("");
  }

  await openSettings(page, "appearance");
  await page.getByTestId("link-preview-style-compact").click();
  await expect(page.getByTestId("link-preview-style-compact")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

for (const [pasteShape, wrapUrl] of [
  ["bare", (url: string) => url],
  ["angle-bracket", (url: string) => `<${url}>`],
] as const) {
  test(`${pasteShape} link preview paste paints before cold resolver work`, async ({
    page,
  }) => {
    const previewUrl = `https://github.com/block/buzz/pull/3246?paste=${pasteShape}`;
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    const input = page.getByTestId("message-input");
    await input.focus();

    const pasteText = wrapUrl(previewUrl);
    const firstPaint = await input.evaluate(async (element, pasteText) => {
      const clipboardData = new DataTransfer();
      clipboardData.setData("text/plain", pasteText);
      const startedAt = performance.now();
      element.dispatchEvent(
        new ClipboardEvent("paste", {
          bubbles: true,
          cancelable: true,
          clipboardData,
        }),
      );
      await new Promise<void>((resolve) =>
        requestAnimationFrame(() => resolve()),
      );
      return {
        elapsedMs: performance.now() - startedAt,
        resolverStarted: (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).some(
          (entry) => entry.command === "fetch_link_preview_metadata",
        ),
        text: element.textContent,
      };
    }, pasteText);
    expect(firstPaint.text).toContain(previewUrl);
    expect(firstPaint.resolverStarted).toBe(false);
    expect(firstPaint.elapsedMs).toBeLessThan(100);
    const composerPreview = page
      .locator("[data-composer-link-previews]")
      .locator('[data-link-preview="github-pull-request"]');
    await expect(input).toContainText(previewUrl, { timeout: 1_000 });
    await expect(
      composerPreview.locator('[data-slot="attachment"]'),
    ).toHaveAttribute("data-state", /^(processing|done)$/, { timeout: 1_000 });
    await expect(page.getByTestId("send-message")).toBeEnabled();
  });
}

test("display-text link preview produces and sends its preview", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?display=text";
  await page.goto("/");
  await page.getByTestId("channel-general").click();

  const input = page.getByTestId("message-input");
  await input.click();
  await input.pressSequentially("review the pull request");
  await page.keyboard.press("ControlOrMeta+a");
  await page.keyboard.press("ControlOrMeta+k");
  const dialog = page.getByRole("dialog", { name: "Add link" });
  await dialog.getByLabel("URL").fill(previewUrl);
  await dialog.getByRole("button", { name: "Save" }).click();

  await expect(input.locator(`a[href="${previewUrl}"]`)).toHaveText(
    "review the pull request",
  );
  await expect(page.locator("[data-composer-link-previews]")).toBeVisible();
  await waitForReadyComposerSnapshots(page);
  await page.getByTestId("send-message").click();

  const row = page.getByTestId("message-row").last();
  await expect(
    row.getByRole("link", { name: "review the pull request", exact: true }),
  ).toHaveAttribute("href", previewUrl);
  await expect(row.locator("[data-link-preview]")).toBeVisible();
  const linkPreviewTags = await page.evaluate(() => {
    const call = [...(window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])]
      .reverse()
      .find((entry) => entry.command === "send_channel_message");
    return (
      call?.payload as { linkPreviewTags?: string[][] | null } | undefined
    )?.linkPreviewTags;
  });
  expect(linkPreviewTags?.map((tag) => tag[3])).toEqual([previewUrl]);
});

test("rich link preview preserves description newlines after sending", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?newlines=1";
  await page.addInitScript(() =>
    localStorage.setItem("buzz.appearance.linkPreviewStyle", "rich"),
  );
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.getByTestId("message-input").fill(previewUrl);
  await waitForReadyComposerSnapshots(page);
  await page.getByTestId("send-message").click();

  const description = page
    .getByTestId("message-row")
    .last()
    .locator('[data-link-preview-inline] [data-slot="attachment-description"]');
  await expect(description.locator("p")).toHaveCount(2);
  await expect(description).toHaveText(
    "First paragraph line one.\nFirst paragraph line two.\n\nSecond paragraph.",
    { useInnerText: true },
  );
});

test("completed link previews normalize a trailing-fragment URL and still send", async ({
  page,
}) => {
  // The third URL carries a trailing `#` (empty fragment). It is normalized to
  // its fragmentless canonical form for the preview and snapshot tag, so it now
  // gets a card like the others; the message body keeps the original URL.
  const previewUrls = [
    "https://twitter.com/tellaho",
    "https://github.com/block/buzz/pull/3246",
    "https://x.com/tellaho/status/1884289176381841506#",
  ];
  const canonicalUrls = [
    "https://twitter.com/tellaho",
    "https://github.com/block/buzz/pull/3246",
    "https://x.com/tellaho/status/1884289176381841506",
  ];
  const pastedText = previewUrls.join("\n");
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.focus();
  await input.evaluate((element, text) => {
    const clipboardData = new DataTransfer();
    clipboardData.setData("text/plain", text);
    element.dispatchEvent(
      new ClipboardEvent("paste", {
        bubbles: true,
        cancelable: true,
        clipboardData,
      }),
    );
  }, pastedText);
  const composerPreviewCards = page.locator(
    "[data-link-preview-composer-card]",
  );
  await expect(composerPreviewCards).toHaveCount(3);
  await waitForReadyComposerSnapshots(page, 3);

  const send = page.getByTestId("send-message");
  await expect(send).toBeEnabled();
  await send.click();
  await expect(page.getByTestId("message-input")).toHaveText("");
  await expect(page.locator("[data-composer-link-previews]")).toHaveCount(0);
  await page.waitForTimeout(250);
  await expect(page.getByTestId("message-input")).toHaveText("");

  const calls = await page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
      (entry) => entry.command === "send_channel_message",
    ),
  );
  expect(calls).toHaveLength(1);
  expect(
    (
      calls[0]?.payload as { linkPreviewTags?: string[][] | null } | undefined
    )?.linkPreviewTags?.map((tag) => tag[3]),
  ).toEqual(canonicalUrls);
});

test("unresolvable preview disappears after the terminal miss", async ({
  page,
}) => {
  const previewUrl = "https://x.com/tellaho/status";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.getByTestId("message-input").fill(previewUrl);

  const composerPreviews = page.locator("[data-composer-link-previews]");
  await expect(composerPreviews).toBeVisible();
  await expect(
    composerPreviews.locator("[data-link-preview-composer-card]"),
  ).toHaveAttribute("data-image-state", "pending");
  await expect(composerPreviews).toHaveCount(0);

  await page.getByTestId("send-message").click();
  const row = page.getByTestId("message-row").last();
  await expect(row).toContainText(previewUrl);
  await expect(row.locator("[data-link-preview]")).toHaveCount(0);
});

test("explicit cancellation suppresses a pending link preview and sends without it", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?send=pending";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.getByTestId("message-input").fill(previewUrl);

  const composerPreviews = page.locator("[data-composer-link-previews]");
  const card = composerPreviews.locator(
    '[data-link-preview="github-pull-request"]',
  );
  const send = page.getByTestId("send-message");
  const cancel = card.getByTestId("composer-hide-link-previews");
  const progress = card.getByTestId("link-preview-progress");
  const textPlaceholder = card.getByTestId("link-preview-text-placeholder");
  await expect(composerPreviews).toHaveAttribute(
    "data-ready-snapshot-count",
    "0",
  );
  await expect(card).toHaveAttribute("data-image-state", "pending");
  await expect(cancel).toHaveAttribute(
    "aria-label",
    "Send without link previews",
  );
  await expect(cancel).toHaveAttribute("title", "Send without link previews");
  await expect(progress).toBeVisible();
  await expect(progress).not.toHaveAttribute("aria-valuenow");
  await expect(textPlaceholder).toBeVisible();
  await expect(card.locator("[data-link-preview-hostname]")).toHaveCount(0);
  await expect(card.getByText("github.com", { exact: true })).toHaveCount(0);
  await expect(cancel).toHaveCSS("opacity", "0");
  await card.hover();
  await expect(cancel).toHaveCSS("opacity", "1");
  await expect(card).toHaveCSS("overflow", "visible");
  await expect(card.locator('[data-slot="attachment"]')).toHaveCSS(
    "overflow",
    "hidden",
  );

  // The mock metadata takes ten seconds. Submit is intentionally available:
  // the pending work is promoted into the floating background preparation UI.
  await expect(send).toBeEnabled();
  await page.waitForTimeout(2_200);
  await expect(composerPreviews).toHaveAttribute(
    "data-has-pending-snapshots",
    "true",
  );
  await expect(card).toHaveAttribute("data-image-state", "pending");
  await expect(send).toBeEnabled();

  // The explicit escape suppresses all previews, preserves the draft link, and
  // makes the durable no-preview intent sendable immediately.
  await cancel.click();
  await expect(composerPreviews).toHaveCount(0);
  await expect(page.getByTestId("message-input")).toContainText(previewUrl);
  await expect(send).toBeEnabled();
  await send.click();

  const row = page.getByTestId("message-row").last();
  await expect(row).toContainText(previewUrl);
  await expect(row.locator("[data-link-preview]")).toHaveCount(0);

  const linkPreviewTags = await page.evaluate(() => {
    const call = [...(window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])]
      .reverse()
      .find((entry) => entry.command === "send_channel_message");
    return (
      call?.payload as { linkPreviewTags?: string[][] | null } | undefined
    )?.linkPreviewTags;
  });
  expect(linkPreviewTags).toEqual([["link-preview", "none"]]);
});

test("Enter during an in-flight snapshot upload hands off and sends once", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill(previewUrl);

  const card = page.locator("[data-link-preview-composer-card]");
  await expect(card).toHaveAttribute("data-image-state", "image");
  await expect(card).toHaveAttribute("data-snapshot-tag-ready", "false");

  await input.press("Enter");
  await expect(input).toHaveText("");
  const progress = page.getByTestId("composer-upload-progress");
  await expect(progress).toHaveAccessibleName("Preparing link preview");
  await expect(page.getByTestId("composer-upload-cancel")).toHaveText("Skip");

  const sendsDuringUpload = await page.evaluate(
    () =>
      (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
        (entry) => entry.command === "send_channel_message",
      ).length,
  );
  expect(sendsDuringUpload).toBe(0);

  const row = page.getByTestId("message-row").last();
  await expect(row).toContainText(previewUrl);
  await expect(row.locator("[data-link-preview]")).toBeVisible();
  await expect(progress).toHaveCount(0);
  const sends = await page.evaluate(
    () =>
      (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
        (entry) => entry.command === "send_channel_message",
      ).length,
  );
  expect(sends).toBe(1);
});

test("async metadata beyond old cutoff still produces preview image", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?slow=metadata";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill(previewUrl);
  await input.press("Enter");

  const progress = page.getByTestId("composer-upload-progress");
  await expect(progress).toHaveAccessibleName("Preparing link preview");
  await page.waitForTimeout(3_200);
  await expect(progress).toBeVisible();

  const row = page.getByTestId("message-row").last();
  await expect(row).toContainText(previewUrl);
  await expect(row.locator("[data-link-preview]")).toHaveAttribute(
    "data-image-state",
    "image",
  );
});

test("async upload beyond metadata budget retains preview image", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?slow=image";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill(previewUrl);
  await input.press("Enter");

  const progress = page.getByTestId("composer-upload-progress");
  await expect(progress).toHaveAccessibleName("Preparing link preview");
  await page.waitForTimeout(3_200);
  await expect(progress).toBeVisible();

  const row = page.getByTestId("message-row").last();
  await expect(row).toContainText(previewUrl);
  await expect(row.locator("[data-link-preview]")).toHaveAttribute(
    "data-image-state",
    "image",
  );
  const tags = await page.evaluate(() => {
    const call = [...(window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])]
      .reverse()
      .find((entry) => entry.command === "send_channel_message");
    return (call?.payload as { linkPreviewTags?: string[][] }).linkPreviewTags;
  });
  expect(tags?.[0]?.[7]).toContain("/media/");
  expect(tags?.[0]?.[8]).not.toBe("");
});

test("Skip wins the upload race and sends without preview", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?skip=1";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill(previewUrl);
  await expect(
    page.locator("[data-link-preview-composer-card]"),
  ).toHaveAttribute("data-snapshot-tag-ready", "false");

  await input.press("Enter");
  const skip = page.getByTestId("composer-upload-cancel");
  await expect(skip).toHaveText("Skip");
  await skip.click();

  const row = page.getByTestId("message-row").last();
  await expect(row).toContainText(previewUrl);
  await expect(row.locator("[data-link-preview]")).toHaveCount(0);
  await page.waitForTimeout(1_500);
  const calls = await page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
      (entry) => entry.command === "send_channel_message",
    ),
  );
  expect(calls).toHaveLength(1);
  expect(
    (calls[0]?.payload as { linkPreviewTags?: string[][] }).linkPreviewTags,
  ).toEqual([]);
});

test("promoted link preview send clears Sending after REST publication", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?pending=preview";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill(previewUrl);
  await input.press("Enter");

  const row = page.getByTestId("message-row").last();
  await expect(row).toContainText(previewUrl);
  await expect(row).toContainText("Sending…");
  await expect(row.locator("[data-link-preview]")).toBeVisible();
  await expect(row).not.toContainText("Sending…");

  const restCalls = await page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
      (entry) => entry.command === "send_channel_message",
    ),
  );
  expect(restCalls).toHaveLength(1);
  expect(
    (restCalls[0]?.payload as { linkPreviewTags?: string[][] }).linkPreviewTags,
  ).toHaveLength(1);
});

test("settled-empty promoted link preview send uses REST and clears Sending after Skip", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?pending=empty";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill(previewUrl);
  await input.press("Enter");
  await page.getByTestId("composer-upload-cancel").click();

  const row = page.getByTestId("message-row").last();
  await expect(row).toContainText(previewUrl);
  await expect(row).toContainText("Sending…");
  await expect(row).not.toContainText("Sending…");
  await expect(row.locator("[data-link-preview]")).toHaveCount(0);

  const result = await page.evaluate(() => ({
    restCalls: (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
      (entry) => entry.command === "send_channel_message",
    ),
    websocketSends: (window.__BUZZ_E2E_SIGNED_EVENTS__ ?? []).filter(
      (event) => event.kind === 9,
    ),
  }));
  expect(result.restCalls).toHaveLength(1);
  expect(
    (result.restCalls[0]?.payload as { linkPreviewTags?: string[][] })
      .linkPreviewTags,
  ).toEqual([]);
  expect(result.websocketSends).toHaveLength(0);
});

test("draft auto-send promotes link preview preparation and sends exactly once", async ({
  page,
}) => {
  // A confirmed Drafts-panel send must fire once immediately, promote preview
  // preparation into the background flow, and eventually publish one enriched
  // event rather than consuming the one-shot trigger while the hook debounces.
  const previewUrl = "https://github.com/block/buzz/pull/3246?draft=autosend";
  const channelId = "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50";

  // Seed a channel draft under the legacy store key (migrated on startup). The
  // main composer keys its draft off the bare channel id, and the Drafts panel
  // navigates with ?autoSend=<that key>, so seeding under the bare id mirrors
  // the real "Send message" target exactly.
  await page.addInitScript(
    ({ storeKey, draftKey, content, channel }) => {
      const timestamp = new Date().toISOString();
      window.localStorage.setItem(
        storeKey,
        JSON.stringify({
          [draftKey]: {
            channelId: channel,
            content,
            createdAt: timestamp,
            pendingImeta: [],
            selectionEnd: content.length,
            selectionStart: content.length,
            spoileredAttachmentUrls: [],
            status: "active",
            updatedAt: timestamp,
          },
        }),
      );
    },
    {
      storeKey: `buzz-drafts.v1:${"deadbeef".repeat(8)}`,
      draftKey: channelId,
      content: previewUrl,
      channel: channelId,
    },
  );

  // Drive the real Drafts-panel "Send message" confirm flow. This does an
  // in-app client navigation to the channel with ?autoSend=<draftKey>, arming
  // the main composer's auto-submit effect — the exact production path.
  await page.goto("/", { waitUntil: "domcontentloaded" });
  await expect(page.getByTestId("home-inbox")).toBeVisible({ timeout: 10_000 });
  await page.getByTestId("inbox-filter-trigger").click();
  await page.getByRole("menuitemradio", { name: "Drafts" }).click();
  await page.keyboard.press("Escape");

  const draftRow = page.locator(`[data-testid='home-draft-item-${channelId}']`);
  await expect(draftRow).toBeVisible({ timeout: 8_000 });
  await draftRow.hover();
  await draftRow
    .getByRole("button", { name: "Send message", exact: true })
    .click();
  const dialog = page.getByRole("alertdialog");
  await expect(dialog).toBeVisible({ timeout: 4_000 });
  await dialog.getByRole("button", { name: "Send", exact: true }).click();

  // Exactly one publish eventually fires and carries the promoted snapshot.
  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
            (entry) => entry.command === "send_channel_message",
          ).length,
      ),
    )
    .toBe(1);

  const linkPreviewTags = await page.evaluate(() => {
    const call = [...(window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])]
      .reverse()
      .find((entry) => entry.command === "send_channel_message");
    return (
      call?.payload as { linkPreviewTags?: string[][] | null } | undefined
    )?.linkPreviewTags;
  });
  expect(linkPreviewTags?.map((tag) => tag[3])).toEqual([previewUrl]);
});

test("rapid Enter presses on a ready link preview send exactly once", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?rapid=1";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill(previewUrl);

  // Wait until the snapshot is fully ready and Send is enabled, so the only
  // thing under test is the composer-local send lock — not preview settling.
  await waitForReadyComposerSnapshots(page);
  await expect(page.getByTestId("send-message")).toBeEnabled();

  // Mash Enter. The synchronous submit lock (isSubmitLockedRef), acquired before
  // any await, must collapse these into exactly one send_channel_message so a
  // duplicate cannot clear shared prep/hydration state mid-send.
  await input.press("Enter");
  await input.press("Enter");
  await input.press("Enter");

  const row = page.getByTestId("message-row").last();
  await expect(row).toContainText(previewUrl);
  await expect(row.locator("[data-link-preview]")).toBeVisible();

  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
            (entry) => entry.command === "send_channel_message",
          ).length,
      ),
    )
    .toBe(1);
});

test("pasting a link and immediately pressing Enter prepares it after submit", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?fast=send";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");

  await input.fill(previewUrl);
  await input.press("Enter");
  await expect(input).toHaveText("");
  await expect(page.getByTestId("composer-upload-cancel")).toHaveText("Skip");

  const row = page.getByTestId("message-row").last();
  await expect(row).toContainText(previewUrl);
  await expect(row.locator("[data-link-preview]")).toBeVisible();
  const calls = await page.evaluate(() =>
    (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
      (entry) => entry.command === "send_channel_message",
    ),
  );
  expect(calls).toHaveLength(1);
});

test("a snapshot media upload failure preserves a metadata-only preview", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?upload=fail";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");
  await input.fill(previewUrl);
  await input.press("Enter");

  const row = page.getByTestId("message-row").last();
  await expect(row).toContainText(previewUrl);
  await expect(row.locator("[data-link-preview]")).toBeVisible();
  const tags = await page.evaluate(() => {
    const call = [...(window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])]
      .reverse()
      .find((entry) => entry.command === "send_channel_message");
    return (call?.payload as { linkPreviewTags?: string[][] }).linkPreviewTags;
  });
  expect(tags).toHaveLength(1);
  expect(tags?.[0]?.slice(0, 7)).toEqual([
    "link-preview",
    "snapshot",
    "1",
    previewUrl,
    "Buzz pull request",
    "GitHub",
    "A sender-authored preview snapshot.",
  ]);
  expect(tags?.[0]?.slice(7)).toEqual(["", "", "", ""]);
});

test("editing a message excludes link previews entirely", async ({ page }) => {
  const message = `Edit-me ${Date.now()}`;
  const previewUrl = "https://github.com/block/buzz/pull/3246?edit=1";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  const input = page.getByTestId("message-input");

  // Send a plain message with no link, then edit it to add a supported URL.
  await input.fill(message);
  await input.press("Enter");
  await expect(page.getByTestId("message-timeline")).toContainText(message);

  await expect(input).toBeFocused();
  await page.keyboard.press("ArrowUp");
  await expect(page.getByTestId("edit-target")).toBeVisible();

  // Adding a link while editing must NOT resolve, upload, gate Save, or render a
  // composer preview card — edit mode does not persist snapshots (decision A).
  await input.fill(`${message} ${previewUrl}`);
  await expect(page.locator("[data-composer-link-previews]")).toHaveCount(0);
  // No snapshot upload was attempted for the edited link.
  const uploadedPreviewMedia = await page.evaluate(
    () =>
      (window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? []).filter(
        (entry) =>
          entry.command === "upload_media_bytes" &&
          typeof (entry.payload as { filename?: string })?.filename ===
            "string" &&
          (entry.payload as { filename: string }).filename.startsWith(
            "link-preview-",
          ),
      ).length,
  );
  expect(uploadedPreviewMedia).toBe(0);
  // Save is not blocked waiting on a snapshot the edit will never emit.
  await expect(page.getByTestId("send-message")).toBeEnabled();
});

test("hiding composer link previews suppresses the whole draft and emits the blanket marker", async ({
  page,
}) => {
  const firstUrl = "https://github.com/block/buzz/pull/3246?hide=all";
  const secondUrl = "https://linear.app/acme/issue/ABC-123/hidden-too";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.getByTestId("message-input").fill(firstUrl);
  await expect(page.locator("[data-composer-link-previews]")).toBeVisible();
  await waitForReadyComposerSnapshots(page);

  await page.getByTestId("composer-hide-link-previews").click();
  await expect(page.locator("[data-composer-link-previews]")).toHaveCount(0);
  await page.getByTestId("message-input").fill(`${firstUrl} ${secondUrl}`);
  await expect(page.locator("[data-composer-link-previews]")).toHaveCount(0);
  await page.getByTestId("send-message").click();

  const row = page.getByTestId("message-row").last();
  await expect(row).toContainText(firstUrl);
  await expect(row).toContainText(secondUrl);
  await expect(row.locator("[data-link-preview]")).toHaveCount(0);
  const linkPreviewTags = await page.evaluate(() => {
    const call = [...(window.__BUZZ_E2E_COMMAND_PAYLOADS__ ?? [])]
      .reverse()
      .find((entry) => entry.command === "send_channel_message");
    return (
      call?.payload as { linkPreviewTags?: string[][] | null } | undefined
    )?.linkPreviewTags;
  });
  expect(linkPreviewTags).toEqual([["link-preview", "none"]]);

  await page.getByTestId("message-input").fill(firstUrl);
  await expect(page.locator("[data-composer-link-previews]")).toBeVisible();
});

test("composer link preview embeds stay attachment-sized while loading and ready", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246";

  for (const width of [800, 420]) {
    await page.setViewportSize({ width: 800, height: 700 });
    await page.goto("/");
    await page.getByTestId("channel-general").click();
    await page.setViewportSize({ width, height: 700 });
    await page
      .getByTestId("message-input")
      .fill(`${previewUrl}?viewport=${width}`);
    const card = page
      .locator("[data-composer-link-previews]")
      .locator('[data-link-preview="github-pull-request"]');
    await expect(card).toBeVisible();
    const initial = await card.evaluate((element) => ({
      height: element.getBoundingClientRect().height,
      width: element.getBoundingClientRect().width,
      thumbnailHeight: element
        .querySelector("[data-link-preview-thumbnail]")
        ?.getBoundingClientRect().height,
      thumbnailWidth: element
        .querySelector("[data-link-preview-thumbnail]")
        ?.getBoundingClientRect().width,
    }));
    if (process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR) {
      await waitForAnimations(page);
      await page.screenshot({
        animations: "disabled",
        path: `${process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR}/composer-${width}-loading.png`,
      });
    }

    await expect(card.locator('[data-slot="attachment"]')).toHaveAttribute(
      "data-state",
      "done",
    );
    const ready = await card.evaluate((element) => ({
      height: element.getBoundingClientRect().height,
      width: element.getBoundingClientRect().width,
      thumbnailHeight: element
        .querySelector("[data-link-preview-thumbnail]")
        ?.getBoundingClientRect().height,
      thumbnailWidth: element
        .querySelector("[data-link-preview-thumbnail]")
        ?.getBoundingClientRect().width,
    }));
    if (process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR) {
      await waitForAnimations(page);
      await page.screenshot({
        animations: "disabled",
        path: `${process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR}/composer-${width}-ready.png`,
      });
    }

    expect(initial.height).toBe(55);
    expect(initial.width).toBe(320);
    expect(initial.thumbnailHeight).toBe(55);
    expect(initial.thumbnailWidth).toBe(55);
    expect(ready).toEqual(initial);
  }
});

test("compact link preview image geometry truncates long titles to one line", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?geometry=1";
  // Sent snapshot media is relay-hosted and rewritten through the
  // authenticated media proxy (#5627), so serve the fixture from the mock
  // proxy origin rather than the raw relay origin.
  await page.route("http://127.0.0.1:54321/media/**", (route) =>
    route.fulfill({
      body: LINK_PREVIEW_IMAGE,
      contentType: "image/png",
    }),
  );
  await page.setViewportSize({ width: 800, height: 700 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.getByTestId("message-input").fill(previewUrl);
  await waitForReadyComposerSnapshots(page);
  await page.getByTestId("send-message").click();

  const row = page.getByTestId("message-row").last();
  const card = row.locator('[data-link-preview="github-pull-request"]');
  const thumbnail = card.locator("[data-link-preview-thumbnail]");
  const title = card.locator('[data-slot="attachment-title"]');
  const image = thumbnail.locator("img");
  await expect(card).toHaveAttribute("data-image-state", "image");
  await expect(image).toHaveJSProperty("complete", true);
  await expect(card).toHaveCSS("height", "64px");
  await expect(thumbnail).toHaveCSS("height", "64px");
  await expect(thumbnail).toHaveCSS("width", "104px");
  await expect(title).toHaveText(
    "Ship a wider horizontal preview with a two-line title that wraps cleanly",
  );
  await expect(title).toHaveCSS("white-space", "nowrap");
  await expect
    .poll(() =>
      title.evaluate((element) => element.scrollWidth - element.clientWidth),
    )
    .toBeGreaterThan(1);

  if (process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR) {
    await waitForAnimations(page);
    await row.screenshot({
      animations: "disabled",
      path: `${process.env.BUZZ_LINK_PREVIEW_SCREENSHOTS_DIR}/recipient-compact-long-title.png`,
    });
  }
});

test("composer no-image link embeds keep the attachment footprint", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/buzz/pull/3246?inline=none";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page.getByTestId("message-input").fill(previewUrl);
  const card = page
    .locator("[data-composer-link-previews]")
    .locator('[data-link-preview="github-pull-request"]');
  await expect(card).toHaveAttribute("data-image-state", "none");
  await expect(card.locator("[data-link-preview-thumbnail]")).toBeVisible();
  await expect(card.locator('[data-slot="attachment-title"]')).toContainText(
    /github\.com|Buzz/,
  );
  await expect(card).toHaveCSS("height", "55px");
});

test("mixed link preview image outcomes keep Compact and Rich fallbacks stable", async ({
  page,
}) => {
  const loadedUrl = "https://github.com/block/buzz/pull/4001";
  const rateLimitedUrl = "https://github.com/block/buzz/pull/4002";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page
    .getByTestId("message-input")
    .fill(`${loadedUrl}\n${rateLimitedUrl}`);
  await waitForReadyComposerSnapshots(page, 2);
  await page.getByTestId("send-message").click();

  const row = page.getByTestId("message-row").last();
  const compactCards = row.locator('[data-link-preview="github-pull-request"]');
  await expect(compactCards).toHaveCount(2);
  await expect(compactCards.nth(0)).toHaveAttribute(
    "data-image-state",
    "image",
  );
  await expect(
    compactCards.nth(0).locator("[data-link-preview-thumbnail]"),
  ).toBeVisible();
  await expect(compactCards.nth(1)).toHaveAttribute("data-image-state", "none");
  await expect(
    compactCards.nth(1).locator("[data-link-preview-image-fallback]"),
  ).toHaveCount(0);

  await openSettings(page, "appearance");
  await page.getByTestId("link-preview-style-rich").click();
  await expect(page.getByTestId("link-preview-style-rich")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await page.getByTestId("settings-back-to-app").click();

  const richCards = row.locator(
    '[data-link-preview="github-pull-request"][data-link-preview-inline]',
  );
  await expect(richCards).toHaveCount(2);
  await expect(
    richCards.nth(1).locator("[data-link-preview-image-fallback]"),
  ).toHaveCount(0);
});

test("fragment link previews render a card per canonical URL", async ({
  page,
}) => {
  // Two links into the SAME page differing only by `#fragment`, plus a link
  // to a second page. The fragment variants collapse to one card (the preview
  // is of the page, not the anchor); the second page adds a second card — two
  // cards total. A resolver that keys previews on the raw fragment-bearing URL
  // drops the fragment cards entirely (the reported bug).
  const fragmentUrlA =
    "https://github.com/block/buzz/pull/3767#pullrequestreview-4857569498";
  const fragmentUrlB = "https://github.com/block/buzz/pull/3767#issuecomment-1";
  const plainUrl = "https://github.com/block/buzz/pull/3867";
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page
    .getByTestId("message-input")
    .fill(`${fragmentUrlA}\n${fragmentUrlB}\n${plainUrl}`);

  const composerCards = page
    .locator("[data-composer-link-previews]")
    .locator('[data-link-preview="github-pull-request"]');
  await expect(composerCards).toHaveCount(2);

  await waitForReadyComposerSnapshots(page, 2);
  await page.getByTestId("send-message").click();

  const row = page.getByTestId("message-row").last();
  // Two preview cards: the fragment variants collapsed to the 3767 page, plus
  // the 3867 page.
  await expect(
    row.locator('[data-link-preview="github-pull-request"]'),
  ).toHaveCount(2);
  // Both original fragment-bearing prose links survive intact and clickable —
  // the fragment is a navigation anchor, only the preview is normalized.
  await expect(row.locator(`a[href="${fragmentUrlA}"]`)).toBeVisible();
  await expect(row.locator(`a[href="${fragmentUrlB}"]`)).toBeVisible();
});

test("link preview browser image errors render a fallback", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await page
    .getByTestId("message-input")
    .fill("https://github.com/block/buzz/pull/4003");
  await waitForReadyComposerSnapshots(page);
  await page.getByTestId("send-message").click();

  const row = page.getByTestId("message-row").last();
  const compactCard = row.locator('[data-link-preview="github-pull-request"]');
  await expect(compactCard).toHaveAttribute("data-image-state", "none");
  await expect(
    compactCard.locator("[data-link-preview-image-fallback]"),
  ).toHaveCount(0);

  await openSettings(page, "appearance");
  await page.getByTestId("link-preview-style-rich").click();
  await expect(page.getByTestId("link-preview-style-rich")).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await page.getByTestId("settings-back-to-app").click();

  const richCard = row.locator(
    '[data-link-preview="github-pull-request"][data-link-preview-inline]',
  );
  await expect(
    richCard.locator("[data-link-preview-image-fallback]"),
  ).toHaveCount(0);
});

test("supported Compact link previews keep the message link visible with square outer corners", async ({
  page,
}) => {
  const previewUrl = "https://github.com/block/sprout/pull/1334";

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill(previewUrl);
  await waitForReadyComposerSnapshots(page);
  await page.getByTestId("send-message").click();

  const row = page.getByTestId("message-row").last();
  await expect(
    row.getByRole("link", { exact: true, name: previewUrl }),
  ).toBeVisible();
  const previewCard = row.locator('[data-link-preview="github-pull-request"]');
  await expect(previewCard).toBeVisible();
  await expectCornerRadiusPx(previewCard, 0);
});

test("send multiple messages in sequence", async ({ page }) => {
  const ts = Date.now();
  const messages = [
    `First message ${ts}`,
    `Second message ${ts}`,
    `Third message ${ts}`,
  ];
  const input = page.getByTestId("message-input");
  const sendButton = page.getByTestId("send-message");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  for (const message of messages) {
    await input.fill(message);
    await sendButton.click();
    await expect(page.getByTestId("message-timeline")).toContainText(message);
  }

  const timeline = page.getByTestId("message-timeline");
  for (const message of messages) {
    await expect(timeline).toContainText(message);
  }
});

test("copy a rendered code block and paste it back as code", async ({
  page,
}) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: "http://127.0.0.1:4173",
  });

  const code = "# not a heading\nconst answer = 42;\n  indented();";

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await page.evaluate(
    (text) => navigator.clipboard.writeText(text),
    `\`\`\`ts\n${code}\n\`\`\``,
  );
  await input.click();
  await page.keyboard.press("ControlOrMeta+V");
  await page.getByTestId("send-message").click();

  const codeBlock = page.locator("[data-code-block]");
  await expect(codeBlock).toHaveCount(1);
  await expectCornerRadiusPx(codeBlock.locator("pre"), 16);
  await expectSmoothCorners(codeBlock.locator("pre"));

  const copyButton = page.getByLabel("Copy code block");
  await expect(copyButton).toHaveCSS("opacity", "0");
  await codeBlock.hover();
  await expect(copyButton).toHaveCSS("opacity", "1");
  await copyButton.click();
  await expect
    .poll(() => page.evaluate(() => navigator.clipboard.readText()))
    .toBe(code);

  await input.click();
  await page.keyboard.press("ControlOrMeta+V");
  await input.press("Enter");

  await expect(codeBlock).toHaveCount(2);
});

test("pasting a long copied code block scrolls composer to cursor", async ({
  page,
}) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: "http://127.0.0.1:4173",
  });

  const longCode = Array.from(
    { length: 48 },
    (_, index) => `const line${index} = ${index};`,
  ).join("\n");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await page.evaluate(
    (text) => navigator.clipboard.writeText(text),
    `\`\`\`ts\n${longCode}\n\`\`\``,
  );
  await input.click();
  await page.keyboard.press("ControlOrMeta+V");
  await page.getByTestId("send-message").click();

  const copiedCodeBlock = page.locator("[data-code-block]");
  await expect(copiedCodeBlock).toHaveCount(1);
  await copiedCodeBlock.hover();
  await page.getByLabel("Copy code block").click();

  await input.fill("typed before paste");
  await page.keyboard.press("ControlOrMeta+V");

  const scrollContainer = page.getByTestId("message-input-scroll");
  await expect
    .poll(() =>
      scrollContainer.evaluate(
        (element) =>
          element.scrollHeight - element.clientHeight - element.scrollTop,
      ),
    )
    .toBeLessThanOrEqual(1);
});

test("code block shows language label when language is specified", async ({
  page,
}) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"], {
    origin: "http://127.0.0.1:4173",
  });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await page.evaluate(
    (text) => navigator.clipboard.writeText(text),
    "```typescript\nconst x = 1;\n```",
  );

  await input.click();
  await page.keyboard.press("ControlOrMeta+V");
  await page.getByTestId("send-message").click();

  const codeBlock = page.locator("[data-code-block]");
  await expect(codeBlock).toBeVisible();
  await expect(codeBlock.getByText("typescript")).toBeVisible();
});

test("typing triple backticks and Enter creates a code block in composer", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.click();
  await page.keyboard.type("```");
  await page.keyboard.press("Enter");

  // A <pre> code block should appear inside the ProseMirror editor
  const editorPre = input.locator("pre");
  await expect(editorPre).toBeVisible();

  // The literal backticks should be consumed (not visible as text)
  await expect(input).not.toContainText("```");
});

test("message input clears after send", async ({ page }) => {
  const message = `Clear after send ${Date.now()}`;
  const input = page.getByTestId("message-input");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await input.fill(message);
  await expect(input).toHaveText(message);
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("message-timeline")).toContainText(message);
  await expect(input).toHaveText("");
});

test("emoji picker inserts emoji into the draft and keeps focus in the composer", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const input = page.getByTestId("message-input");
  await input.fill("Ship");

  await page.getByTestId("composer-emoji-button").click();

  // emoji-mart renders inside a Shadow DOM web component — use the search
  // input to find the rocket emoji, then click it.
  const pickerEl = page.locator("em-emoji-picker");
  const searchInput = pickerEl.locator("input[type='search']");
  await searchInput.fill("rocket");
  await pickerEl.locator("button[aria-label='🚀']").first().click();

  await expect(input).toHaveText("Ship🚀");
  await expect(input).toBeFocused();

  await input.pressSequentially(" now");
  await expect(input).toHaveText("Ship🚀 now");
});

test("empty message cannot be sent", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const sendButton = page.getByTestId("send-message");
  await expect(sendButton).toBeDisabled();
});

test("send message with Enter key", async ({ page }) => {
  const message = `Enter key send ${Date.now()}`;
  const input = page.getByTestId("message-input");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await input.fill(message);
  await input.press("Enter");

  await expect(page.getByTestId("message-timeline")).toContainText(message);
});

test("messages persist across channel switches", async ({ page }) => {
  const message = `Persist across switch ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(message);

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-timeline")).toContainText(message);
});

test("draft is preserved when switching channels", async ({ page }) => {
  const draft = `Unsent draft ${Date.now()}`;
  const input = page.getByTestId("message-input");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Type a draft but do not send it
  await input.fill(draft);
  await expect(input).toHaveText(draft);

  // Switch to another channel — composer should be empty
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(input).toHaveText("");

  // Switch back — the draft should still be there
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(input).toHaveText(draft);
});

test("sending a message clears the draft", async ({ page }) => {
  const message = `Sent message ${Date.now()}`;
  const input = page.getByTestId("message-input");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Type and send a message
  await input.fill(message);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(message);

  // Switch away and back — composer should be empty, not restored from draft
  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(input).toHaveText("");
});

test("different channels have independent messages", async ({ page }) => {
  const ts = Date.now();
  const generalMessage = `General only ${ts}`;
  const randomMessage = `Random only ${ts}`;

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.getByTestId("message-input").fill(generalMessage);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(
    generalMessage,
  );

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");
  await expect(page.getByTestId("message-timeline")).not.toContainText(
    generalMessage,
  );

  await page.getByTestId("message-input").fill(randomMessage);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(
    randomMessage,
  );

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-timeline")).toContainText(
    generalMessage,
  );
  await expect(page.getByTestId("message-timeline")).not.toContainText(
    randomMessage,
  );
});

test("day divider appears in timeline", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await expect(page.getByTestId("message-timeline")).toContainText(
    "Welcome to general",
  );
  await expect(page.getByTestId("message-timeline-day-divider")).toBeVisible();
});

test("send message to DM channel p-tags the recipient", async ({ page }) => {
  const message = `DM message ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("channel-alice-tyler").click();
  await expect(page.getByTestId("chat-title")).toHaveText("alice-tyler");

  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();

  await expect(page.getByTestId("message-timeline")).toContainText(message);
  await expect
    .poll(() =>
      page.evaluate((content) => {
        const events = (
          window as Window & {
            __BUZZ_E2E_SIGNED_EVENTS__?: Array<{
              content: string;
              tags: string[][];
            }>;
          }
        ).__BUZZ_E2E_SIGNED_EVENTS__;
        return events?.find((event) => event.content === content)?.tags ?? [];
      }, message),
    )
    .toContainEqual(["p", TEST_IDENTITIES.alice.pubkey]);
});

test("sends a thread message to its parent channel with a root-thread link", async ({
  page,
}) => {
  const timestamp = Date.now();
  const rootContent = `🧵 Share source thread ${timestamp}`;
  const priorChannelMessage = `Prior channel message ${timestamp}`;
  const replySummary = `Share this reply ${timestamp}`;
  const attachmentSha = "d".repeat(64);
  const attachmentUrl = `http://localhost:3000/media/${attachmentSha}.txt`;
  const customEmojiUrl = "https://example.com/send-to-channel-party.svg";
  const previewUrl = "https://github.com/block/buzz/pull/5305";
  const ownReplyContent = [
    `${replySummary} with @alice :party:`,
    `[launch-notes.txt](${attachmentUrl})`,
    previewUrl,
  ].join("\n\n");
  const imetaTag = [
    "imeta",
    `url ${attachmentUrl}`,
    "m text/plain",
    `x ${attachmentSha}`,
    "size 42",
    "filename launch-notes.txt",
  ];
  const emojiTag = ["emoji", "party", customEmojiUrl];
  const mentionTag = ["mention", TEST_IDENTITIES.alice.pubkey];
  const linkPreviewTag = [
    "link-preview",
    "snapshot",
    "1",
    previewUrl,
    "Add Send to channel for thread messages",
    "GitHub",
    "A shared link preview preserved from the source thread message.",
    "",
    "",
    "",
    "",
  ];

  await page.route(customEmojiUrl, (route) =>
    route.fulfill({
      body: '<svg xmlns="http://www.w3.org/2000/svg" width="32" height="32"><circle cx="16" cy="16" r="15" fill="#a78bfa"/><path d="M8 18l5 5 11-13" fill="none" stroke="white" stroke-width="3"/></svg>',
      contentType: "image/svg+xml",
    }),
  );

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await page.waitForFunction(
    () =>
      typeof window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__ === "function" &&
      (window.__BUZZ_E2E_HAS_MOCK_LIVE_SUBSCRIPTION__?.({
        channelName: "general",
      }) ??
        false),
  );

  const { ownReplyId, rootId } = await page.evaluate(
    ({ alicePubkey, ownReply, root, semanticTags }) => {
      const emit = window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__;
      if (!emit) throw new Error("Mock message emitter is unavailable.");
      const rootEvent = emit({
        channelName: "general",
        content: root,
        pubkey: alicePubkey,
      });
      const ownReplyEvent = emit({
        channelName: "general",
        content: ownReply,
        extraTags: semanticTags,
        mentionPubkeys: [alicePubkey],
        parentEventId: rootEvent.id,
      });
      return {
        ownReplyId: ownReplyEvent.id,
        rootId: rootEvent.id,
      };
    },
    {
      alicePubkey: TEST_IDENTITIES.alice.pubkey,
      ownReply: ownReplyContent,
      root: rootContent,
      semanticTags: [imetaTag, emojiTag, mentionTag, linkPreviewTag],
    },
  );

  const timeline = page.getByTestId("message-timeline");
  const rootRow = timeline.locator(`[data-message-id="${rootId}"]`);
  await expect(rootRow).toContainText(rootContent);

  await page.getByTestId("message-input").fill(priorChannelMessage);
  await page.getByTestId("send-message").click();
  const priorChannelRow = timeline
    .getByTestId("message-row")
    .filter({ hasText: priorChannelMessage });
  await expect(priorChannelRow).toBeVisible();
  await expect(priorChannelRow.getByTestId("message-send-status")).toHaveCount(
    0,
  );

  await timeline
    .locator(
      `[data-testid="message-thread-summary"][data-thread-head-id="${rootId}"]`,
    )
    .click();
  const threadPanel = page.getByTestId("message-thread-panel");
  const threadRootRow = threadPanel.locator(`[data-message-id="${rootId}"]`);
  const rootMoreActions = threadRootRow.getByTestId(`more-actions-${rootId}`);
  await rootMoreActions.click({ force: true });
  await expect(page.getByRole("menu")).toBeVisible();
  await expect(
    page.getByRole("menuitem", { name: "Send to channel" }),
  ).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(page.getByRole("menu")).toHaveCount(0);

  const ownReplyRow = threadPanel.locator(`[data-message-id="${ownReplyId}"]`);
  await expect(ownReplyRow).toContainText(replySummary);
  await ownReplyRow
    .getByTestId(`more-actions-${ownReplyId}`)
    .click({ force: true });
  const sendToChannelItem = page.getByRole("menuitem", {
    name: "Send to channel",
  });
  const sendToChannelIcon = sendToChannelItem.getByTestId(
    "send-to-channel-icon",
  );
  await expect(sendToChannelIcon).toBeVisible();
  await expect(sendToChannelIcon).toHaveAttribute("aria-hidden", "true");
  await expect(sendToChannelIcon).toHaveClass(/lucide-hash-arrow-in/);
  await expect
    .poll(async () => {
      const box = await sendToChannelIcon.boundingBox();
      return box ? [box.width, box.height] : null;
    })
    .toEqual([16, 16]);
  await sendToChannelItem.click();

  await expect(
    page.locator("[data-sonner-toast]").filter({ hasText: "Sent to channel" }),
  ).toBeVisible();
  await expect
    .poll(() =>
      page.evaluate((content) => {
        return Boolean(
          (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).findLast(
            (entry) =>
              entry.command === "send_channel_message" &&
              (entry.payload as { content?: string } | undefined)?.content ===
                content,
          ),
        );
      }, ownReplyContent),
    )
    .toBe(true);
  const sentPayload = await page.evaluate(
    (content) =>
      (window.__BUZZ_E2E_COMMAND_LOG__ ?? []).findLast(
        (entry) =>
          entry.command === "send_channel_message" &&
          (entry.payload as { content?: string } | undefined)?.content ===
            content,
      )?.payload as Record<string, unknown> | undefined,
    ownReplyContent,
  );
  expect(sentPayload).toMatchObject({
    channelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
    content: ownReplyContent,
    emojiTags: [emojiTag],
    linkPreviewTags: [linkPreviewTag],
    mediaTags: [imetaTag],
    mentionPubkeys: [TEST_IDENTITIES.alice.pubkey],
    mentionTags: [mentionTag],
    parentEventId: null,
    sentFromThreadTag: ["buzz:sent-from-thread", rootId, rootContent],
  });

  await page.getByTestId("auxiliary-panel-close").click();

  const sharedRow = timeline
    .getByTestId("message-row")
    .filter({ hasText: replySummary })
    .last();
  await expect
    .poll(async () => {
      if (await sharedRow.isVisible()) return true;
      const scrollToLatest = page.getByTestId("message-scroll-to-latest");
      if (await scrollToLatest.isVisible()) await scrollToLatest.click();
      return false;
    })
    .toBe(true);
  await expect(sharedRow.getByTestId("message-author")).toHaveText(
    "npub1mock...",
  );
  await expect(sharedRow.getByTestId("message-avatar-fallback")).toBeVisible();
  await expect(sharedRow.locator('[data-mention=""]')).toContainText("alice");
  await expect(sharedRow.locator("img[data-custom-emoji]")).toHaveAttribute(
    "src",
    customEmojiUrl,
  );
  await expect(sharedRow.getByTestId("file-card")).toContainText(
    "launch-notes.txt",
  );
  await expect(
    sharedRow.locator('[data-link-preview="github-pull-request"]'),
  ).toContainText("Add Send to channel for thread messages");
  const sourceLine = sharedRow.getByTestId("sent-from-thread");
  await expect(sourceLine).toContainText("Sent from thread:");
  await expect(sourceLine).toHaveClass(/message-markdown/);
  await expect(sourceLine).toHaveClass(/pt-0\.5/);
  await expect(sourceLine).toHaveClass(/text-sm/);
  await expect(sourceLine).toHaveClass(/font-normal/);
  await expect(sourceLine).toHaveClass(/leading-4/);
  await expect(sourceLine).toHaveClass(/text-muted-foreground\/70/);
  const rootLink = sourceLine.locator("[data-message-link]");
  const sourcePrefix = sourceLine.locator("span").first();
  const rootLinkLabel = rootContent;
  await expect(rootLink).toHaveText(rootLinkLabel);
  await expect(rootLink).toHaveAttribute(
    "aria-label",
    "Open thread in general",
  );
  await expect(rootLink).toHaveAttribute("title", rootLinkLabel);
  await expect(rootLink).toHaveClass(/max-w-80/);
  await expect(rootLink).toHaveClass(/truncate/);
  await expect(rootLink).toHaveClass(/inline-block/);
  await expect(rootLink).toHaveClass(/font-medium/);
  await expect(rootLink).not.toHaveClass(/mention-chip/);
  await expect(rootLink).not.toHaveClass(/border-b/);
  const rootLinkText = rootLink.locator("[data-message-link-text]");
  const rootLinkEmoji = rootLink.locator("[data-message-link-emoji]");
  await expect(rootLinkText).toHaveText(` Share source thread ${timestamp}`);
  await expect(rootLinkText).not.toHaveClass(/border-b/);
  await expect(rootLinkEmoji).toHaveText("🧵");
  await expect(rootLinkEmoji).not.toHaveClass(/border-b/);
  await expect(rootLink).not.toHaveAttribute("data-hovered");
  const [prefixColor, linkColorBeforeHover] = await Promise.all([
    sourcePrefix.evaluate((element) => getComputedStyle(element).color),
    rootLink.evaluate((element) => getComputedStyle(element).color),
  ]);
  expect(linkColorBeforeHover).not.toBe(prefixColor);
  await expect
    .poll(() =>
      rootLink.evaluate((element) => getComputedStyle(element).backgroundColor),
    )
    .toBe("rgba(0, 0, 0, 0)");
  await expect
    .poll(() =>
      rootLinkText.evaluate((element) => getComputedStyle(element).boxShadow),
    )
    .toBe("none");

  await rootLink.hover();
  await expect(rootLink).toHaveAttribute("data-hovered", "");
  await expect
    .poll(() => rootLink.evaluate((element) => getComputedStyle(element).color))
    .toBe(linkColorBeforeHover);
  await expect
    .poll(() =>
      rootLinkText.evaluate(
        (element) => getComputedStyle(element).boxShadow !== "none",
      ),
    )
    .toBe(true);

  await expect
    .poll(() =>
      rootLinkEmoji.evaluate((element) => getComputedStyle(element).boxShadow),
    )
    .toBe("none");

  await rootLink.click();
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel.getByTestId("message-thread-head")).toContainText(
    rootContent,
  );
});

test("shows your avatar on your own message when profile avatar is set", async ({
  page,
}) => {
  const message = `Avatar message ${Date.now()}`;
  const avatarUrl =
    'data:image/svg+xml,%3Csvg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"%3E%3Crect width="16" height="16" rx="4" fill="%2300a36c"/%3E%3C/svg%3E';

  await page.goto("/");
  await openSettings(page, "profile");
  await page.getByTestId("profile-avatar-edit").click();
  await page.getByTestId("profile-avatar-url").fill(avatarUrl);
  await page.getByTestId("profile-avatar-done").click();
  await page.getByTestId("settings-back-to-app").click();

  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill(message);
  await page.getByTestId("send-message").click();

  const lastMessage = page.getByTestId("message-row").last();
  await expect(lastMessage).toContainText(message);
  await expect(lastMessage.getByTestId("message-avatar-image")).toHaveAttribute(
    "src",
    avatarUrl,
  );
});

test("opens a single-level thread panel with inline expansion", async ({
  page,
}) => {
  const timestamp = Date.now();
  const firstReply = `First threaded reply ${timestamp}`;
  const siblingReply = `Sibling threaded reply ${timestamp}`;
  const nestedReply = `Nested threaded reply ${timestamp}`;
  const nestedReplyFromBob = `Nested reply from Bob ${timestamp}`;
  const fillerReplies = Array.from(
    { length: 14 },
    (_, index) => `Thread filler reply ${index} ${timestamp}`,
  );

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("message-timeline")).toContainText(
    "Welcome to general",
  );

  const timeline = page.getByTestId("message-timeline");
  const timelineRows = timeline.getByTestId("message-row");
  const threadPanel = page.getByTestId("message-thread-panel");
  const threadBody = threadPanel.getByTestId("message-thread-body");
  const threadComposer = threadPanel.locator('[data-testid="message-input"]');
  const threadSendButton = threadPanel.getByTestId("send-message");
  const threadReplies = threadPanel.getByTestId("message-thread-replies");
  const rootMessage = timelineRows.first();
  const rootMessageId = await rootMessage.getAttribute("data-message-id");
  if (!rootMessageId) {
    throw new Error("Expected root message row to have a data-message-id.");
  }
  const rootSummaryRow = timeline.locator(
    `[data-testid="message-thread-summary"][data-thread-head-id="${rootMessageId}"]`,
  );

  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel.getByTestId("message-thread-head")).toContainText(
    "Welcome to general",
  );

  await threadComposer.fill(firstReply);
  await threadSendButton.click();
  await expect(threadReplies).toContainText(firstReply);

  await threadComposer.fill(siblingReply);
  await threadSendButton.click();
  await expect(threadReplies).toContainText(siblingReply);

  for (const fillerReply of fillerReplies) {
    await threadComposer.fill(fillerReply);
    await threadSendButton.click();
    await expect(threadReplies).toContainText(fillerReply);
  }

  await expect
    .poll(async () => {
      return threadBody.evaluate((element) => {
        const body = element as HTMLDivElement;
        return body.scrollHeight - body.clientHeight;
      });
    })
    // Compact continuation rows intentionally reduce the available overflow;
    // this test only needs enough space to prove the thread body scrolls.
    .toBeGreaterThan(0);

  await expect(
    timeline.getByTestId("message-row").filter({ hasText: firstReply }),
  ).toHaveCount(0);
  await expect(
    timeline.getByTestId("message-row").filter({ hasText: siblingReply }),
  ).toHaveCount(0);

  await expect(rootSummaryRow).toContainText("16 replies");
  await expect(
    rootSummaryRow.getByTestId("message-thread-summary-participant"),
  ).toHaveCount(1);
  await expect
    .poll(() =>
      rootSummaryRow
        .getByTestId("message-thread-summary-participant")
        .first()
        .evaluate((wrapper) => {
          const avatar = wrapper.firstElementChild;
          if (!(avatar instanceof HTMLElement)) return "missing";
          const rect = avatar.getBoundingClientRect();
          return `${Math.round(rect.width)}x${Math.round(rect.height)}`;
        }),
    )
    .toBe("24x24");
  const summaryGeometry = await measureThreadSummaryGeometry(rootSummaryRow);
  expect(
    Math.abs(summaryGeometry.authorLeft - summaryGeometry.bodyLeft),
  ).toBeLessThanOrEqual(1);
  expect(
    Math.abs(summaryGeometry.avatarLeft - summaryGeometry.bodyLeft),
  ).toBeLessThanOrEqual(1);
  expect(
    Math.abs(
      summaryGeometry.summaryButtonContentLeft - summaryGeometry.bodyLeft,
    ),
  ).toBeLessThanOrEqual(1);
  expect(
    Math.abs(
      summaryGeometry.summaryButtonLeft - summaryGeometry.messageRowLeft,
    ),
  ).toBeLessThanOrEqual(1);
  expect(summaryGeometry.summaryButtonLeft).toBeLessThan(
    summaryGeometry.bodyLeft,
  );
  expect(
    Math.abs(
      summaryGeometry.bodyLeft -
        summaryGeometry.summaryButtonLeft -
        summaryGeometry.summaryButtonPaddingLeft,
    ),
  ).toBeLessThanOrEqual(1);
  expect(summaryGeometry.summarySurfaceLeft).toBeLessThan(
    summaryGeometry.avatarLeft,
  );
  expect(
    Math.abs(
      summaryGeometry.avatarLeft - summaryGeometry.summarySurfaceLeft - 4,
    ),
  ).toBeLessThanOrEqual(1);
  expect(
    Math.abs(summaryGeometry.topPadding - summaryGeometry.bottomPadding),
  ).toBeLessThanOrEqual(1);

  await page.mouse.move(0, 0);
  const rootSummaryWidthBeforeHover = await rootSummaryRow.evaluate((row) =>
    Math.round(row.getBoundingClientRect().width),
  );
  await expect
    .poll(() =>
      rootSummaryRow
        .getByTestId("message-thread-summary-last-reply")
        .evaluate((label) =>
          Number.parseFloat(getComputedStyle(label).opacity),
        ),
    )
    .toBeGreaterThan(0.8);
  await expect
    .poll(() =>
      rootSummaryRow
        .getByTestId("message-thread-summary-hover-action")
        .evaluate((label) =>
          Number.parseFloat(getComputedStyle(label).opacity),
        ),
    )
    .toBeLessThan(0.1);
  await rootSummaryRow.hover();
  await expect
    .poll(() =>
      rootSummaryRow
        .getByTestId("message-thread-summary-last-reply")
        .evaluate((label) =>
          Number.parseFloat(getComputedStyle(label).opacity),
        ),
    )
    .toBeLessThan(0.1);
  await expect
    .poll(() =>
      rootSummaryRow
        .getByTestId("message-thread-summary-hover-action")
        .evaluate((label) =>
          Number.parseFloat(getComputedStyle(label).opacity),
        ),
    )
    .toBeGreaterThan(0.8);
  await expect
    .poll(() =>
      rootSummaryRow.evaluate((row) =>
        Math.round(row.getBoundingClientRect().width),
      ),
    )
    .toBe(rootSummaryWidthBeforeHover);

  await threadPanel.getByTestId("auxiliary-panel-close").click();
  await expect(threadPanel).toBeHidden();

  await rootSummaryRow.click();
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel.getByTestId("message-thread-head")).toContainText(
    "Welcome to general",
  );

  const firstReplyRow = threadReplies
    .getByTestId("message-row")
    .filter({ hasText: firstReply })
    .first();
  await firstReplyRow.hover();
  await firstReplyRow.getByRole("button", { name: "Reply" }).click();

  await expect(threadPanel.getByTestId("message-thread-head")).toContainText(
    "Welcome to general",
  );
  await expect(threadPanel.getByTestId("message-thread-back")).toHaveCount(0);

  await threadComposer.fill(nestedReply);
  await threadSendButton.click();

  const nestedReplyRow = threadReplies
    .getByTestId("message-row")
    .filter({ hasText: nestedReply })
    .first();
  await expect(nestedReplyRow).toBeVisible();
  await expect(
    timeline.getByTestId("message-row").filter({ hasText: nestedReply }),
  ).toHaveCount(0);

  await expect(
    threadReplies.getByTestId("message-row").filter({ hasText: siblingReply }),
  ).toHaveCount(1);
  await expectThreadReplyUnobscured(nestedReplyRow);

  const firstReplyId = await firstReplyRow.getAttribute("data-message-id");
  if (!firstReplyId) {
    throw new Error("Expected first reply row to have a data-message-id.");
  }

  await page.evaluate(
    ({ content, parentEventId, pubkey }) => {
      window.__BUZZ_E2E_EMIT_MOCK_MESSAGE__?.({
        channelName: "general",
        content,
        parentEventId,
        pubkey,
      });
    },
    {
      content: nestedReplyFromBob,
      parentEventId: firstReplyId,
      pubkey: TEST_IDENTITIES.bob.pubkey,
    },
  );
  const nestedReplyFromBobRow = threadReplies
    .getByTestId("message-row")
    .filter({ hasText: nestedReplyFromBob })
    .first();
  await expect(nestedReplyFromBobRow).toBeVisible();

  const firstReplySummaryRow = threadReplies.locator(
    `[data-testid="message-thread-summary"][data-thread-head-id="${firstReplyId}"]`,
  );
  await expect(firstReplySummaryRow).toHaveCount(0);
  const firstReplyBranchGuide = threadReplies.locator(
    `[data-testid="thread-collapse-guide"][data-thread-head-id="${firstReplyId}"]`,
  );
  await expect(firstReplyBranchGuide).not.toHaveCount(0);

  await expect(rootSummaryRow).toContainText("18 replies");
  await expect(
    rootSummaryRow.getByTestId("message-thread-summary-participant"),
  ).toHaveCount(2);
  await expect
    .poll(() =>
      rootSummaryRow
        .getByTestId("message-thread-summary-participant")
        .evaluateAll((participants) =>
          participants
            .map((participant) => getComputedStyle(participant).zIndex)
            .join(","),
        ),
    )
    .toBe("1,2");

  await expectThreadReplyUnobscured(nestedReplyRow);

  await firstReplyBranchGuide.first().click();
  await expect(firstReplySummaryRow).toHaveCount(1);
  await expect(firstReplySummaryRow).toContainText("2 replies");
  await expect(
    threadReplies.getByTestId("message-row").filter({ hasText: nestedReply }),
  ).toHaveCount(0);
  await expect(
    threadReplies
      .getByTestId("message-row")
      .filter({ hasText: nestedReplyFromBob }),
  ).toHaveCount(0);
});

test("thread panel width uses session storage and reset handle", async ({
  page,
}) => {
  const customWidthPx = 520;
  const defaultWidthPx = 380;

  await page.addInitScript((width) => {
    window.sessionStorage.setItem(
      "buzz.desktop.thread-panel-width",
      String(width),
    );
  }, customWidthPx);

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const timeline = page.getByTestId("message-timeline");
  const rootMessage = timeline.getByTestId("message-row").first();
  const threadPanel = page.getByTestId("message-thread-panel");
  const resizeHandle = threadPanel.getByTestId(
    "right-auxiliary-pane-resize-handle",
  );

  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();
  await expect(threadPanel).toBeVisible();

  await expect
    .poll(async () => {
      return threadPanel.evaluate((panel) => {
        const element = panel as HTMLElement;
        return Math.round(element.getBoundingClientRect().width);
      });
    })
    .toBe(customWidthPx);

  await resizeHandle.dblclick();

  await expect
    .poll(async () => {
      return threadPanel.evaluate((panel) => {
        const element = panel as HTMLElement;
        return Math.round(element.getBoundingClientRect().width);
      });
    })
    .toBe(defaultWidthPx);

  await threadPanel.getByTestId("auxiliary-panel-close").click();
  await expect(threadPanel).toBeHidden();

  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();
  await expect(threadPanel).toBeVisible();

  await expect
    .poll(async () => {
      return threadPanel.evaluate((panel) => {
        const element = panel as HTMLElement;
        return Math.round(element.getBoundingClientRect().width);
      });
    })
    .toBe(defaultWidthPx);
});

test("narrow thread view collapses channel header actions into a menu", async ({
  page,
}) => {
  await page.setViewportSize({ width: 980, height: 720 });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("channel-add-bot-trigger")).toHaveCount(0);
  await expect(page.getByTestId("channel-actions-menu-trigger")).toHaveCount(0);

  const rootMessage = page.locator('[data-message-id="mock-general-alice"]');
  const threadPanel = page.getByTestId("message-thread-panel");

  await rootMessage.hover();
  await page.getByTestId("reply-message-mock-general-alice").click();
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel.getByTestId("message-thread-back")).toHaveCount(0);

  const menuTrigger = page.getByTestId("channel-actions-menu-trigger");
  await expect(menuTrigger).toBeVisible();
  await expect(page.getByTestId("channel-add-bot-trigger")).toHaveCount(0);
  await expect(page.getByTestId("channel-members-trigger")).toBeHidden();
  await expect(page.getByTestId("channel-management-trigger")).toBeHidden();

  const menuBox = await menuTrigger.boundingBox();
  const threadPanelBox = await threadPanel.boundingBox();
  if (!menuBox || !threadPanelBox) {
    throw new Error("Expected header action menu and thread panel bounds");
  }
  const menuGap = threadPanelBox.x - (menuBox.x + menuBox.width);
  const headerPaddingInlineEnd = await page
    .getByTestId("chat-header")
    .evaluate((header) =>
      Number.parseFloat(window.getComputedStyle(header).paddingRight),
    );
  expect(menuGap).toBeGreaterThanOrEqual(0);
  expect(menuGap).toBeLessThanOrEqual(headerPaddingInlineEnd + menuBox.width);

  await menuTrigger.click();

  await expect(page.getByTestId("channel-add-bot-trigger")).toHaveCount(0);
  await expect(page.getByTestId("channel-members-trigger")).toBeVisible();
  await expect(page.getByTestId("channel-start-huddle-trigger")).toBeVisible();
  await expect(page.getByTestId("channel-management-trigger")).toBeVisible();
});

test("single-panel thread view hides channel actions", async ({ page }) => {
  await page.setViewportSize({ width: 860, height: 720 });

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("channel-add-bot-trigger")).toHaveCount(0);

  const rootMessage = page.locator('[data-message-id="mock-general-alice"]');
  const threadPanel = page.getByTestId("message-thread-panel");

  await rootMessage.hover();
  await page.getByTestId("reply-message-mock-general-alice").click();
  await expect(threadPanel).toBeVisible();
  await expect(threadPanel.getByTestId("message-thread-back")).toBeVisible();
  await expect(page.getByTestId("channel-actions-menu-trigger")).toHaveCount(0);
  await expect(page.getByTestId("channel-add-bot-trigger")).toHaveCount(0);

  await threadPanel.getByTestId("message-thread-back").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");
  await expect(page.getByTestId("channel-add-bot-trigger")).toHaveCount(0);
});

test("composer is focused after selecting a channel", async ({ page }) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Without clicking the input, typing should land in the composer.
  const input = page.getByTestId("message-input");
  await expect(input).toBeFocused();

  await page.keyboard.type("autofocus-on-channel-select");
  await expect(input).toHaveText("autofocus-on-channel-select");
});

test("composer is focused after switching to a different channel", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("channel-random").click();
  await expect(page.getByTestId("chat-title")).toHaveText("random");

  const input = page.getByTestId("message-input");
  await expect(input).toBeFocused();
});

test("thread composer is focused after clicking the reply icon", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Seed a message to reply to.
  const seed = `Thread autofocus seed ${Date.now()}`;
  const mainInput = page.getByTestId("message-input");
  await mainInput.fill(seed);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(seed);

  const rootMessage = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .last();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();

  const threadInput = threadPanel.getByTestId("message-input");
  await expect(threadInput).toBeFocused();

  await page.keyboard.type("typed-into-thread");
  await expect(threadInput).toHaveText("typed-into-thread");
});

test("thread refetch preserves a live reply and reaction received in flight", async ({
  page,
}) => {
  await installMockBridge(page, { threadRepliesDelayMs: 800 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const rootMessage = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .first();
  const rootId = await rootMessage.getAttribute("data-message-id");
  if (!rootId) throw new Error("Expected a thread root id.");

  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();
  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();

  const reply = `Live reply during thread fetch ${Date.now()}`;
  const replyId = await page.evaluate(
    async ({ channelId, content, parentEventId }) => {
      const bridgeWindow = window as Window & {
        __BUZZ_E2E_INVOKE_MOCK_COMMAND__?: (
          command: string,
          payload?: Record<string, unknown>,
        ) => Promise<unknown>;
      };
      const invoke = bridgeWindow.__BUZZ_E2E_INVOKE_MOCK_COMMAND__;
      if (!invoke) throw new Error("Mock Tauri invoke bridge is unavailable.");
      const sent = (await invoke("send_channel_message", {
        channelId,
        content,
        parentEventId,
      })) as { event_id: string };
      await invoke("add_reaction", { eventId: sent.event_id, emoji: "👍" });
      return sent.event_id;
    },
    {
      channelId: "9a1657ac-f7aa-5db0-b632-d8bbeb6dfb50",
      content: reply,
      parentEventId: rootId,
    },
  );

  const replyRow = threadPanel.locator(`[data-message-id="${replyId}"]`);
  await expect(replyRow).toContainText(reply);
  await expect(replyRow.getByLabel("Toggle 👍 reaction")).toBeVisible();

  // The delayed get_thread_replies response was snapshotted before the live
  // events. Wait past query completion: neither cache addition may disappear.
  await page.waitForTimeout(1_200);
  await expect(replyRow).toContainText(reply);
  await expect(replyRow.getByLabel("Toggle 👍 reaction")).toBeVisible();
});

test("thread reply appears after relay closes and restores its live subscription", async ({
  page,
}) => {
  await installMockBridge(page, { closeChannelLiveSubscriptionOnce: true });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const seed = `Thread CLOSED seed ${Date.now()}`;
  await page.getByTestId("message-input").fill(seed);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(seed);

  const rootMessage = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .last();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();

  const threadPanel = page.getByTestId("message-thread-panel");
  const reply = `Thread reply after CLOSED ${Date.now()}`;
  await threadPanel.getByTestId("message-input").fill(reply);
  await page.waitForTimeout(1_100);
  await threadPanel.getByTestId("send-message").click();

  await expect(threadPanel).toContainText(reply);
});

test("thread composer keeps focus after sending a thread reply", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  // Seed a root message we can open a thread on. At this point only one
  // composer is mounted, so plain getByTestId is unambiguous.
  const seed = `Thread focus-after-send seed ${Date.now()}`;
  await page.getByTestId("message-input").fill(seed);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(seed);

  const rootMessage = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .last();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();

  const threadInput = threadPanel.getByTestId("message-input");
  await expect(threadInput).toBeFocused();

  // Send a thread reply. After the send, `isSending` flips and back to false
  // in both the main and thread composers; the thread input must keep focus.
  const reply = `Thread reply ${Date.now()}`;
  await page.keyboard.type(reply);
  await expect(threadInput).toHaveText(reply);
  await page.keyboard.press("Enter");

  // Wait for the send to settle.
  await expect(threadPanel).toContainText(reply);

  // The thread input should still be focused — not the main composer.
  // Both composers expose the same `message-input` data-testid, so we
  // verify directly that `document.activeElement` lives inside the thread
  // panel rather than the main pane.
  const focusInThreadPanel = await page.evaluate(() => {
    const panel = document.querySelector<HTMLElement>(
      '[data-testid="message-thread-panel"]',
    );
    const active = document.activeElement as HTMLElement | null;
    return Boolean(panel && active && panel.contains(active));
  });
  expect(focusInThreadPanel).toBe(true);

  await expect(threadInput).toBeFocused();
});

test("ArrowUp in an empty composer edits your last message right after sending", async ({
  page,
}) => {
  // Regression: after a send, the composer keeps DOM focus and ProseMirror
  // would consume ArrowUp before it reached the edit-last-message handler,
  // so ↑ did nothing until you clicked out and back. The handler now lives
  // in the editor keymap, so ↑ must work with no intermediate click.
  const message = `Edit-last via arrow up ${Date.now()}`;
  const input = page.getByTestId("message-input");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await input.fill(message);
  await input.press("Enter");
  await expect(page.getByTestId("message-timeline")).toContainText(message);

  // Composer stays focused after send — no click, just press ↑.
  await expect(input).toBeFocused();
  await page.keyboard.press("ArrowUp");

  // Edit mode is entered for the just-sent message.
  const editBanner = page.getByTestId("edit-target");
  await expect(editBanner).toBeVisible();
  await expect(editBanner).toContainText("Editing message");
  await expect(editBanner).not.toContainText(message);
  await expect(input).toHaveText(message);
});

test("ArrowUp does not edit when the composer has draft text", async ({
  page,
}) => {
  // Guard: ↑ must only hijack to edit when the composer is empty, so it
  // never steals the arrow key from someone navigating drafted text.
  const sent = `Sent before draft ${Date.now()}`;
  const draft = `Half-typed draft ${Date.now()}`;
  const input = page.getByTestId("message-input");

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await input.fill(sent);
  await input.press("Enter");
  await expect(page.getByTestId("message-timeline")).toContainText(sent);

  await input.fill(draft);
  await expect(input).toHaveText(draft);
  await page.keyboard.press("ArrowUp");

  // No edit mode; the draft is untouched.
  await expect(page.getByTestId("edit-target")).toHaveCount(0);
  await expect(input).toHaveText(draft);
});

test("ArrowUp edits your last thread reply right after sending it", async ({
  page,
}) => {
  // Same fix must hold in the thread composer (shares MessageComposer).
  const seed = `Thread arrow-up seed ${Date.now()}`;
  const reply = `Thread reply to edit ${Date.now()}`;

  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  await page.getByTestId("message-input").fill(seed);
  await page.getByTestId("send-message").click();
  await expect(page.getByTestId("message-timeline")).toContainText(seed);

  const rootMessage = page
    .getByTestId("message-timeline")
    .getByTestId("message-row")
    .last();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();

  const threadPanel = page.getByTestId("message-thread-panel");
  await expect(threadPanel).toBeVisible();
  const threadInput = threadPanel.getByTestId("message-input");
  await expect(threadInput).toBeFocused();

  await page.keyboard.type(reply);
  await page.keyboard.press("Enter");
  await expect(threadPanel).toContainText(reply);

  // No click — press ↑ in the still-focused thread composer.
  await page.keyboard.press("ArrowUp");

  const editBanner = threadPanel.getByTestId("edit-target");
  await expect(editBanner).toBeVisible();
  await expect(editBanner).toContainText("Editing message");
  await expect(editBanner).not.toContainText(reply);
  await expect(threadInput).toHaveText(reply);
});

test("action bar stays within the timeline when the thread panel is open", async ({
  page,
}) => {
  // Narrow viewport + open thread panel => the timeline shrinks to a column.
  // A long unbreakable token must not widen message rows past that column,
  // or the right-anchored action bar is pushed offscreen (regression: #1081
  // fixed the wrap but rows still expanded to content min-width).
  await page.setViewportSize({ width: 1024, height: 800 });
  await page.goto("/");
  await page.getByTestId("channel-general").click();
  await expect(page.getByTestId("chat-title")).toHaveText("general");

  const timeline = page.getByTestId("message-timeline");
  const input = page.getByTestId("message-input").first();
  const send = page.getByTestId("send-message").first();

  const longUrl = `https://example.com/${"a".repeat(180)}/path`;
  await input.fill(longUrl);
  await send.click();
  await expect(timeline).toContainText("example.com");

  const rootMessage = timeline.getByTestId("message-row").first();
  await rootMessage.hover();
  await rootMessage.getByRole("button", { name: "Reply" }).click();
  await expect(page.getByTestId("message-thread-panel")).toBeVisible();

  const wideRow = timeline.getByTestId("message-row").last();
  await wideRow.scrollIntoViewIfNeeded();
  await wideRow.hover();
  const bar = wideRow.locator('[data-testid^="message-action-bar-"]');
  await expect(bar).toBeVisible();

  const timelineBox = await timeline.boundingBox();
  const rowBox = await wideRow.boundingBox();
  const barBox = await bar.boundingBox();
  if (!timelineBox || !rowBox || !barBox) {
    throw new Error("Expected timeline, row, and action bar to have geometry.");
  }

  expect(rowBox.x + rowBox.width).toBeLessThanOrEqual(
    timelineBox.x + timelineBox.width + 1,
  );
  expect(barBox.x + barBox.width).toBeLessThanOrEqual(
    timelineBox.x + timelineBox.width + 1,
  );
});
