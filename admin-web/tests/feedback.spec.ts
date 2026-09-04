import { expect, type Page, test } from "@playwright/test";

/// Injects a minimal window.nostr stub so a test can drive nip98 mode without
/// a real NIP-07 extension. It echoes the tags it signed (including the
/// `payload` tag added for body-bearing requests) so a test can assert the
/// credential committed to the exact body bytes.
async function seedNip98(page: Page) {
  await page.addInitScript(() => {
    (window as Window & { nostr?: unknown }).nostr = {
      signEvent: async (event: {
        kind: number;
        created_at: number;
        tags: string[][];
        content: string;
      }) => {
        const serialized = JSON.stringify([
          event.kind,
          event.created_at,
          event.tags,
          event.content,
        ]);
        let h = 0;
        for (let i = 0; i < serialized.length; i++) {
          h = (Math.imul(31, h) + serialized.charCodeAt(i)) | 0;
        }
        const id = (h >>> 0).toString(16).padStart(8, "0").repeat(8);
        return {
          ...event,
          id,
          pubkey: "b".repeat(64),
          sig: "c".repeat(128),
        };
      },
    };
  });
}

function decodeNostrHeader(header: string): { tags: string[][] } {
  return JSON.parse(atob(header.replace(/^Nostr /, "")));
}

async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(text),
  );
  return Array.from(new Uint8Array(digest), (b) =>
    b.toString(16).padStart(2, "0"),
  ).join("");
}

const FEEDBACK_ONE = "11111111-1111-1111-1111-111111111111";

function summary(overrides: Record<string, unknown>) {
  return {
    id: FEEDBACK_ONE,
    communityId: "6d474feb-c50a-44e4-a0b5-f30532df49bc",
    communityHost: "design.buzz.xyz",
    submitterPubkey: "21".repeat(32),
    category: "bug",
    bodySummary: "Composer froze on paste.",
    status: "new",
    receivedAt: "2026-07-17T17:30:00Z",
    ...overrides,
  };
}

test("nip98 mode: changing status PATCHes the relay with a payload-bound credential", async ({
  page,
}) => {
  await seedNip98(page);

  const patches: { method: string; body: string; authorization?: string }[] =
    [];
  await page.route("**/api/admin/v1/feedback", async (route) => {
    // The list load is authenticated (probe already resolved nip98 mode).
    await route.fulfill({
      contentType: "application/json",
      body: JSON.stringify([summary({ status: "new" })]),
    });
  });
  await page.route("**/api/admin/v1/reports**", (route) => {
    // Probe: reject unauthenticated so the SPA resolves nip98 mode.
    if (!route.request().headers().authorization) {
      route.fulfill({
        status: 401,
        headers: { "www-authenticate": "Nostr" },
        contentType: "application/json",
        body: JSON.stringify({ error: { code: "unauthorized", message: "x" } }),
      });
      return;
    }
    route.fulfill({ contentType: "application/json", body: "[]" });
  });
  await page.route(
    `**/api/admin/v1/feedback/${FEEDBACK_ONE}`,
    async (route) => {
      const request = route.request();
      patches.push({
        method: request.method(),
        body: request.postData() ?? "",
        authorization: request.headers().authorization,
      });
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ status: "reviewed" }),
      });
    },
  );

  await page.goto("/feedback");
  await expect(page.getByRole("heading", { name: "Feedback" })).toBeVisible();

  await page.getByRole("combobox").last().selectOption("reviewed");

  await expect.poll(() => patches.length).toBeGreaterThan(0);
  const patch = patches[0];
  expect(patch.method).toBe("PATCH");
  expect(JSON.parse(patch.body)).toEqual({ status: "reviewed" });
  // The credential is a NIP-98 event whose payload tag is the SHA-256 of the
  // exact body bytes the request sent.
  expect(patch.authorization).toMatch(/^Nostr /);
  const { tags } = decodeNostrHeader(patch.authorization as string);
  const payload = tags.find((t) => t[0] === "payload")?.[1];
  expect(payload).toBe(await sha256Hex(patch.body));
  // The row adopts the status from the PATCH response.
  await expect(page.getByRole("combobox").last()).toHaveValue("reviewed");
});

test("nip98 mode: a failed PATCH surfaces an error and does not lie about the status", async ({
  page,
}) => {
  await seedNip98(page);

  await page.route("**/api/admin/v1/feedback", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify([summary({ status: "new" })]),
    }),
  );
  await page.route("**/api/admin/v1/reports**", (route) => {
    if (!route.request().headers().authorization) {
      route.fulfill({
        status: 401,
        headers: { "www-authenticate": "Nostr" },
        contentType: "application/json",
        body: JSON.stringify({ error: { code: "unauthorized", message: "x" } }),
      });
      return;
    }
    route.fulfill({ contentType: "application/json", body: "[]" });
  });
  // Gate the failing PATCH so the test can observe the in-flight (pending)
  // state before the 500 lands.
  let releasePatch: () => void = () => {};
  const patchInFlight = new Promise<void>((resolve) => {
    releasePatch = resolve;
  });
  await page.route(
    `**/api/admin/v1/feedback/${FEEDBACK_ONE}`,
    async (route) => {
      await patchInFlight;
      await route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({ error: { code: "internal", message: "boom" } }),
      });
    },
  );

  await page.goto("/feedback");
  const control = page.getByRole("combobox").last();
  await control.selectOption("archived");

  // While the PATCH is in flight the control is disabled, shows no error, and
  // still reads the original server status — an optimistic set (even one that
  // rolls back on failure) would surface the unpersisted value here.
  await expect(control).toBeDisabled();
  await expect(control).toHaveValue("new");
  await expect(page.getByRole("alert")).toHaveCount(0);

  releasePatch();

  // The failed write surfaces an error and leaves the control at the original
  // server status — it never optimistically adopts the requested value.
  await expect(page.getByRole("alert")).toHaveText("Update failed");
  await expect(control).toHaveValue("new");
  await expect(control).toBeEnabled();
});

test("disabled mode: the status control is a read-only badge with no write affordance", async ({
  page,
}) => {
  // Probe returns 200 → disabled mode. No mutation must be offered.
  await page.route("**/api/admin/v1/reports**", (route) =>
    route.fulfill({ contentType: "application/json", body: "[]" }),
  );
  await page.route("**/api/admin/v1/feedback", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify([summary({ status: "reviewed" })]),
    }),
  );

  let patched = false;
  await page.route(`**/api/admin/v1/feedback/${FEEDBACK_ONE}`, (route) => {
    patched = true;
    route.fulfill({ contentType: "application/json", body: "{}" });
  });

  await page.goto("/feedback");
  await expect(page.getByRole("heading", { name: "Feedback" })).toBeVisible();
  // Authoritative status is shown as a badge; the only combobox on the page is
  // the status filter, not a per-row write control.
  await expect(page.getByText("Reviewed").last()).toBeVisible();
  await expect(page.getByRole("combobox")).toHaveCount(3);
  await page.waitForTimeout(200);
  expect(patched).toBe(false);
});

test("a purged feedback row (null provenance) renders without crashing search or attachments", async ({
  page,
}) => {
  // Disabled mode keeps the test focused on nullable-host handling.
  await page.route("**/api/admin/v1/reports**", (route) =>
    route.fulfill({ contentType: "application/json", body: "[]" }),
  );
  await page.route("**/api/admin/v1/feedback", (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify([
        summary({ communityId: null, communityHost: null, status: "new" }),
      ]),
    }),
  );

  await page.goto("/feedback");
  await expect(page.getByText("Community unavailable").first()).toBeVisible();
  // Search a term that only matches the submitter pubkey (index 3) — the
  // filter must evaluate the null host (index 1) on the way there without
  // throwing. "2121" is present in the pubkey but not the body summary.
  await page.getByPlaceholder("Search feedback").fill("2121");
  await expect(page.getByText("Composer froze on paste.")).toBeVisible();
  await expect(page.getByText("1 of 1 submissions")).toBeVisible();
});

test("a purged feedback detail derives no attachments from an absent host", async ({
  page,
}) => {
  const imageHash = "a".repeat(64);
  await page.route("**/api/admin/v1/reports**", (route) =>
    route.fulfill({ contentType: "application/json", body: "[]" }),
  );
  await page.route("**/api/admin/v1/feedback", (route) =>
    route.fulfill({ contentType: "application/json", body: "[]" }),
  );
  let attachmentRequested = false;
  await page.route(
    `**/api/admin/v1/feedback/${FEEDBACK_ONE}/attachments/**`,
    (route) => {
      attachmentRequested = true;
      route.fulfill({ contentType: "application/octet-stream", body: "x" });
    },
  );
  await page.route(`**/api/admin/v1/feedback/${FEEDBACK_ONE}`, (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        id: FEEDBACK_ONE,
        communityId: null,
        communityHost: null,
        eventId: "31".repeat(32),
        submitterPubkey: "21".repeat(32),
        category: "bug",
        body: "Broken.\n\n![shot](https://design.buzz.xyz/media/x.png)",
        tags: [
          [
            "imeta",
            "url https://design.buzz.xyz/media/x.png",
            "m image/png",
            `x ${imageHash}`,
            "filename shot.png",
          ],
        ],
        status: "new",
        eventCreatedAt: "2026-07-17T17:25:00Z",
        receivedAt: "2026-07-17T17:30:00Z",
      }),
    }),
  );

  await page.goto(`/feedback/${FEEDBACK_ONE}`);
  await expect(page.getByText("Community unavailable").first()).toBeVisible();
  await expect(page.getByText("Attachments")).toHaveCount(0);
  await page.waitForTimeout(200);
  // The authoritative host is absent, so no attachment fetch is derived.
  expect(attachmentRequested).toBe(false);
});

test("a hostile attachment served as octet-stream is download-only, never a navigable typed document", async ({
  page,
}) => {
  // The reporter's imeta claims image/png, but the relay sniffs the stored
  // bytes and forces application/octet-stream for anything that is not a
  // verified passive raster image (e.g. an HTML/SVG payload). The client must
  // trust the served type, so this attachment renders as a download link — no
  // <img>, no target="_blank" navigation to a typed document on the admin
  // origin.
  const hash = "a".repeat(64);
  await page.route("**/api/admin/v1/reports**", (route) =>
    route.fulfill({ contentType: "application/json", body: "[]" }),
  );
  await page.route("**/api/admin/v1/feedback", (route) =>
    route.fulfill({ contentType: "application/json", body: "[]" }),
  );
  await page.route(
    `**/api/admin/v1/feedback/${FEEDBACK_ONE}/attachments/**`,
    (route) =>
      route.fulfill({
        contentType: "application/octet-stream",
        body: "<script>alert(1)</script>",
      }),
  );
  await page.route(`**/api/admin/v1/feedback/${FEEDBACK_ONE}`, (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        id: FEEDBACK_ONE,
        communityId: "6d474feb-c50a-44e4-a0b5-f30532df49bc",
        communityHost: "design.buzz.xyz",
        eventId: "31".repeat(32),
        submitterPubkey: "21".repeat(32),
        category: "bug",
        body: "Broken.\n\n![shot](https://design.buzz.xyz/media/x.png)",
        tags: [
          [
            "imeta",
            "url https://design.buzz.xyz/media/x.png",
            "m image/png",
            `x ${hash}`,
            "filename shot.png",
          ],
        ],
        status: "new",
        eventCreatedAt: "2026-07-17T17:25:00Z",
        receivedAt: "2026-07-17T17:30:00Z",
      }),
    }),
  );

  await page.goto(`/feedback/${FEEDBACK_ONE}`);
  const link = page.getByRole("link", { name: /shot\.png/ });
  await expect(link).toBeVisible();
  await expect(link).toHaveAttribute("download", "shot.png");
  await expect(link).not.toHaveAttribute("target", "_blank");
  // The hostile payload is never rendered as an inline image.
  await expect(page.locator("figure.image-attachment img")).toHaveCount(0);
});
