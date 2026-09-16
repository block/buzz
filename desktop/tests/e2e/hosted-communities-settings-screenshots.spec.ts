import { expect, test, type Page } from "@playwright/test";
import { npubEncode } from "nostr-tools/nip19";

import { waitForAnimations } from "../helpers/animations";
import { installMockBridge } from "../helpers/bridge";
import { openSettings } from "../helpers/settings";

const OUTDIR = "test-results/hosted-communities";
const DEFAULT_MOCK_PUBKEY = "deadbeef".repeat(8);
/** A second valid identity key, used only as a contradictory hosted npub. */
const OTHER_HEX = "b".repeat(64);

/**
 * Install the default hosted-communities fixture and open its settings
 * section. `builderlabIdentity` overrides the bound account identity so a
 * spec can drive independent — even contradictory — `pubkey_hex`/`npub`
 * fields; the mock bridge passes both through verbatim, like the native
 * command.
 */
async function openHostedCommunitiesSettings(
  page: Page,
  builderlabIdentity?: { npub?: string; pubkey_hex?: string } | null,
) {
  await installMockBridge(page, {
    builderlabAuth: {
      email: "owner@example.com",
      expiresAt: "2099-01-01T00:00:00Z",
    },
    builderlabIdentity: builderlabIdentity ?? {
      pubkey_hex: DEFAULT_MOCK_PUBKEY,
    },
    builderlabCommunities: [
      {
        id: "active-community",
        name: "E2E Test",
        normalized_host: "localhost:3000",
      },
      {
        id: "other-community",
        name: "Design studio",
        normalized_host: "design-studio.communities.buzz.xyz",
      },
    ],
  });
  await page.goto("/");
  await openSettings(page, "hosted-communities");
}

test.beforeEach(async ({ page }) => {
  await openHostedCommunitiesSettings(page);
});

test("identity: mismatch rows follow pubkey_hex, never the hosted npub or raw hex", async ({
  page,
}) => {
  await openHostedCommunitiesSettings(page, {
    pubkey_hex: "f".repeat(64),
    npub: npubEncode(OTHER_HEX),
  });

  await expect(
    page.getByText("This account is connected to a different Buzz identity"),
  ).toBeVisible();
  const settingsView = page.getByTestId("settings-view");
  await expect(
    page
      .getByText("Account uses", { exact: true })
      .locator("xpath=following-sibling::dd[1]"),
  ).toHaveText(npubEncode("f".repeat(64)));
  await expect(
    page
      .getByText("This device", { exact: true })
      .locator("xpath=following-sibling::dd[1]"),
  ).toHaveText(npubEncode(DEFAULT_MOCK_PUBKEY));
  // The independently valid but contradictory hosted npub, and the raw hex
  // it would stand in for, must never render.
  await expect(settingsView.getByText(npubEncode(OTHER_HEX))).toHaveCount(0);
  await expect(settingsView.getByText("f".repeat(64))).toHaveCount(0);
});

test("identity: connected row follows pubkey_hex when the hosted npub encodes another key", async ({
  page,
}) => {
  await openHostedCommunitiesSettings(page, {
    pubkey_hex: DEFAULT_MOCK_PUBKEY,
    npub: npubEncode(OTHER_HEX),
  });

  const connectedNpub = page
    .getByText("Buzz identity connected")
    .locator("span.font-mono");
  await expect(connectedNpub).toHaveText(npubEncode(DEFAULT_MOCK_PUBKEY));
  await expect(
    page.getByTestId("settings-view").getByText(npubEncode(OTHER_HEX)),
  ).toHaveCount(0);
});

test("identity: unusable bound hex renders the neutral label, not the hosted npub or raw hex", async ({
  page,
}) => {
  await openHostedCommunitiesSettings(page, {
    // Valid hex alphabet, wrong length — unusable as an identity key.
    pubkey_hex: "f".repeat(63),
    npub: npubEncode(OTHER_HEX),
  });

  await expect(
    page.getByText("This account is connected to a different Buzz identity"),
  ).toBeVisible();
  const settingsView = page.getByTestId("settings-view");
  await expect(
    page
      .getByText("Account uses", { exact: true })
      .locator("xpath=following-sibling::dd[1]"),
  ).toHaveText("Unavailable");
  await expect(settingsView.getByText(npubEncode(OTHER_HEX))).toHaveCount(0);
  await expect(settingsView.getByText("f".repeat(63))).toHaveCount(0);
});

test("identity: consistent hosted identity renders its canonical npub", async ({
  page,
}) => {
  await openHostedCommunitiesSettings(page, {
    pubkey_hex: DEFAULT_MOCK_PUBKEY,
    npub: npubEncode(DEFAULT_MOCK_PUBKEY),
  });

  const connectedNpub = page
    .getByText("Buzz identity connected")
    .locator("span.font-mono");
  await expect(connectedNpub).toHaveText(npubEncode(DEFAULT_MOCK_PUBKEY));
});

test("identity: unlinked account offers linking, never a connected claim", async ({
  page,
}) => {
  await installMockBridge(page, {
    builderlabAuth: {
      email: "owner@example.com",
      expiresAt: "2099-01-01T00:00:00Z",
    },
    // No identity object at all: the account has not linked a Buzz key.
    builderlabIdentity: null,
    builderlabCommunities: [
      {
        id: "active-community",
        name: "E2E Test",
        normalized_host: "localhost:3000",
      },
    ],
  });
  await page.goto("/");
  await openSettings(page, "hosted-communities");

  await expect(
    page.getByText("Link this account to your Buzz identity"),
  ).toBeVisible();
  await expect(page.getByText("Buzz identity connected")).toHaveCount(0);
  // The seeded owned community still lists — every affordance that does
  // not act on the binding stays available — but Connect is an action on
  // the binding and cannot occur without a usable bound key: no row
  // affordance, and no onboarding it could start.
  await expect(page.getByTestId("hosted-community-row")).toHaveCount(1);
  await expect(
    page.getByRole("button", { name: "Connect", exact: true }),
  ).toHaveCount(0);
  // Creation stays unavailable: there is no authoritative key to bind the
  // new community to.
  await expect(page.getByLabel("Community address")).toBeDisabled();
  await expect(
    page.getByRole("button", { name: "Create and connect", exact: true }),
  ).toBeDisabled();
});

test("identity: padded or mixed-case bound hex is the same key, not a mismatch", async ({
  page,
}) => {
  await openHostedCommunitiesSettings(page, {
    // The same key this device signs with, padded and uppercased. It
    // normalizes to the local key on both sides of the comparison — never
    // a mismatch demanding delete/rebind of an identity the device already
    // holds.
    pubkey_hex: `  ${DEFAULT_MOCK_PUBKEY.toUpperCase()}  `,
  });

  await expect(
    page.getByText("This account is connected to a different Buzz identity"),
  ).toHaveCount(0);
  const connectedNpub = page
    .getByText("Buzz identity connected")
    .locator("span.font-mono");
  await expect(connectedNpub).toHaveText(npubEncode(DEFAULT_MOCK_PUBKEY));
  // A binding that is the same key after normalization keeps its Connect
  // affordance — recovery is reserved for a binding that actually differs.
  await expect(
    page.getByRole("button", { name: "Connect", exact: true }).first(),
  ).toBeVisible();
});

/**
 * Identity payloads whose authoritative `pubkey_hex` cannot act as a key.
 * Each is an identity *object* — so presence alone must never read as a
 * connected, ready account — yet none carries a key the app can use.
 */
const UNUSABLE_BOUND_KEY_PAYLOADS: Array<
  [label: string, payload: { npub?: string; pubkey_hex?: string }]
> = [
  ["missing key", {}],
  ["non-hex key", { pubkey_hex: "zz".repeat(32) }],
  ["npub-only key", { npub: npubEncode(OTHER_HEX) }],
  // A checksum-valid npub stored in the hex field itself is not a hex
  // key: it must fail closed like any other unusable spelling, never
  // display as the account's authoritative key, and never read as a
  // usable binding when the local comparison is skipped.
  ["npub stored in the hex field", { pubkey_hex: npubEncode(OTHER_HEX) }],
];

for (const [label, payload] of UNUSABLE_BOUND_KEY_PAYLOADS) {
  test(`identity: ${label} is recovery, never a connected account or actions`, async ({
    page,
  }) => {
    await openHostedCommunitiesSettings(page, payload);

    const settingsView = page.getByTestId("settings-view");
    // No connected claim anywhere on the surface, despite the identity
    // object being present.
    await expect(page.getByText("Buzz identity connected")).toHaveCount(0);
    // The mismatch recovery block owns the identity panel instead.
    await expect(
      page.getByText("This account is connected to a different Buzz identity"),
    ).toBeVisible();
    await expect(
      page
        .getByText("Account uses", { exact: true })
        .locator("xpath=following-sibling::dd[1]"),
    ).toHaveText("Unavailable");
    // The unverified server-sent npub spelling never renders.
    await expect(settingsView.getByText(npubEncode(OTHER_HEX))).toHaveCount(0);
    // Connect stays unavailable for every owned community.
    await expect(
      page.getByRole("button", { name: "Connect", exact: true }),
    ).toHaveCount(0);
    // Creation stays unavailable: no key the new community would bind to.
    await expect(page.getByLabel("Community address")).toBeDisabled();
    await expect(
      page.getByRole("button", { name: "Create and connect", exact: true }),
    ).toBeDisabled();
  });
}

test("capture: community icon picker sits beside its hosted community", async ({
  page,
}) => {
  const activeRow = page
    .getByTestId("hosted-community-row")
    .filter({ hasText: "E2E Test" });
  const otherRow = page
    .getByTestId("hosted-community-row")
    .filter({ hasText: "Design studio" });

  await expect(activeRow.getByTestId("community-icon-settings")).toBeVisible();
  await expect(otherRow.getByTestId("community-icon-settings")).toHaveCount(0);

  const iconDataUrl = `data:image/svg+xml,${encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128"><rect width="128" height="128" rx="28" fill="#ff56c3"/><text x="64" y="80" text-anchor="middle" font-size="48">😅</text></svg>',
  )}`;
  await activeRow.getByLabel("Add community icon").click();

  const picker = page.getByRole("group", { name: "Community icon picker" });
  await expect(picker).toBeVisible();
  await expect(page.getByRole("tab", { name: "Image" })).toBeVisible();
  await expect(page.getByRole("tab", { name: "Emoji" })).toBeVisible();
  await page.getByPlaceholder("Paste a URL").fill(iconDataUrl);
  await page.getByRole("button", { name: "Apply" }).click();

  const icon = activeRow.getByRole("img", { name: /community icon$/i });
  await expect(icon).toBeVisible();
  const maskImage = await activeRow
    .getByTestId("community-icon-mask")
    .evaluate((element) => getComputedStyle(element).webkitMaskImage);
  expect(maskImage).toContain("radial-gradient");
  await expect(page.getByTestId("community-icon-save")).toHaveCount(0);
  await expect
    .poll(() =>
      page.evaluate(() =>
        window.__BUZZ_E2E_SIGNED_EVENTS__?.some(
          (event) =>
            event.kind === 9033 &&
            event.tags.some(
              (tag) => tag[0] === "icon" && tag[1]?.startsWith("data:image/"),
            ),
        ),
      ),
    )
    .toBe(true);

  const iconBox = await activeRow
    .getByTestId("community-icon-settings")
    .boundingBox();
  const nameBox = await activeRow
    .getByText("E2E Test", { exact: true })
    .boundingBox();
  expect(iconBox).not.toBeNull();
  expect(nameBox).not.toBeNull();
  expect(iconBox?.x ?? Number.POSITIVE_INFINITY).toBeLessThan(
    nameBox?.x ?? Number.NEGATIVE_INFINITY,
  );

  await waitForAnimations(page);
  await activeRow.screenshot({
    path: `${OUTDIR}/01-community-icon-row.png`,
  });
});
