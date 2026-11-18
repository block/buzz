import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

// ── Composer-hook regression: fast clear + re-paste of the same URL ───────────
//
// useComposerLinkPreviews feeds useResolvedLinkPreviews from DEBOUNCED content
// (350ms) but tracks URL-presence newness from the LIVE content. Without that
// live-href signal a fast clear-then-repaste of the same URL inside the debounce
// window never commits an empty debounced set, so the resolver never sees the
// URL leave and never refetches — and the stale snapshot tag built from the
// pre-clear metadata stays sendable. This drives the real composer hook through
// that gesture and asserts the stale tag is not sendable and a fresh fetch is
// forced. (PR #5510, second follow-up.)

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

/** @type {Map<string, (args: unknown) => Promise<unknown>>} */
const ipcHandlers = new Map();

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  dom.window.__TAURI_INTERNALS__ = {
    invoke: (cmd, args) => {
      const handler = ipcHandlers.get(cmd);
      return handler
        ? handler(args)
        : Promise.reject(new Error(`unmocked Tauri command: ${cmd}`));
    },
    transformCallback: () => Math.random(),
  };
});

after(() => dom.window.close());

const HREF = "https://example.com/composer-re-entry";
const DEBOUNCE_WAIT_MS = 400; // > LINK_PREVIEW_DEBOUNCE_MS (350)

function metadata(overrides = {}) {
  return {
    title: "A story",
    siteName: "Example",
    description: "Story description",
    imageDataUrl: null,
    imageDomain: null,
    imageFetchState: "none",
    imageRetryAfterMs: null,
    faviconDataUrl: null,
    ...overrides,
  };
}

test("composer input versions retain only active hrefs while re-entry advances", async () => {
  const { updateComposerLinkPreviewInput } = await import(
    "./useComposerLinkPreviews.tsx"
  );
  const secondHref = "https://example.com/second";
  let input = {
    content: "",
    hrefs: new Set(),
    hrefVersions: new Map(),
    nextHrefVersion: 0,
  };

  input = updateComposerLinkPreviewInput(input, `see ${HREF}`, null);
  const firstVersion = input.hrefVersions.get(HREF);
  assert.equal(input.hrefVersions.size, 1);

  input = updateComposerLinkPreviewInput(input, `see ${secondHref}`, null);
  assert.deepEqual(
    [...input.hrefVersions.keys()],
    [secondHref],
    "departed href history is pruned instead of retained for the composer lifetime",
  );

  input = updateComposerLinkPreviewInput(input, `see ${HREF}`, null);
  assert.deepEqual([...input.hrefVersions.keys()], [HREF]);
  assert.ok(
    input.hrefVersions.get(HREF) > firstVersion,
    "a re-entered href receives a new monotonic version after its old entry was pruned",
  );
});

test("composer forces a refetch and drops the stale tag on a fast clear+re-paste of the same URL", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { resetLinkPreviewMetadataCache } = await import(
    "@/shared/lib/useResolvedLinkPreviews.ts"
  );
  const { updateComposerLinkPreviewInput, useComposerLinkPreviews } =
    await import("./useComposerLinkPreviews.tsx");

  resetLinkPreviewMetadataCache();
  ipcHandlers.clear();

  // Relay origin for media URLs (composer fetches it once on mount).
  ipcHandlers.set("get_relay_http_url", () =>
    Promise.resolve("https://relay.example.com"),
  );
  // Media upload always succeeds instantly so a snapshot tag can be built.
  ipcHandlers.set("upload_media_bytes", () =>
    Promise.resolve({
      url: "https://relay.example.com/media/x",
      sha256: "deadbeef",
      size: 1,
      type: "image/png",
      uploaded: 0,
    }),
  );
  // Call 1 (initial paste) resolves to a transient failure -> sendable fallback,
  // instantly. Call 2 (the forced re-entry refetch) resolves to a success but
  // only when we release `resolveRefetch`, so the test can observe the
  // intermediate window where the stale tag is gone and Send is held pending
  // BEFORE the fresh result lands (in the app the refetch is a real network
  // round-trip; instant resolution would collapse the window under test).
  let fetchCalls = 0;
  let resolveRefetch;
  ipcHandlers.set("fetch_link_preview_metadata", () => {
    fetchCalls += 1;
    if (fetchCalls === 1) {
      return Promise.resolve(
        metadata({
          imageFetchState: "transient_failure",
          imageRetryAfterMs: 900_000,
        }),
      );
    }
    return new Promise((resolve) => {
      resolveRefetch = () =>
        resolve(
          metadata({
            imageDataUrl: "data:image/png;base64,QQ==",
            imageDomain: "images.example.com",
            imageFetchState: "image",
          }),
        );
    });
  });

  const flushDebounceAndSettle = async () => {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, DEBOUNCE_WAIT_MS));
    });
  };

  try {
    let previewInput = updateComposerLinkPreviewInput(
      {
        content: "",
        hrefs: new Set(),
        hrefVersions: new Map(),
        nextHrefVersion: 0,
      },
      `see ${HREF}`,
      null,
    );
    const { result, rerender, unmount } = renderHook(
      ({ content, hrefVersions }) =>
        useComposerLinkPreviews(content, true, hrefVersions),
      { initialProps: previewInput },
    );

    // 1. Paste settles to a transient-failure fallback with a ready snapshot tag.
    await flushDebounceAndSettle();
    assert.equal(fetchCalls, 1);
    assert.equal(
      result.current.getReadyTags().length,
      1,
      "the transient fallback produced a sendable snapshot tag",
    );
    assert.equal(result.current.hasPendingSnapshots, false);

    // 2. Fast gesture: clear the URL, then re-paste the SAME URL, with both
    //    editor updates folded into one React batch. The debounced candidates
    //    and the committed live href set therefore never observe empty.
    // Model two editor onUpdate calls folded into one React batch. The final
    // href set equals the previous commit, but the update-boundary version has
    // advanced because the URL left and re-entered between those updates.
    await act(async () => {
      previewInput = updateComposerLinkPreviewInput(previewInput, "see ", null);
      previewInput = updateComposerLinkPreviewInput(
        previewInput,
        `see ${HREF}`,
        null,
      );
      rerender(previewInput);
    });

    // 3a. The stale tag must be gone AS SOON AS the same URL re-enters — this is
    //     the core invariant and it holds synchronously (render-time detection
    //     drops the tag and excludes the href from sendable output), so no timer
    //     needs to fire first. Send is held pending until a fresh tag lands.
    await act(async () => {});
    assert.equal(
      result.current.getReadyTags().length,
      0,
      "stale snapshot tag must not be sendable the moment the URL re-enters",
    );
    assert.equal(
      result.current.hasPendingSnapshots,
      true,
      "Send must be held pending after a clear+re-paste",
    );

    // 3b. Once the debounce settles, the re-entry has forced a fresh fetch. It is
    //     still in flight (the deferred refetch has NOT resolved), so the stale
    //     tag stays gone and Send stays pending. Pre-fix the re-entry is
    //     invisible, so no refetch starts and this fails fast.
    await flushDebounceAndSettle();
    assert.equal(
      fetchCalls,
      2,
      "the re-entry forced a fresh fetch (still in flight)",
    );
    assert.equal(
      result.current.getReadyTags().length,
      0,
      "stale snapshot tag must not be sendable while the re-entry refetches",
    );
    assert.equal(
      result.current.hasPendingSnapshots,
      true,
      "Send must be held pending while the re-entry refetches",
    );

    // 4. The forced refetch resolves (success); a fresh tag becomes sendable and
    //    its media is the freshly-fetched image, not the pre-clear empty
    //    fallback (proving the tag was rebuilt from new metadata).
    await act(async () => {
      resolveRefetch();
    });
    await flushDebounceAndSettle();
    const [freshTag] = result.current.getReadyTags();
    assert.equal(
      result.current.getReadyTags().length,
      1,
      "a fresh sendable tag lands after the refetch",
    );
    assert.ok(
      freshTag?.includes("https://relay.example.com/media/x"),
      "the sendable tag carries the freshly-fetched snapshot media",
    );
    assert.equal(result.current.hasPendingSnapshots, false);

    unmount();
  } finally {
    cleanup();
    ipcHandlers.clear();
  }
});

// A blocked re-entry can leave again before its forced refetch settles. The
// abandoned phase must not poison a later paste of the now-healthy cached result.
test("a removed blocked re-entry can later use metadata that resolved while absent", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { resetLinkPreviewMetadataCache } = await import(
    "@/shared/lib/useResolvedLinkPreviews.ts"
  );
  const { updateComposerLinkPreviewInput, useComposerLinkPreviews } =
    await import("./useComposerLinkPreviews.tsx");

  resetLinkPreviewMetadataCache();
  ipcHandlers.clear();
  ipcHandlers.set("get_relay_http_url", () =>
    Promise.resolve("https://relay.example.com"),
  );
  ipcHandlers.set("upload_media_bytes", () =>
    Promise.resolve({
      url: "https://relay.example.com/media/fresh-after-absence",
      sha256: "f8e5",
      size: 1,
      type: "image/png",
      uploaded: 0,
    }),
  );

  let fetchCalls = 0;
  let resolveRefetch;
  ipcHandlers.set("fetch_link_preview_metadata", () => {
    fetchCalls += 1;
    if (fetchCalls === 1) {
      return Promise.resolve(
        metadata({
          imageFetchState: "transient_failure",
          imageRetryAfterMs: 900_000,
        }),
      );
    }
    return new Promise((resolve) => {
      resolveRefetch = () =>
        resolve(
          metadata({
            imageDataUrl: "data:image/png;base64,QQ==",
            imageDomain: "images.example.com",
            imageFetchState: "image",
          }),
        );
    });
  });

  const settle = async () => {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, DEBOUNCE_WAIT_MS));
    });
  };

  try {
    let previewInput = updateComposerLinkPreviewInput(
      {
        content: "",
        hrefs: new Set(),
        hrefVersions: new Map(),
        nextHrefVersion: 0,
      },
      `see ${HREF}`,
      null,
    );
    const { result, rerender, unmount } = renderHook(
      ({ content, hrefVersions }) =>
        useComposerLinkPreviews(content, true, hrefVersions),
      { initialProps: previewInput },
    );

    await settle();
    assert.equal(result.current.getReadyTags().length, 1);

    // Re-enter the cached negative and wait until its forced refetch is in flight.
    await act(async () => {
      previewInput = updateComposerLinkPreviewInput(previewInput, "see ", null);
      previewInput = updateComposerLinkPreviewInput(
        previewInput,
        `see ${HREF}`,
        null,
      );
      rerender(previewInput);
    });
    await settle();
    assert.equal(fetchCalls, 2);
    assert.equal(result.current.getReadyTags().length, 0);
    assert.equal(result.current.hasPendingSnapshots, true);

    // Remove the blocked href, then let its refetch populate healthy metadata
    // while no candidate is active.
    await act(async () => {
      previewInput = updateComposerLinkPreviewInput(previewInput, "see ", null);
      rerender(previewInput);
    });
    await settle();
    await act(async () => resolveRefetch());
    await settle();
    assert.equal(result.current.getReadyTags().length, 0);

    // A later paste should use the healthy result immediately after debounce;
    // the abandoned "blocked" phase must not survive and suppress its tag.
    await act(async () => {
      previewInput = updateComposerLinkPreviewInput(
        previewInput,
        `see ${HREF}`,
        null,
      );
      rerender(previewInput);
    });
    await settle();
    const [freshTag] = result.current.getReadyTags();
    assert.equal(
      fetchCalls,
      2,
      "healthy metadata is preserved without a third fetch",
    );
    assert.equal(result.current.getReadyTags().length, 1);
    assert.ok(
      freshTag?.includes("https://relay.example.com/media/fresh-after-absence"),
      "the later paste becomes sendable with metadata resolved while absent",
    );
    assert.equal(result.current.hasPendingSnapshots, false);

    unmount();
  } finally {
    cleanup();
    ipcHandlers.clear();
  }
});

// ── Composer-hook regression: stale in-flight upload after re-entry ───────────
//
// A pre-clear transient-fallback snapshot upload (U1) can still be in flight
// when the URL is cleared and re-pasted. The re-entry forces a refetch to fresh
// metadata, and the upload effect starts a fresh upload (U2). When the stale U1
// finally settles it must NOT publish a snapshot tag built from the pre-clear
// metadata — even though its re-entry phase marker was already cleared once the
// refetch reached fresh ready. The per-href upload generation fence makes the
// stale completion a no-op, and the generation-aware dedup guard lets U2 start
// even while U1's slot is still occupied. Reverting either the generation fence
// in `.then` or the generation-aware dedup guard makes this fail (U1 publishes
// its stale favicon, or U2 never starts). (PR #5510, third follow-up.)
test("a stale in-flight upload cannot publish after the URL re-enters and a fresh upload wins", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { resetLinkPreviewMetadataCache } = await import(
    "@/shared/lib/useResolvedLinkPreviews.ts"
  );
  const { updateComposerLinkPreviewInput, useComposerLinkPreviews } =
    await import("./useComposerLinkPreviews.tsx");

  resetLinkPreviewMetadataCache();
  ipcHandlers.clear();

  ipcHandlers.set("get_relay_http_url", () =>
    Promise.resolve("https://relay.example.com"),
  );

  // Gate the FIRST media upload (U1's favicon — its image is null in the
  // transient fallback, so the favicon is U1's only upload) so it stays in
  // flight across the clear + re-paste and the fresh-metadata resolution. Every
  // later upload resolves instantly with a "fresh" URL. If a stale tag ever
  // reaches the sendable set it will carry the STALE favicon URL, which the
  // assertions forbid.
  let uploadCalls = 0;
  let releaseStaleUpload;
  ipcHandlers.set("upload_media_bytes", () => {
    uploadCalls += 1;
    if (uploadCalls === 1) {
      return new Promise((resolve) => {
        releaseStaleUpload = () =>
          resolve({
            url: "https://relay.example.com/media/STALE",
            sha256: "5741313",
            size: 1,
            type: "image/png",
            uploaded: 0,
          });
      });
    }
    return Promise.resolve({
      url: "https://relay.example.com/media/FRESH",
      sha256: "f8e5",
      size: 1,
      type: "image/png",
      uploaded: 0,
    });
  });

  // Call 1 (initial paste): transient failure WITH a favicon, so it produces a
  // sendable fallback whose upload (the favicon) is the gated U1. Call 2 (the
  // forced re-entry refetch): a full success that only resolves when released,
  // so the intermediate window (U1 in flight, refetch pending) is observable.
  let fetchCalls = 0;
  let resolveRefetch;
  ipcHandlers.set("fetch_link_preview_metadata", () => {
    fetchCalls += 1;
    if (fetchCalls === 1) {
      return Promise.resolve(
        metadata({
          faviconDataUrl: "data:image/png;base64,QQ==",
          imageFetchState: "transient_failure",
          imageRetryAfterMs: 900_000,
        }),
      );
    }
    return new Promise((resolve) => {
      resolveRefetch = () =>
        resolve(
          metadata({
            faviconDataUrl: "data:image/png;base64,Qg==",
            imageDataUrl: "data:image/png;base64,Qw==",
            imageDomain: "images.example.com",
            imageFetchState: "image",
          }),
        );
    });
  });

  const flushDebounceAndSettle = async () => {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, DEBOUNCE_WAIT_MS));
    });
  };

  try {
    let previewInput = updateComposerLinkPreviewInput(
      {
        content: "",
        hrefs: new Set(),
        hrefVersions: new Map(),
        nextHrefVersion: 0,
      },
      `see ${HREF}`,
      null,
    );
    const { result, rerender, unmount } = renderHook(
      ({ content, hrefVersions }) =>
        useComposerLinkPreviews(content, true, hrefVersions),
      { initialProps: previewInput },
    );

    // 1. Paste settles to a transient-failure fallback. Its favicon upload (U1)
    //    is gated and stays in flight, so no sendable tag exists yet.
    await flushDebounceAndSettle();
    assert.equal(fetchCalls, 1);
    assert.equal(
      uploadCalls,
      1,
      "U1 (the stale fallback favicon) is in flight",
    );
    assert.equal(
      result.current.getReadyTags().length,
      0,
      "U1 has not settled, so no snapshot tag is sendable yet",
    );

    // 2. Fast gesture: clear then re-paste the SAME URL inside the debounce.
    await act(async () => {
      previewInput = updateComposerLinkPreviewInput(previewInput, "see ", null);
      previewInput = updateComposerLinkPreviewInput(
        previewInput,
        `see ${HREF}`,
        null,
      );
      rerender(previewInput);
    });

    // 3. The re-entry forces a fresh fetch; resolve it to fresh success. The
    //    upload effect must start a FRESH upload (U2) even though U1 still holds
    //    the slot, then produce a fresh sendable tag.
    await flushDebounceAndSettle();
    assert.equal(fetchCalls, 2, "the re-entry forced a fresh fetch");
    await act(async () => {
      resolveRefetch();
    });
    await flushDebounceAndSettle();
    assert.ok(
      uploadCalls >= 2,
      "a fresh upload (U2) started despite U1 still holding the slot",
    );
    const [freshTag] = result.current.getReadyTags();
    assert.equal(
      result.current.getReadyTags().length,
      1,
      "the fresh upload produced a sendable tag",
    );
    assert.ok(
      freshTag?.includes("https://relay.example.com/media/FRESH"),
      "the sendable tag carries the freshly-uploaded media",
    );
    assert.ok(
      !freshTag?.includes("https://relay.example.com/media/STALE"),
      "the sendable tag must not carry the stale pre-clear media",
    );

    // 4. Release the stale U1. Its completion must be a no-op: it cannot
    //    overwrite the fresh tag with one built from pre-clear metadata.
    await act(async () => {
      releaseStaleUpload();
    });
    await flushDebounceAndSettle();
    const [tagAfterStale] = result.current.getReadyTags();
    assert.equal(
      result.current.getReadyTags().length,
      1,
      "still exactly one sendable tag after the stale upload settles",
    );
    assert.ok(
      tagAfterStale?.includes("https://relay.example.com/media/FRESH") &&
        !tagAfterStale?.includes("https://relay.example.com/media/STALE"),
      "the stale upload cannot publish its pre-clear tag after settling",
    );

    unmount();
  } finally {
    cleanup();
    ipcHandlers.clear();
  }
});

// The ordinary production gesture starts from a mounted empty composer. Live
// href tracking must not mark the pasted href handled before the debounced
// candidate exists, or the eventual resolver pass will reuse a cached negative.
test("composer refetches a cached negative when a link is pasted into an empty draft", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { resetLinkPreviewMetadataCache } = await import(
    "@/shared/lib/useResolvedLinkPreviews.ts"
  );
  const { useComposerLinkPreviews } = await import(
    "./useComposerLinkPreviews.tsx"
  );

  resetLinkPreviewMetadataCache();
  ipcHandlers.clear();
  ipcHandlers.set("get_relay_http_url", () =>
    Promise.resolve("https://relay.example.com"),
  );
  ipcHandlers.set("upload_media_bytes", () =>
    Promise.resolve({
      url: "https://relay.example.com/media/fresh",
      sha256: "f8e5",
      size: 1,
      type: "image/png",
      uploaded: 0,
    }),
  );

  let fetchCalls = 0;
  ipcHandlers.set("fetch_link_preview_metadata", () => {
    fetchCalls += 1;
    return Promise.resolve(
      fetchCalls === 1
        ? metadata({
            imageFetchState: "transient_failure",
            imageRetryAfterMs: 900_000,
          })
        : metadata({
            imageDataUrl: "data:image/png;base64,QQ==",
            imageDomain: "images.example.com",
            imageFetchState: "image",
          }),
    );
  });

  try {
    // Seed the shared cache with the negative result before the composer sees A.
    const { useResolvedLinkPreviews } = await import(
      "@/shared/lib/useResolvedLinkPreviews.ts"
    );
    const cached = renderHook(() =>
      useResolvedLinkPreviews([
        {
          kind: "generic-link",
          href: HREF,
          title: HREF,
          provider: "example.com",
          imageUrl: null,
        },
      ]),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 50));
    });
    assert.equal(fetchCalls, 1, "the negative was cached before paste");
    cached.unmount();

    const { result, rerender, unmount } = renderHook(
      ({ content }) => useComposerLinkPreviews(content),
      { initialProps: { content: "" } },
    );
    await act(async () => rerender({ content: `see ${HREF}` }));
    assert.equal(
      fetchCalls,
      1,
      "debounce has not resolved the pasted href yet",
    );
    assert.equal(result.current.hasPendingSnapshots, true);

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, DEBOUNCE_WAIT_MS));
    });
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 50));
    });
    assert.equal(fetchCalls, 2, "paste invalidated and refetched the negative");
    assert.equal(result.current.getReadyTags().length, 1);
    unmount();
  } finally {
    cleanup();
    ipcHandlers.clear();
  }
});
// A concurrent render may execute the hook and then suspend before commit. No
// href presence, block, generation, or tag mutation from that abandoned render
// may affect the previously committed composer.
test("an abandoned concurrent render cannot invalidate the committed snapshot tag", async () => {
  const React = await import("react");
  const { act, cleanup, render } = await import("@testing-library/react");
  const { resetLinkPreviewMetadataCache } = await import(
    "@/shared/lib/useResolvedLinkPreviews.ts"
  );
  const { useComposerLinkPreviews } = await import(
    "./useComposerLinkPreviews.tsx"
  );

  resetLinkPreviewMetadataCache();
  ipcHandlers.clear();
  ipcHandlers.set("get_relay_http_url", () =>
    Promise.resolve("https://relay.example.com"),
  );
  ipcHandlers.set("upload_media_bytes", () =>
    Promise.resolve({
      url: "https://relay.example.com/media/stable",
      sha256: "57ab1e",
      size: 1,
      type: "image/png",
      uploaded: 0,
    }),
  );
  ipcHandlers.set("fetch_link_preview_metadata", () =>
    Promise.resolve(
      metadata({
        imageFetchState: "transient_failure",
        imageRetryAfterMs: 900_000,
      }),
    ),
  );

  let latest;
  const never = new Promise(() => {});
  function Suspender() {
    throw never;
  }
  function Harness({ content, suspend }) {
    latest = useComposerLinkPreviews(content);
    return suspend ? React.createElement(Suspender) : null;
  }

  try {
    const view = render(
      React.createElement(Harness, { content: `see ${HREF}`, suspend: false }),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, DEBOUNCE_WAIT_MS));
    });
    assert.equal(latest.getReadyTags().length, 1);

    // Render a clear that executes the hook but suspends before commit, then
    // supersede it with the unchanged committed content.
    await act(async () => {
      React.startTransition(() =>
        view.rerender(
          React.createElement(Harness, { content: "", suspend: true }),
        ),
      );
      await Promise.resolve();
    });
    await act(async () => {
      view.rerender(
        React.createElement(Harness, {
          content: `see ${HREF}`,
          suspend: false,
        }),
      );
      await Promise.resolve();
    });

    assert.equal(
      latest.getReadyTags().length,
      1,
      "the committed tag remains sendable after the abandoned render",
    );
    assert.equal(latest.hasPendingSnapshots, false);
    view.unmount();
  } finally {
    cleanup();
    ipcHandlers.clear();
  }
});

// ── Composer clone-URL classification ────────────────────────────────────────
//
// A same-relay `/git/<owner>/<repo>` clone URL is a Buzz repository entity: the
// renderer normalizes it onto `buzz://repo` and shows it as an inline chip, not
// a standalone card. The composer must reach the same verdict from the same
// active relay origin — without it the URL is classified as an external
// generic-link, enters snapshot fetching, and shows a card the sent message
// then contradicts.

const CLONE_OWNER = "a".repeat(64);
const RELAY_ORIGIN = "https://relay.example.com";
const CLONE_HREF = `${RELAY_ORIGIN}/git/${CLONE_OWNER}/relay-tools.git`;

test("composer input classifies a same-relay clone URL as a Buzz entity", async () => {
  const { updateComposerLinkPreviewInput } = await import(
    "./useComposerLinkPreviews.tsx"
  );
  const empty = {
    content: "",
    hrefs: new Set(),
    hrefVersions: new Map(),
    nextHrefVersion: 0,
  };

  assert.deepEqual(
    [
      ...updateComposerLinkPreviewInput(
        empty,
        `clone ${CLONE_HREF}`,
        RELAY_ORIGIN,
      ).hrefs,
    ],
    [],
    "a same-relay clone URL is a chip-only entity, never a preview candidate",
  );
  // A different origin sharing the path shape stays an ordinary external link.
  assert.deepEqual(
    [
      ...updateComposerLinkPreviewInput(
        empty,
        `clone ${CLONE_HREF}`,
        "https://evil.example.com",
      ).hrefs,
    ],
    [CLONE_HREF],
  );
});

test("composer never fetches a snapshot for a same-relay clone URL", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { getCachedRelayOrigin } = await import("@/shared/lib/mediaUrl.ts");
  const { resetLinkPreviewMetadataCache } = await import(
    "@/shared/lib/useResolvedLinkPreviews.ts"
  );
  const { useComposerLinkPreviews } = await import(
    "./useComposerLinkPreviews.tsx"
  );

  resetLinkPreviewMetadataCache();
  ipcHandlers.clear();
  ipcHandlers.set("get_relay_http_url", () => Promise.resolve(RELAY_ORIGIN));
  let fetchCalls = 0;
  ipcHandlers.set("fetch_link_preview_metadata", () => {
    fetchCalls += 1;
    return Promise.resolve(metadata());
  });

  try {
    // The origin resolves asynchronously; the classification under test only
    // exists once it is known, so wait for the shared cache to publish it.
    const deadline = Date.now() + 5000;
    while (getCachedRelayOrigin() !== RELAY_ORIGIN && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 25));
    }
    assert.equal(getCachedRelayOrigin(), RELAY_ORIGIN);

    const { result, unmount } = renderHook(() =>
      useComposerLinkPreviews(`clone ${CLONE_HREF}`),
    );
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, DEBOUNCE_WAIT_MS));
    });
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 50));
    });

    assert.equal(
      fetchCalls,
      0,
      "a Buzz repository entity must not enter external snapshot fetching",
    );
    assert.equal(result.current.previewList, null);
    assert.deepEqual(result.current.getReadyTags(), []);
    assert.deepEqual(result.current.getLiveCandidates(), []);
    assert.equal(result.current.hasPendingSnapshots, false);
    unmount();
  } finally {
    cleanup();
    ipcHandlers.clear();
  }
});
