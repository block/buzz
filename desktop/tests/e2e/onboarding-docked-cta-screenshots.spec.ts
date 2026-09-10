import { expect, type Locator, type Page, test } from "@playwright/test";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge, TEST_IDENTITIES } from "../helpers/bridge";
import { seedActiveIdentity } from "../helpers/onboarding";

const BLANK_TYLER_IDENTITY = {
  ...TEST_IDENTITIES.tyler,
  username: "",
};

const SHOT_DIR = "test-results/onboarding-docked-cta";
const COMMUNITY_ONBOARDING_TRANSACTION_STORAGE_KEY =
  "buzz-community-onboarding-transaction.v1";
const NCRYPTSEC =
  "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p";

test.use({ viewport: { width: 1280, height: 800 } });

async function expectSharedCardGeometry(page: Page, expectedWidth = 610) {
  const geometry = await page
    .getByTestId("onboarding-content-card")
    .evaluate((element) => {
      const rect = element.getBoundingClientRect();
      const styles = window.getComputedStyle(element);
      return {
        borderRadius: styles.borderRadius,
        height: rect.height,
        paddingBottom: styles.paddingBottom,
        paddingLeft: styles.paddingLeft,
        paddingRight: styles.paddingRight,
        paddingTop: styles.paddingTop,
        width: rect.width,
      };
    });

  expect(geometry.width).toBeCloseTo(expectedWidth, 0);
  expect(geometry.height).toBeCloseTo(664, 0);
  expect(geometry.borderRadius).toBe("32px");
  expect(geometry.paddingTop).toBe("48px");
  expect(geometry.paddingRight).toBe("48px");
  expect(geometry.paddingBottom).toBe("48px");
  expect(geometry.paddingLeft).toBe("48px");
}

async function expectUsesFullCardWidth(element: Locator) {
  const geometry = await element.evaluate((node) => {
    const available = node.closest<HTMLElement>(
      ".buzz-onboarding-transition-content",
    );
    if (!available) throw new Error("Onboarding content column is missing");
    const availableBox = available.getBoundingClientRect();
    const elementBox = node.getBoundingClientRect();
    return {
      availableLeft: availableBox.left,
      availableWidth: availableBox.width,
      elementLeft: elementBox.left,
      elementWidth: elementBox.width,
    };
  });
  expect(geometry.elementLeft).toBeCloseTo(geometry.availableLeft, 0);
  expect(geometry.elementWidth).toBeCloseTo(geometry.availableWidth, 0);
}

async function expectHorizontalCardTransition(
  page: Page,
  pageTestId: string,
  expectedDirection: "forward" | "backward",
) {
  const transition = page
    .getByTestId(pageTestId)
    .locator(".buzz-onboarding-transition-line")
    .first();
  await expect(transition).toHaveAttribute(
    "data-onboarding-direction",
    expectedDirection,
  );

  const motion = await transition.evaluate((line) => {
    const content = line.querySelector<HTMLElement>(
      ":scope > .buzz-onboarding-transition-content",
    );
    if (!content) throw new Error("Onboarding transition content is missing");
    const frame = line.closest<HTMLElement>(".buzz-onboarding-step-frame");
    if (!frame) throw new Error("Onboarding transition frame is missing");

    const animationName = window.getComputedStyle(content).animationName;
    const keyframes: Array<{ transform: string; x: number; y: number }> = [];
    const visitRules = (rules: CSSRuleList) => {
      for (const rule of Array.from(rules)) {
        if (rule instanceof CSSKeyframesRule && rule.name === animationName) {
          for (const frame of Array.from(rule.cssRules)) {
            const transform = (frame as CSSKeyframeRule).style.transform;
            const matrix = new DOMMatrixReadOnly(transform || "none");
            keyframes.push({
              transform: transform || "none",
              x: matrix.m41,
              y: matrix.m42,
            });
          }
          continue;
        }
        if ("cssRules" in rule) {
          try {
            visitRules((rule as CSSGroupingRule).cssRules);
          } catch {
            // Cross-origin and unsupported grouping rules are irrelevant here.
          }
        }
      }
    };
    for (const styleSheet of Array.from(document.styleSheets)) {
      try {
        visitRules(styleSheet.cssRules);
      } catch {
        // Ignore stylesheets whose rules the browser does not expose.
      }
    }

    const activeFrames = line
      .getAnimations({ subtree: true })
      .flatMap((animation) => {
        const effect = animation.effect;
        if (!(effect instanceof KeyframeEffect)) return [];
        const target = effect.target;
        return effect
          .getKeyframes()
          .filter((frame) => frame.transform && frame.transform !== "none")
          .map((frame) => {
            const transform = String(frame.transform);
            const matrix = new DOMMatrixReadOnly(transform);
            return {
              target:
                target instanceof HTMLElement
                  ? target.className.toString()
                  : (target?.nodeName ?? "unknown"),
              transform,
              x: matrix.m41,
              y: matrix.m42,
            };
          });
      });

    const frameRect = frame.getBoundingClientRect();
    const lineRect = line.getBoundingClientRect();
    const travel = 48;
    const runway = {
      backwardStart: lineRect.left - travel,
      forwardEnd: lineRect.left + content.offsetWidth + travel,
      frameLeft: frameRect.left,
      frameRight: frameRect.right,
    };

    return { activeFrames, animationName, keyframes, runway };
  });

  expect(motion.animationName).toBe(
    `buzz-onboarding-line-slide-${expectedDirection}`,
  );
  expect(motion.keyframes.length).toBeGreaterThanOrEqual(2);
  for (const frame of [...motion.keyframes, ...motion.activeFrames]) {
    expect(
      Math.abs(frame.y),
      `Unexpected vertical motion in ${frame.transform}`,
    ).toBeLessThan(0.01);
  }
  const enteringFrame = motion.keyframes[0];
  if (!enteringFrame) throw new Error("Transition entry frame is missing");
  expect(enteringFrame.x).toBe(expectedDirection === "forward" ? 48 : -48);
  if (expectedDirection === "forward") {
    expect(motion.runway.forwardEnd).toBeLessThanOrEqual(
      motion.runway.frameRight + 0.5,
    );
  } else {
    expect(motion.runway.backwardStart).toBeGreaterThanOrEqual(
      motion.runway.frameLeft - 0.5,
    );
  }
}

test("machine onboarding: landing, backup, setup docked CTAs", async ({
  page,
}) => {
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");

  const gate = page.getByTestId("machine-onboarding-gate");
  await expect(gate).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/01-landing.png` });

  await page.getByRole("button", { name: "Use an existing key" }).click();
  await expect(
    page.getByRole("heading", { name: "Enter your private key" }),
  ).toBeVisible();
  await expectHorizontalCardTransition(
    page,
    "machine-onboarding-gate",
    "forward",
  );
  const importCard = page.getByTestId("onboarding-content-card");
  await expect(importCard).toBeVisible();
  await expectSharedCardGeometry(page);
  await expect(page.getByLabel("Private key", { exact: true })).toBeVisible();
  await expectUsesFullCardWidth(page.getByTestId("nostr-import-nsec-input"));
  await expect(importCard).toHaveCSS("background-color", "rgb(255, 255, 255)");
  await expect(page.getByTestId("nostr-import-card")).toHaveCount(0);
  await expect(importCard.locator("svg filter")).toHaveCount(0);
  const onboardingBack = page.getByTestId("onboarding-back");
  await expect(onboardingBack).toHaveCSS("width", "52px");
  await expect(onboardingBack).toHaveCSS("height", "52px");
  await expect(onboardingBack.locator("svg")).toHaveCSS("width", "24px");
  await expect(onboardingBack.locator("svg")).toHaveCSS("height", "24px");
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/01b-enter-key.png` });

  await page.getByTestId("nostr-import-file-button").click();
  await expect(page.getByTestId("backup-recovery-dialog")).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/01c-backup-file-sheet.png` });
  await page.getByRole("button", { name: "Back", exact: true }).click();

  await page.getByTestId("nostr-import-phone-link").click();
  await expect(page.getByTestId("phone-recovery-dialog")).toBeVisible();
  await expect(page.getByTestId("identity-recovery-qr")).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/01d-phone-recovery-sheet.png` });
  await page.getByRole("button", { name: "Back", exact: true }).click();

  await page.getByTestId("nostr-import-nsec-input").fill(NCRYPTSEC);
  await expect(
    page.getByRole("heading", { name: "Unlock your account" }),
  ).toBeVisible();
  await expect(page.getByTestId("backup-password-timeline")).toBeVisible();
  await expect(page.getByTestId("restore-ncryptsec-affordance")).toBeVisible();
  await expect(page.getByTestId("restore-unlock-icon")).toBeVisible();
  await expect(page.getByTestId("nostr-import-passphrase")).toBeFocused();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/01e-restore-backup.png` });

  // The first Back returns to key selection; the second leaves import.
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(importCard).toBeVisible();
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Create a new identity key" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await expect(
    page.getByRole("heading", { name: "Create a private identity key" }),
  ).toBeVisible();
  await expectHorizontalCardTransition(
    page,
    "onboarding-page-key-intro",
    "forward",
  );
  await expectUsesFullCardWidth(page.getByTestId("onboarding-key-guidance"));
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/02-key-introduction.png` });
  await page.getByRole("button", { name: "Create my private key" }).click();
  await expect(
    page.getByRole("heading", {
      name: "Your private identity key",
    }),
  ).toBeVisible();
  await expectHorizontalCardTransition(
    page,
    "onboarding-page-backup",
    "forward",
  );
  await expectSharedCardGeometry(page);
  const keyGeometry = await page.evaluate(() => {
    const keyWell = document
      .querySelector('[data-testid="backup-key-well"]')
      ?.getBoundingClientRect();
    const keyWellStyles = window.getComputedStyle(
      document.querySelector('[data-testid="backup-key-well"]') ??
        document.documentElement,
    );
    const backupRow = document
      .querySelector('[data-testid="backup-option-password"]')
      ?.getBoundingClientRect();
    const copyButton = document
      .querySelector('[data-testid="backup-copy-key"]')
      ?.getBoundingClientRect();
    const keyValue = document.querySelector('[data-testid="backup-key-value"]');
    const keyRows = keyValue
      ? (() => {
          const range = document.createRange();
          range.selectNodeContents(keyValue);
          return new Set(
            Array.from(range.getClientRects()).map((rect) =>
              Math.round(rect.top),
            ),
          ).size;
        })()
      : 0;
    return {
      backupRowHeight: backupRow?.height ?? 0,
      backupRowWidth: backupRow?.width ?? 0,
      copyButtonHeight: copyButton?.height ?? 0,
      keyRows,
      keyWellPaddingLeft: keyWellStyles.paddingLeft,
      keyWellPaddingRight: keyWellStyles.paddingRight,
      keyWellHeight: keyWell?.height ?? 0,
      keyWellWidth: keyWell?.width ?? 0,
    };
  });
  expect(keyGeometry.keyWellWidth).toBeCloseTo(512, 0);
  expect(keyGeometry.keyWellHeight).toBeCloseTo(122, 0);
  expect(keyGeometry.keyRows).toBe(2);
  expect(keyGeometry.keyWellPaddingLeft).toBe("16px");
  expect(keyGeometry.keyWellPaddingRight).toBe("16px");
  expect(keyGeometry.copyButtonHeight).toBeCloseTo(32, 0);
  expect(keyGeometry.backupRowWidth).toBeCloseTo(512, 0);
  expect(keyGeometry.backupRowHeight).toBeCloseTo(48, 0);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/02-backup.png` });

  const backupOption = page.getByTestId("backup-option-password");
  await backupOption.hover();
  await expect(backupOption).not.toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/02a-backup-option-hover.png` });

  // Private-key text is masked by default; explicit reveal fetches it, and the
  // existing hover/copy treatment remains operable afterward.
  const keyValue = page.getByTestId("backup-key-value");
  const copyButton = page.getByTestId("backup-copy-key");
  const revealButton = page.getByTestId("backup-reveal-key");
  await expect(keyValue).toBeVisible();
  await expect(keyValue).not.toContainText("nsec1mock");
  await revealButton.click();
  await expect(keyValue).toContainText("nsec1mock");
  await expect(revealButton).toHaveAttribute("aria-label", "Hide private key");
  await expect(copyButton).toBeVisible();
  await expect(copyButton).toBeEnabled();
  await copyButton.click();
  await expect(copyButton).toContainText("Copied to clipboard");
  await expect(keyValue).toHaveCSS("filter", "none");
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/02b-backup-hover.png` });

  // The locked-backup action is part of the generated-key sheet.
  await page.getByTestId("backup-option-password").click();
  await expect(page.getByTestId("onboarding-page-download")).toBeVisible();
  await expectHorizontalCardTransition(
    page,
    "onboarding-page-download",
    "forward",
  );
  const passwordPanel = page.getByTestId("backup-password-panel");
  await expect(passwordPanel).toBeVisible();
  await expectUsesFullCardWidth(passwordPanel);
  await expect(passwordPanel).not.toHaveClass(/buzz-card-textured/);
  await expect(passwordPanel).toHaveCSS("padding-left", "0px");
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/02d-backup-password.png` });

  await page.getByTestId("backup-passphrase-generate").click();
  const generatorPopover = page.getByRole("dialog");
  await expect(generatorPopover).toBeVisible();
  await expect(generatorPopover).not.toHaveClass(/buzz-card-textured/);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/02e-backup-generator.png` });
  await page.keyboard.press("Escape");

  await page.getByTestId("backup-return-to-onboarding").click();
  await expect(page.getByTestId("onboarding-page-backup")).toBeVisible();
  await expectHorizontalCardTransition(
    page,
    "onboarding-page-backup",
    "backward",
  );
  await page.getByTestId("onboarding-next").click();
  await expect(
    page.getByRole("heading", { name: "Connect your AI provider" }),
  ).toBeVisible();
  const subscriptionMethod = page.getByTestId(
    "onboarding-harness-method-subscription",
  );
  const apiMethod = page.getByTestId("onboarding-harness-method-api");
  await expect(subscriptionMethod).toContainText(
    "Simpler setup — use the harness and models included with your AI subscription",
  );
  await expect(apiMethod).toContainText(
    "More flexibility — choose a compatible harness, provider, and model",
  );
  await expect(subscriptionMethod).not.toHaveCSS(
    "background-color",
    "rgba(0, 0, 0, 0)",
  );
  await expectUsesFullCardWidth(subscriptionMethod);
  await expectUsesFullCardWidth(apiMethod);
  await expectSharedCardGeometry(page);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/03-setup.png` });

  await page.getByTestId("onboarding-harness-method-subscription").click();
  await expect(
    page.getByRole("heading", { name: "Continue with an AI subscription" }),
  ).toBeVisible();
  await expectHorizontalCardTransition(page, "onboarding-page-2", "forward");
  await expect(page.getByText(/More harnesses can be added in/)).toHaveCount(0);
  await expect(page.getByTestId("onboarding-setup-skip")).toBeVisible();
  await expect(page.getByTestId("onboarding-setup-next")).toHaveCount(0);
  await expect(
    page.getByText("CLI not detected", { exact: false }),
  ).toHaveCount(0);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/03b-subscriptions.png` });

  await page.getByTestId("onboarding-runtime-details-codex").click();
  await expect(
    page.getByTestId("onboarding-harness-setup-guide"),
  ).toBeVisible();
  const guideCard = page.getByTestId("onboarding-harness-setup-guide-card");
  await expect(guideCard).toContainText("Codex");
  await expect(guideCard).toContainText(
    "Codex is not detected on this computer.",
  );
  await expect(
    guideCard.getByTestId("onboarding-harness-open-setup-guide"),
  ).toHaveText("Open guide");
  await expect(
    page.getByTestId("onboarding-runtime-install-codex"),
  ).toHaveCount(0);
  await expectHorizontalCardTransition(page, "onboarding-page-2", "forward");
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/03c-harness-guide.png` });

  await page.getByTestId("onboarding-back").click();
  await expect(
    page.getByRole("heading", { name: "Continue with an AI subscription" }),
  ).toBeVisible();
  await expectHorizontalCardTransition(page, "onboarding-page-2", "backward");
  await page.getByTestId("onboarding-back").click();
  await expect(
    page.getByRole("heading", { name: "Connect your AI provider" }),
  ).toBeVisible();
  await expectHorizontalCardTransition(page, "onboarding-page-2", "backward");
  await page.getByTestId("onboarding-harness-method-api").click();
  await expect(
    page.getByRole("heading", { name: "Connect with an API key" }),
  ).toBeVisible();
  await expect(
    page.getByText(
      "Choose your provider and enter an API key to connect to the Buzz harness.",
    ),
  ).toBeVisible();
  await expectHorizontalCardTransition(
    page,
    "onboarding-page-config",
    "forward",
  );
  await expect(page.getByTestId("global-agent-default-harness")).toHaveCount(0);
  await expectUsesFullCardWidth(page.getByTestId("global-agent-provider"));
  await expect(
    page.getByTestId("onboarding-use-different-harness"),
  ).toBeVisible();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/03d-api-buzz-model.png` });

  await page.getByTestId("global-agent-provider").click();
  await page.getByTestId("global-agent-provider-option-anthropic").click();
  await expect(page.getByTestId("persona-provider-api-key")).toBeVisible();
  await expect(
    page.getByText("ANTHROPIC_API_KEY", { exact: true }),
  ).toHaveCount(0);
  await expect(page.getByTestId("global-agent-model")).toHaveCount(0);
  await expect(
    page.getByTestId("global-agent-thinking-effort-select"),
  ).toHaveCount(0);
  const providerBeforeKey = await page
    .getByTestId("global-agent-provider")
    .boundingBox();
  const apiKeyBeforeKey = await page
    .getByTestId("persona-provider-api-key")
    .boundingBox();
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOT_DIR}/03d2-api-provider-selected.png`,
  });

  await page.getByTestId("persona-provider-api-key").fill("sk-test-key");
  await expect(page.getByTestId("global-agent-model")).toHaveText(
    "Claude Opus 4.6",
  );
  const providerAfterKey = await page
    .getByTestId("global-agent-provider")
    .boundingBox();
  const apiKeyAfterKey = await page
    .getByTestId("persona-provider-api-key")
    .boundingBox();
  expect(providerBeforeKey).not.toBeNull();
  expect(apiKeyBeforeKey).not.toBeNull();
  expect(providerAfterKey).not.toBeNull();
  expect(apiKeyAfterKey).not.toBeNull();
  expect(
    Math.abs((providerBeforeKey?.y ?? 0) - (providerAfterKey?.y ?? 0)),
  ).toBeLessThan(2);
  expect(
    Math.abs((apiKeyBeforeKey?.y ?? 0) - (apiKeyAfterKey?.y ?? 0)),
  ).toBeLessThan(2);
  await waitForAnimations(page);
  await page.screenshot({
    path: `${SHOT_DIR}/03d3-api-key-entered.png`,
  });

  await page.getByTestId("onboarding-use-different-harness").click();
  await expect(
    page.getByRole("heading", { name: "Choose a harness" }),
  ).toBeVisible();
  await expectHorizontalCardTransition(page, "onboarding-page-2", "forward");
  await expect(page.getByTestId("onboarding-runtime-buzz-agent")).toContainText(
    "Recommended",
  );
  await expect(
    page.getByTestId("onboarding-runtime-ready-buzz-agent"),
  ).toHaveCount(0);
  await page.getByTestId("onboarding-runtime-buzz-agent").hover();
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/03e-api-harnesses.png` });

  await page.getByTestId("onboarding-back").click();
  await expect(
    page.getByRole("heading", { name: "Connect with an API key" }),
  ).toBeVisible();
  await expect(page.getByTestId("global-agent-provider")).toHaveText(
    "Anthropic",
  );
  await expect(page.getByTestId("persona-provider-api-key")).toHaveValue(
    "sk-test-key",
  );
  await expectHorizontalCardTransition(
    page,
    "onboarding-page-config",
    "backward",
  );
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/03f-api-return.png` });
});

test("machine key import remains usable in a short viewport", async ({
  page,
}) => {
  await page.setViewportSize({ width: 900, height: 620 });
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Use an existing key" }).click();

  const heading = page.getByRole("heading", { name: "Enter your private key" });
  const input = page.getByLabel("Private key", { exact: true });
  const footer = page.getByTestId("onboarding-footer-slot");
  await expect(heading).toBeVisible();
  await expect(input).toBeVisible();
  await expect(footer).toBeVisible();

  const layout = await page.evaluate(() => {
    const heading = document.querySelector("h1")?.getBoundingClientRect();
    const input = document
      .querySelector<HTMLInputElement>("#nostr-private-key")
      ?.getBoundingClientRect();
    const footer = document
      .querySelector('[data-testid="onboarding-footer-slot"]')
      ?.getBoundingClientRect();
    return {
      footerTop: footer?.top ?? 0,
      headingBottom: heading?.bottom ?? 0,
      inputBottom: input?.bottom ?? 0,
      inputTop: input?.top ?? 0,
      clientWidth: document.documentElement.clientWidth,
      scrollHeight: document.documentElement.scrollHeight,
      scrollWidth: document.documentElement.scrollWidth,
    };
  });
  expect(layout.inputTop).toBeGreaterThan(layout.headingBottom);
  expect(layout.footerTop).toBeGreaterThan(layout.inputBottom);
  expect(layout.scrollHeight).toBeGreaterThanOrEqual(620);
  expect(layout.scrollWidth).toBe(layout.clientWidth);
});

test("identity-key help stays inside the onboarding card", async ({ page }) => {
  await page.setViewportSize({ width: 900, height: 650 });
  await installMockBridge(page, undefined, {
    skipCommunitySeed: true,
    skipOnboardingSeed: true,
  });
  await page.goto("/");
  await page.getByRole("button", { name: "Create a new identity key" }).click();
  await expect(
    page.getByRole("button", { name: "Learn how identity keys work" }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "Learn how identity keys work" })
    .click();

  const help = page.getByTestId("identity-key-help-dialog");
  await expect(help).toBeVisible();
  await expect(
    help.getByRole("heading", { name: "What’s an identity key?" }),
  ).toBeVisible();
  await expectUsesFullCardWidth(help.getByTestId("identity-key-help-body"));
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/02f-identity-key-help.png` });
  const geometry = await help.evaluate((element) => ({
    clientWidth: document.documentElement.clientWidth,
    left: element.getBoundingClientRect().left,
    right: element.getBoundingClientRect().right,
    scrollWidth: document.documentElement.scrollWidth,
  }));
  expect(geometry.left).toBeGreaterThanOrEqual(0);
  expect(geometry.right).toBeLessThanOrEqual(geometry.clientWidth);
  expect(geometry.scrollWidth).toBe(geometry.clientWidth);
});

test("relay onboarding: profile and avatar docked CTAs", async ({ page }) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await installMockBridge(page, undefined, { skipOnboardingSeed: true });
  await page.goto("/");

  await expect(page.getByTestId("onboarding-page-1")).toBeVisible();
  await expectSharedCardGeometry(page);
  await expect(page.getByTestId("onboarding-back")).toBeVisible();
  await page.getByTestId("onboarding-display-name").fill("Ada Lovelace");
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/04-profile.png` });

  await page.getByTestId("onboarding-next").click();
  await expect(page.getByTestId("onboarding-page-avatar")).toBeVisible();
  await page
    .getByTestId("onboarding-avatar-url")
    .fill("https://example.com/onboarding-avatar.png");
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/05-avatar.png` });
});

test("community onboarding: profile and starter-team cards", async ({
  page,
}) => {
  await seedActiveIdentity(page, BLANK_TYLER_IDENTITY);
  await page.addInitScript(
    ({ pubkey, transactionStorageKey }) => {
      window.localStorage.setItem(
        `buzz-machine-onboarding-complete.v2:${pubkey}`,
        "true",
      );
      const timestamp = new Date().toISOString();
      window.localStorage.setItem(
        transactionStorageKey,
        JSON.stringify({
          id: "screenshot-community-profile",
          source: "first-community",
          stage: "profile",
          relayUrl: "ws://localhost:3000",
          communityName: "Default",
          communityId: "e2e-default-community",
          addedCommunity: true,
          createdAt: timestamp,
          updatedAt: timestamp,
        }),
      );
    },
    {
      pubkey: BLANK_TYLER_IDENTITY.pubkey,
      transactionStorageKey: COMMUNITY_ONBOARDING_TRANSACTION_STORAGE_KEY,
    },
  );
  await installMockBridge(
    page,
    { profileHasEvent: false },
    {
      relayWsUrl: "ws://localhost:3000",
      skipOnboardingSeed: true,
    },
  );
  await page.goto("/");

  await expect(
    page.getByRole("heading", { name: "Build your profile" }),
  ).toBeVisible();
  await expect(page.getByTestId("onboarding-content-card")).toBeVisible();
  await expectSharedCardGeometry(page);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/06-community-profile.png` });

  await page.getByTestId("community-profile-name-key").fill("Ada Lovelace");
  await page.getByTestId("community-profile-next").click();
  await expect(
    page.getByRole("heading", { name: "Meet your starter team" }),
  ).toBeVisible();
  await expectSharedCardGeometry(page);
  await waitForAnimations(page);
  await page.screenshot({ path: `${SHOT_DIR}/07-starter-team.png` });
});
