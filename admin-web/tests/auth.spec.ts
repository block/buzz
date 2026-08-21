import { expect, type Page, test } from "@playwright/test";

const TOKEN =
  "5f0e1d2c3b4a59687786958493a2b1c0decadebeefcafe0123456789abcdef01";
const STORAGE_KEY = "buzz-admin-token";

async function seedToken(page: Page, token = TOKEN) {
  await page.addInitScript(
    ([key, value]) => {
      sessionStorage.setItem(key, value);
    },
    [STORAGE_KEY, token],
  );
}

/// Records the Authorization header of every admin API call the SPA makes.
async function recordAuthorization(page: Page, body: unknown = []) {
  const seen: (string | undefined)[] = [];
  await page.route("**/api/admin/v1/**", async (route) => {
    const authorization = route.request().headers().authorization;
    seen.push(authorization);
    // Simulate token-mode relay: 401 without a credential, 200 with one.
    if (!authorization) {
      await route.fulfill({
        status: 401,
        contentType: "application/json",
        body: JSON.stringify({
          error: { code: "unauthorized", message: "token required" },
        }),
      });
    } else {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify(body),
      });
    }
  });
  return seen;
}

test("the dashboard prompts for a token before any api call", async ({
  page,
}) => {
  const seen = await recordAuthorization(page);
  await page.goto("/reports");
  await expect(
    page.getByRole("heading", { name: "Admin token required" }),
  ).toBeVisible();
  // The probe fires unauthenticated (no Authorization header), but no
  // bearer-credentialed call has been made yet.
  expect(seen.filter(Boolean)).toHaveLength(0);

  await page.getByPlaceholder("Admin token").fill(TOKEN);
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(
    page.getByRole("heading", { name: "Open reports" }),
  ).toBeVisible();
  expect(seen).toContain(`Bearer ${TOKEN}`);
});

test("api calls carry the stored token", async ({ page }) => {
  await seedToken(page);
  const seen = await recordAuthorization(page);
  await page.goto("/reports");
  await expect(
    page.getByRole("heading", { name: "Open reports" }),
  ).toBeVisible();
  expect(seen).toEqual([`Bearer ${TOKEN}`]);
});

test("the token survives a reload within the session", async ({ page }) => {
  // Simulate token mode: 401 for unauthenticated, 200 for authenticated.
  await page.route("**/api/admin/v1/**", (route) => {
    const authorization = route.request().headers().authorization;
    if (!authorization) {
      route.fulfill({
        status: 401,
        contentType: "application/json",
        body: JSON.stringify({
          error: { code: "unauthorized", message: "token required" },
        }),
      });
    } else {
      route.fulfill({ contentType: "application/json", body: "[]" });
    }
  });
  await page.goto("/reports");
  await page.getByPlaceholder("Admin token").fill(TOKEN);
  await page.getByRole("button", { name: "Continue" }).click();
  await expect(
    page.getByRole("heading", { name: "Open reports" }),
  ).toBeVisible();

  await page.reload();
  await expect(
    page.getByRole("heading", { name: "Open reports" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Admin token required" }),
  ).toHaveCount(0);
});

test("a rejected token clears storage and re-prompts once", async ({
  page,
}) => {
  await seedToken(page, "f".repeat(64));
  await page.route("**/api/admin/v1/**", (route) =>
    route.fulfill({
      status: 401,
      contentType: "application/json",
      body: JSON.stringify({
        error: { code: "unauthorized", message: "rejected" },
      }),
    }),
  );

  await page.goto("/reports");
  const prompt = page.getByRole("heading", { name: "Admin token required" });
  await expect(prompt).toHaveCount(1);
  await expect(page.getByText("That token was rejected.")).toBeVisible();
  expect(
    await page.evaluate((key) => sessionStorage.getItem(key), STORAGE_KEY),
  ).toBeNull();
});

test("attachments are fetched with the token and rendered from blob urls", async ({
  page,
}) => {
  const id = "feedback-with-attachments";
  const imageHash = "a".repeat(64);
  const fileHash = "b".repeat(64);
  const imageUrl = `https://design.buzz.xyz/media/${imageHash}.png`;
  const fileUrl = `https://design.buzz.xyz/media/${fileHash}.txt`;
  await seedToken(page);

  const attachmentRequests: { path: string; authorization?: string }[] = [];
  await page.route(`**/api/admin/v1/feedback/${id}/attachments/**`, (route) => {
    attachmentRequests.push({
      path: new URL(route.request().url()).pathname,
      authorization: route.request().headers().authorization,
    });
    route.fulfill({ contentType: "application/octet-stream", body: "bytes" });
  });
  await page.route(`**/api/admin/v1/feedback/${id}`, (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        id,
        communityId: "one",
        communityHost: "design.buzz.xyz",
        eventId: "31".repeat(32),
        submitterPubkey: "21".repeat(32),
        category: "bug",
        body: "Composer froze.",
        tags: [
          [
            "imeta",
            `url ${imageUrl}`,
            "m image/png",
            `x ${imageHash}`,
            "filename screenshot.png",
          ],
          [
            "imeta",
            `url ${fileUrl}`,
            "m text/plain",
            `x ${fileHash}`,
            "filename diagnostics.txt",
          ],
        ],
        eventCreatedAt: "2026-07-17T17:25:00Z",
        receivedAt: "2026-07-17T17:30:00Z",
      }),
    }),
  );

  await page.goto(`/feedback/${id}`);
  await expect(
    page.getByRole("img", { name: "screenshot.png" }),
  ).toHaveAttribute("src", /^blob:/);
  await expect(
    page.getByRole("link", { name: /diagnostics.txt/ }),
  ).toHaveAttribute("href", /^blob:/);

  expect(attachmentRequests.map((request) => request.path).sort()).toEqual(
    [
      `/api/admin/v1/feedback/${id}/attachments/${imageHash}`,
      `/api/admin/v1/feedback/${id}/attachments/${fileHash}`,
    ].sort(),
  );
  for (const request of attachmentRequests) {
    expect(request.authorization).toBe(`Bearer ${TOKEN}`);
  }
});

interface ObjectUrlLog {
  created: string[];
  revoked: string[];
}

declare global {
  interface Window {
    objectUrlLog: ObjectUrlLog;
    clearedCount: number;
  }
}

/// Records every object URL the SPA creates and revokes, so a test can prove a
/// blob handed to the DOM is released rather than merely replaced.
async function instrumentObjectUrls(page: Page) {
  await page.addInitScript(() => {
    const log: ObjectUrlLog = { created: [], revoked: [] };
    window.objectUrlLog = log;
    const create = URL.createObjectURL.bind(URL);
    const revoke = URL.revokeObjectURL.bind(URL);
    URL.createObjectURL = (source: Blob | MediaSource) => {
      const url = create(source);
      log.created.push(url);
      return url;
    };
    URL.revokeObjectURL = (url: string) => {
      log.revoked.push(url);
      revoke(url);
    };
  });
}

const FEEDBACK_ID = "feedback-with-attachments";
const IMAGE_HASH = "a".repeat(64);
const FILE_HASH = "b".repeat(64);

/// A feedback detail carrying one image and one non-image attachment.
async function routeFeedbackDetail(page: Page) {
  const host = "design.buzz.xyz";
  await page.route(`**/api/admin/v1/feedback?**`, (route) =>
    route.fulfill({ contentType: "application/json", body: "[]" }),
  );
  await page.route(`**/api/admin/v1/feedback/${FEEDBACK_ID}`, (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        id: FEEDBACK_ID,
        communityId: "one",
        communityHost: host,
        eventId: "31".repeat(32),
        submitterPubkey: "21".repeat(32),
        category: "bug",
        body: "Composer froze.",
        tags: [
          [
            "imeta",
            `url https://${host}/media/${IMAGE_HASH}.png`,
            "m image/png",
            `x ${IMAGE_HASH}`,
            "filename screenshot.png",
          ],
          [
            "imeta",
            `url https://${host}/media/${FILE_HASH}.txt`,
            "m text/plain",
            `x ${FILE_HASH}`,
            "filename diagnostics.txt",
          ],
        ],
        eventCreatedAt: "2026-07-17T17:25:00Z",
        receivedAt: "2026-07-17T17:30:00Z",
      }),
    }),
  );
}

test("attachment object urls are revoked when the view is left", async ({
  page,
}) => {
  await seedToken(page);
  await instrumentObjectUrls(page);
  await routeFeedbackDetail(page);
  await page.route(
    `**/api/admin/v1/feedback/${FEEDBACK_ID}/attachments/**`,
    (route) =>
      route.fulfill({ contentType: "application/octet-stream", body: "bytes" }),
  );

  await page.goto(`/feedback/${FEEDBACK_ID}`);
  const imageUrl = await page
    .getByRole("img", { name: "screenshot.png" })
    .getAttribute("src");
  const fileUrl = await page
    .getByRole("link", { name: /diagnostics.txt/ })
    .getAttribute("href");
  expect(imageUrl).toMatch(/^blob:/);
  expect(fileUrl).toMatch(/^blob:/);
  expect(await page.evaluate(() => window.objectUrlLog.revoked)).toEqual([]);

  await page.getByRole("link", { name: "Back to feedback" }).click();
  await expect(page.getByRole("heading", { name: "Feedback" })).toBeVisible();

  await expect
    .poll(() => page.evaluate(() => window.objectUrlLog.revoked))
    .toEqual(expect.arrayContaining([imageUrl, fileUrl]));
});

test("an attachment that arrives after the view is left is revoked immediately", async ({
  page,
}) => {
  await seedToken(page);
  await instrumentObjectUrls(page);
  await routeFeedbackDetail(page);
  let release = () => {};
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.route(
    `**/api/admin/v1/feedback/${FEEDBACK_ID}/attachments/**`,
    async (route) => {
      await held;
      await route.fulfill({
        contentType: "application/octet-stream",
        body: "bytes",
      });
    },
  );

  await page.goto(`/feedback/${FEEDBACK_ID}`);
  // Both fetches are held, so no blob exists yet.
  await expect(page.getByText("Loading…")).toBeVisible();
  expect(await page.evaluate(() => window.objectUrlLog.created)).toEqual([]);

  // Leave before either fetch resolves, then let both complete.
  await page.getByRole("link", { name: "Back to feedback" }).click();
  await expect(page.getByRole("heading", { name: "Feedback" })).toBeVisible();
  release();

  await expect
    .poll(() => page.evaluate(() => window.objectUrlLog.revoked.length))
    .toBe(2);
  const log = await page.evaluate(() => window.objectUrlLog);
  expect(log.revoked.sort()).toEqual(log.created.sort());
  // Nothing was ever handed to the DOM: the blobs outlived their view.
  await expect(page.getByRole("img", { name: "screenshot.png" })).toHaveCount(
    0,
  );
});

test("concurrent rejected requests re-prompt exactly once", async ({
  page,
}) => {
  await seedToken(page, "f".repeat(64));
  await routeFeedbackDetail(page);
  // Counts how many rejections reached the centralized clearing path, so the
  // test can distinguish "two 401s collapsed into one prompt" from "only one
  // request ever failed".
  await page.addInitScript(() => {
    let cleared = 0;
    const remove = sessionStorage.removeItem.bind(sessionStorage);
    sessionStorage.removeItem = (key: string) => {
      cleared += 1;
      remove(key);
    };
    Object.defineProperty(window, "clearedCount", { get: () => cleared });
  });

  // Both attachment requests are held until the second arrives, so the two
  // 401s are in flight at the same time.
  let secondArrived = () => {};
  const bothInFlight = new Promise<void>((resolve) => {
    secondArrived = resolve;
  });
  let pending = 2;
  await page.route(
    `**/api/admin/v1/feedback/${FEEDBACK_ID}/attachments/**`,
    async (route) => {
      pending -= 1;
      if (pending === 0) secondArrived();
      await bothInFlight;
      await route.fulfill({
        status: 401,
        contentType: "application/json",
        body: JSON.stringify({
          error: { code: "unauthorized", message: "rejected" },
        }),
      });
    },
  );

  await page.goto(`/feedback/${FEEDBACK_ID}`);
  await expect(
    page.getByRole("heading", { name: "Admin token required" }),
  ).toHaveCount(1);
  await expect(page.getByText("That token was rejected.")).toBeVisible();
  expect(
    await page.evaluate((key) => sessionStorage.getItem(key), STORAGE_KEY),
  ).toBeNull();
  expect(pending).toBe(0);
  await expect
    .poll(() => page.evaluate(() => window.clearedCount))
    .toBeGreaterThanOrEqual(2);
  await expect(
    page.getByRole("heading", { name: "Admin token required" }),
  ).toHaveCount(1);
});

test("probe: disabled mode skips the token prompt when probe returns 200", async ({
  page,
}) => {
  // No token in storage. The probe to /api/admin/v1/reports returns 200,
  // indicating the relay runs in disabled mode. The dashboard must
  // render directly without showing the token prompt.
  await page.route("**/api/admin/v1/reports**", (route) =>
    route.fulfill({ contentType: "application/json", body: "[]" }),
  );

  await page.goto("/reports");

  await expect(
    page.getByRole("heading", { name: "Open reports" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Admin token required" }),
  ).toHaveCount(0);
});

test("probe: token mode shows the prompt when probe returns 401", async ({
  page,
}) => {
  // No token in storage. The probe to /api/admin/v1/reports returns 401,
  // indicating the relay requires a bearer token. The prompt must be shown.
  await page.route("**/api/admin/v1/**", (route) =>
    route.fulfill({
      status: 401,
      headers: { "www-authenticate": "Bearer" },
      contentType: "application/json",
      body: JSON.stringify({
        error: { code: "unauthorized", message: "token required" },
      }),
    }),
  );

  await page.goto("/reports");

  await expect(
    page.getByRole("heading", { name: "Admin token required" }),
  ).toBeVisible();
  // The prompt must not say "rejected" on a fresh first visit.
  await expect(page.getByText("That token was rejected.")).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Open reports" })).toHaveCount(
    0,
  );
});

test("probe: nip98 mode without a NIP-07 extension shows the installation screen", async ({
  page,
}) => {
  // No token in storage. The probe returns 401 with WWW-Authenticate: Nostr,
  // and window.nostr is NOT injected. The dashboard must show the extension
  // installation screen instead of the token prompt or the dashboard.
  await page.route("**/api/admin/v1/**", (route) =>
    route.fulfill({
      status: 401,
      headers: { "www-authenticate": "Nostr" },
      contentType: "application/json",
      body: JSON.stringify({
        error: { code: "unauthorized", message: "nip98 required" },
      }),
    }),
  );

  await page.goto("/reports");

  await expect(
    page.getByRole("heading", { name: "Nostr extension required" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Admin token required" }),
  ).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "Open reports" })).toHaveCount(
    0,
  );
});

test("probe: nip98 mode with a mocked NIP-07 extension signs requests and renders the dashboard", async ({
  page,
}) => {
  // Inject a minimal window.nostr stub that returns a fake signed event.
  // The relay mock accepts any Authorization: Nostr header.
  await page.addInitScript(() => {
    (window as Window & { nostr?: unknown }).nostr = {
      signEvent: async (event: {
        kind: number;
        created_at: number;
        tags: string[][];
        content: string;
      }) => ({
        ...event,
        id: "a".repeat(64),
        pubkey: "b".repeat(64),
        sig: "c".repeat(128),
      }),
    };
  });

  const authorizationHeaders: (string | undefined)[] = [];
  await page.route("**/api/admin/v1/**", async (route) => {
    const headers = route.request().headers();
    authorizationHeaders.push(headers.authorization);
    // Probe: return 401 Nostr to trigger nip98 mode detection.
    if (!headers.authorization) {
      await route.fulfill({
        status: 401,
        headers: { "www-authenticate": "Nostr" },
        contentType: "application/json",
        body: JSON.stringify({
          error: { code: "unauthorized", message: "nip98 required" },
        }),
      });
    } else {
      // Any Authorization: Nostr header → accept.
      await route.fulfill({ contentType: "application/json", body: "[]" });
    }
  });

  await page.goto("/reports");

  await expect(
    page.getByRole("heading", { name: "Open reports" }),
  ).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Nostr extension required" }),
  ).toHaveCount(0);
  await expect(
    page.getByRole("heading", { name: "Admin token required" }),
  ).toHaveCount(0);
  // The authenticated request used Authorization: Nostr.
  const authenticatedHeaders = authorizationHeaders.filter(Boolean);
  expect(authenticatedHeaders.length).toBeGreaterThan(0);
  for (const h of authenticatedHeaders) {
    expect(h).toMatch(/^Nostr /);
  }
});

test("nip98 mode: first-401-then-200 retries once and renders the dashboard", async ({
  page,
}) => {
  // Models a credential that is momentarily rejected (clock skew, key
  // rotation) then accepted on the second attempt.
  let signCount = 0;
  await page.addInitScript(() => {
    (window as Window & { nostr?: unknown }).nostr = {
      signEvent: async (event: {
        kind: number;
        created_at: number;
        tags: string[][];
        content: string;
      }) => ({
        ...event,
        id: "a".repeat(64),
        pubkey: "b".repeat(64),
        sig: "c".repeat(128),
      }),
    };
  });

  const authCalls: string[] = [];
  await page.route("**/api/admin/v1/**", async (route) => {
    const headers = route.request().headers();
    if (!headers.authorization) {
      // Probe — announce nip98 mode.
      await route.fulfill({
        status: 401,
        headers: { "www-authenticate": "Nostr" },
        contentType: "application/json",
        body: JSON.stringify({
          error: { code: "unauthorized", message: "nip98 required" },
        }),
      });
      return;
    }
    authCalls.push(headers.authorization);
    signCount++;
    if (signCount === 1) {
      // First authenticated attempt → reject.
      await route.fulfill({
        status: 401,
        headers: { "www-authenticate": "Nostr" },
        contentType: "application/json",
        body: JSON.stringify({
          error: { code: "unauthorized", message: "rejected" },
        }),
      });
    } else {
      // Second attempt → accept.
      await route.fulfill({ contentType: "application/json", body: "[]" });
    }
  });

  await page.goto("/reports");

  // The retry should succeed. Wait for network to settle (both attempts
  // complete) before asserting the authCalls count.
  await page.waitForLoadState("networkidle");
  // Exactly two distinct Nostr credentials were sent (one per attempt).
  expect(authCalls).toHaveLength(2);
  for (const h of authCalls) {
    expect(h).toMatch(/^Nostr /);
  }
});

test("nip98 mode: persistent 401 surfaces error after exactly one retry", async ({
  page,
}) => {
  // Every authenticated request returns 401. The SPA must attempt exactly
  // two requests (first attempt + one retry) and then surface the error —
  // never a third attempt.
  await page.addInitScript(() => {
    (window as Window & { nostr?: unknown }).nostr = {
      signEvent: async (event: {
        kind: number;
        created_at: number;
        tags: string[][];
        content: string;
      }) => ({
        ...event,
        id: "a".repeat(64),
        pubkey: "b".repeat(64),
        sig: "c".repeat(128),
      }),
    };
  });

  const authCalls: string[] = [];
  await page.route("**/api/admin/v1/**", async (route) => {
    const headers = route.request().headers();
    if (!headers.authorization) {
      await route.fulfill({
        status: 401,
        headers: { "www-authenticate": "Nostr" },
        contentType: "application/json",
        body: JSON.stringify({
          error: { code: "unauthorized", message: "nip98 required" },
        }),
      });
      return;
    }
    authCalls.push(headers.authorization);
    await route.fulfill({
      status: 401,
      headers: { "www-authenticate": "Nostr" },
      contentType: "application/json",
      body: JSON.stringify({
        error: { code: "unauthorized", message: "rejected" },
      }),
    });
  });

  await page.goto("/reports");

  // After the retry fails, the error state renders. The StateView shows
  // "Could not load data" inside a role=alert region.
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(
    page.getByRole("heading", { name: "Could not load data" }),
  ).toBeVisible();
  // Exactly two Nostr credentials sent — no third attempt.
  await page.waitForLoadState("networkidle");
  expect(authCalls).toHaveLength(2);
});
