import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

import {
  __linkPreviewMetadataTest,
  fetchBuzzEntityMetadata,
  isBuzzEntityPreview,
  resetLinkPreviewMetadataCache,
  resolveLinkPreview,
  withEntityFallbacks,
} from "./useResolvedLinkPreviews.ts";

const preview = {
  kind: "generic-link",
  href: "https://example.com/story",
  provider: "example.com",
  title: "example.com/story",
  typeLabel: "link",
};

function metadata(overrides = {}) {
  return {
    title: "A story",
    siteName: "Example",
    description: "Story description",
    imageDataUrl: null,
    imageDomain: null,
    imageFetchState: "none",
    imageRetryAfterMs: null,
    ...overrides,
  };
}

test("pending external metadata reserves the image treatment", () => {
  assert.deepEqual(resolveLinkPreview(preview, undefined), {
    ...preview,
    imageState: "pending",
  });
});

test("pending Buzz entity metadata remains image-less", () => {
  const entityPreview = {
    kind: "buzz-repository",
    href: `buzz://repo?owner=${"cd".repeat(32)}&d=buzz`,
    provider: "Buzz",
    title: "buzz",
    typeLabel: "repo",
  };
  assert.deepEqual(resolveLinkPreview(entityPreview, undefined), {
    ...entityPreview,
    imageState: "none",
  });
});

test("resolved image metadata keeps the reserved image treatment", () => {
  const resolved = resolveLinkPreview(preview, {
    title: "A story",
    siteName: "Example",
    imageDataUrl: "data:image/jpeg;base64,abc",
    imageDomain: "cdn.example.com",
  });

  assert.equal(resolved.imageState, "image");
  assert.equal(resolved.provider, "Example");
  assert.equal(resolved.imageDomain, "cdn.example.com");
});

test("resolved metadata without a complete image collapses to the compact treatment", () => {
  const resolved = resolveLinkPreview(preview, {
    title: "A story",
    siteName: "Example",
    imageDataUrl: null,
    imageDomain: null,
  });

  assert.equal(resolved.imageState, "none");
  assert.equal(resolved.imageDataUrl, null);
  assert.equal(resolved.imageDomain, null);
});

test("transient and rejected image fetches use the stable fallback treatment", () => {
  const transient = resolveLinkPreview(
    preview,
    metadata({
      imageFetchState: "transient_failure",
      imageRetryAfterMs: 900_000,
    }),
  );
  const rejected = resolveLinkPreview(
    preview,
    metadata({ imageFetchState: "rejected" }),
  );

  assert.equal(transient.imageState, "fallback");
  assert.equal(rejected.imageState, "fallback");
});

test("metadata cache keys deduplicate URL fragments", () => {
  assert.equal(
    __linkPreviewMetadataTest.metadataCacheKey(
      "https://github.com/block/buzz/pull/3834#issuecomment-1",
    ),
    "https://github.com/block/buzz/pull/3834",
  );
});

test("transient metadata expires at the server retry boundary", () => {
  assert.equal(
    __linkPreviewMetadataTest.metadataExpiry(
      metadata({
        imageFetchState: "transient_failure",
        imageRetryAfterMs: 900_000,
      }),
      1_000,
    ),
    901_000,
  );
});

test("metadata loader retries transient images after the server cooldown", async () => {
  let now = 1_000;
  let calls = 0;
  const loader = __linkPreviewMetadataTest.createMetadataLoader({
    fetcher: async () => {
      calls += 1;
      return calls === 1
        ? metadata({
            imageFetchState: "transient_failure",
            imageRetryAfterMs: 10_000,
          })
        : metadata({
            imageDataUrl: "data:image/jpeg;base64,abc",
            imageDomain: "images.example.com",
            imageFetchState: "image",
          });
    },
    now: () => now,
  });

  assert.equal(
    (await loader.load(preview.href)).metadata?.imageFetchState,
    "transient_failure",
  );
  assert.equal(calls, 1);

  now += 10_000;
  assert.equal(
    (await loader.load(preview.href)).metadata?.imageFetchState,
    "image",
  );
  assert.equal(calls, 2);
});

test("metadata loader retries rejected requests after the negative-cache TTL", async () => {
  let now = 1_000;
  let calls = 0;
  const loader = __linkPreviewMetadataTest.createMetadataLoader({
    fetcher: async () => {
      calls += 1;
      if (calls === 1) throw new Error("temporary failure");
      return metadata();
    },
    now: () => now,
  });

  assert.equal((await loader.load(preview.href)).metadata, null);
  assert.equal((await loader.load(preview.href)).metadata, null);
  assert.equal(calls, 1);

  now += 5 * 60_000;
  assert.deepEqual((await loader.load(preview.href)).metadata, metadata());
  assert.equal(calls, 2);
});

test("metadata loader coalesces fragment variants and bounds concurrency", async () => {
  let active = 0;
  let maxActive = 0;
  let calls = 0;
  const loader = __linkPreviewMetadataTest.createMetadataLoader({
    concurrency: 2,
    fetcher: async () => {
      calls += 1;
      active += 1;
      maxActive = Math.max(maxActive, active);
      await new Promise((resolve) => setImmediate(resolve));
      active -= 1;
      return metadata();
    },
  });

  await Promise.all([
    loader.load("https://example.com/one#first"),
    loader.load("https://example.com/one#second"),
    loader.load("https://example.com/two"),
    loader.load("https://example.com/three"),
  ]);

  assert.equal(calls, 3);
  assert.equal(maxActive, 2);
});

test("withEntityFallbacks re-adds previews dropped by null metadata", () => {
  const entityPreview = {
    kind: "buzz-pull-request",
    href: `buzz://pr?id=${"ab".repeat(32)}&owner=${"cd".repeat(32)}&d=buzz`,
    provider: "Buzz",
    title: `buzz #${"ab".repeat(4)}`,
    typeLabel: "PR",
  };

  assert.deepEqual(withEntityFallbacks([entityPreview], []), [
    { ...entityPreview, imageState: "none" },
  ]);
});

test("withEntityFallbacks keeps resolved previews and preserves order", () => {
  const first = {
    kind: "buzz-repository",
    href: `buzz://repo?owner=${"cd".repeat(32)}&d=buzz`,
    provider: "Buzz",
    title: "buzz",
    typeLabel: "repo",
  };
  const second = {
    kind: "buzz-issue",
    href: `buzz://issue?id=${"ef".repeat(32)}&owner=${"cd".repeat(32)}&d=buzz`,
    provider: "Buzz",
    title: `buzz #${"ef".repeat(4)}`,
    typeLabel: "issue",
  };
  const resolvedSecond = {
    ...second,
    title: "Fix the preview cards",
    imageState: "none",
  };

  assert.deepEqual(withEntityFallbacks([first, second], [resolvedSecond]), [
    { ...first, imageState: "none" },
    resolvedSecond,
  ]);
});

test("entity fallback eligibility is kind-scoped", () => {
  assert.equal(
    isBuzzEntityPreview({
      ...preview,
      kind: "buzz-repository",
      href: `buzz://repo?owner=${"cd".repeat(32)}&d=buzz`,
    }),
    true,
  );
  assert.equal(
    isBuzzEntityPreview({ ...preview, href: "buzz://future?id=example" }),
    false,
  );
});

test("withEntityFallbacks still drops unresolved external links", () => {
  assert.deepEqual(withEntityFallbacks([preview], []), []);
  assert.deepEqual(
    withEntityFallbacks([{ ...preview, href: "buzz://future?id=example" }], []),
    [],
  );
});

function relayEvent({
  id,
  kind,
  pubkey,
  content = "",
  tags = [],
  createdAt = 1,
}) {
  return { id, kind, pubkey, created_at: createdAt, content, tags, sig: "" };
}

test("Buzz PR metadata includes repository identity and trusted root context", async () => {
  const owner = "cd".repeat(32);
  const attacker = "ef".repeat(32);
  const id = "ab".repeat(32);
  const repoAddress = `30617:${owner}:buzz`;
  const commit = "1234567".padEnd(40, "0");
  const events = [
    relayEvent({
      id: "01".repeat(32),
      kind: 30617,
      pubkey: owner,
      tags: [
        ["d", "buzz"],
        ["name", "Buzz Desktop"],
        ["default-branch", "main"],
      ],
    }),
    relayEvent({
      id,
      kind: 1618,
      pubkey: owner,
      content: "Body",
      tags: [
        ["a", repoAddress],
        ["subject", "Restore entity cards"],
        ["branch-name", "fix/cards"],
        ["target-branch", "release"],
        ["c", commit],
      ],
    }),
    relayEvent({
      id: "02".repeat(32),
      kind: 1633,
      pubkey: attacker,
      createdAt: 20,
      tags: [["e", id]],
    }),
    relayEvent({
      id: "03".repeat(32),
      kind: 1630,
      pubkey: owner,
      createdAt: 10,
      tags: [["e", id]],
    }),
    ...Array.from({ length: 25 }, (_, index) =>
      relayEvent({
        id: index.toString(16).padStart(64, "0"),
        kind: 1633,
        pubkey: owner,
        createdAt: 100 + index,
        tags: [["e", index.toString(16).padStart(64, "f")]],
      }),
    ),
  ];
  const fetchEvents = async (filter) =>
    events
      .filter(
        (event) =>
          (!filter.kinds || filter.kinds.includes(event.kind)) &&
          (!filter.ids || filter.ids.includes(event.id)) &&
          (!filter.authors || filter.authors.includes(event.pubkey)) &&
          (!filter["#d"] ||
            event.tags.some(
              (tag) => tag[0] === "d" && filter["#d"].includes(tag[1]),
            )) &&
          (!filter["#a"] ||
            event.tags.some(
              (tag) => tag[0] === "a" && filter["#a"].includes(tag[1]),
            )) &&
          (!filter["#e"] ||
            event.tags.some(
              (tag) => tag[0] === "e" && filter["#e"].includes(tag[1]),
            )),
      )
      .sort((left, right) => right.created_at - left.created_at)
      .slice(0, filter.limit);

  const result = await fetchBuzzEntityMetadata(
    `buzz://pr?id=${id}&owner=${owner}&d=buzz`,
    fetchEvents,
  );
  assert.equal(result?.siteName, "Buzz Desktop");
  assert.equal(result?.title, "Restore entity cards");
  assert.equal(result?.description, "Open · fix/cards → release · 1234567");
  assert.equal(result?.faviconDataUrl, null);
  assert.equal(result?.imageDataUrl, null);
});

test("Buzz entity roots reject ambiguous repository tags", async () => {
  const owner = "cd".repeat(32);
  const attacker = "ef".repeat(32);
  const targetAddress = `30617:${owner}:buzz`;
  const attackerAddress = `30617:${attacker}:other`;
  const repository = relayEvent({
    id: "01".repeat(32),
    kind: 30617,
    pubkey: owner,
    tags: [
      ["d", "buzz"],
      ["name", "Buzz Desktop"],
      ["default-branch", "main"],
    ],
  });

  for (const [type, kind] of [
    ["pr", 1618],
    ["issue", 1621],
  ]) {
    const id = (type === "pr" ? "ab" : "bc").repeat(32);
    const root = relayEvent({
      id,
      kind,
      pubkey: attacker,
      tags: [
        ["a", attackerAddress],
        ["a", targetAddress],
        ["subject", "Misbound entity"],
      ],
    });
    const result = await fetchBuzzEntityMetadata(
      `buzz://${type}?id=${id}&owner=${owner}&d=buzz`,
      async (filter) =>
        filter.kinds?.includes(30617)
          ? [repository]
          : filter.ids?.includes(id)
            ? [root]
            : [],
    );
    assert.equal(result, null, `${type} with multiple repository tags`);
  }
});

test("Buzz repository metadata stays image-less and exposes default branch", async () => {
  const owner = "cd".repeat(32);
  const result = await fetchBuzzEntityMetadata(
    `buzz://repo?owner=${owner}&d=relay-tools`,
    async () => [
      relayEvent({
        id: "01".repeat(32),
        kind: 30617,
        pubkey: owner,
        content: "Fallback description",
        tags: [
          ["d", "relay-tools"],
          ["name", "Relay Tools"],
          ["description", "Operator tooling for relays"],
          ["status", "active"],
          ["default-branch", "trunk"],
        ],
      }),
    ],
  );
  assert.equal(result?.siteName, "Relay Tools");
  assert.equal(result?.title, "Operator tooling for relays");
  assert.equal(result?.description, "active · default: trunk");
  assert.equal(result?.faviconDataUrl, null);
  assert.equal(result?.imageDataUrl, null);
  assert.equal(result?.imageDomain, null);
});

test("Buzz project metadata resolves from the 30621 announcement", async () => {
  const owner = "cd".repeat(32);
  const result = await fetchBuzzEntityMetadata(
    `buzz://project?owner=${owner}&d=pollinator`,
    async () => [
      relayEvent({
        id: "01".repeat(32),
        kind: 30621,
        pubkey: owner,
        tags: [
          ["d", "pollinator"],
          ["name", "Pollinator"],
          ["description", "Cross-repo pollination pipeline"],
          ["a", `30617:${owner}:flappy-bee`],
          ["a", `30617:${owner}:hive-tools`],
        ],
      }),
    ],
  );
  assert.equal(result?.siteName, "Pollinator");
  assert.equal(result?.title, "Cross-repo pollination pipeline");
  assert.equal(result?.description, "2 repositories");
  assert.equal(result?.imageDataUrl, null);
});

test("Buzz project metadata declines a missing or invalid announcement", async () => {
  const owner = "cd".repeat(32);
  assert.equal(
    await fetchBuzzEntityMetadata(
      `buzz://project?owner=${owner}&d=pollinator`,
      async () => [],
    ),
    null,
  );
  // Two `d` tags fail NIP-MP envelope validation.
  assert.equal(
    await fetchBuzzEntityMetadata(
      `buzz://project?owner=${owner}&d=pollinator`,
      async () => [
        relayEvent({
          id: "01".repeat(32),
          kind: 30621,
          pubkey: owner,
          tags: [
            ["d", "pollinator"],
            ["d", "duplicate"],
          ],
        }),
      ],
    ),
    null,
  );
});

test("invalidateNegative drops a cached null miss so the next load refetches", async () => {
  // A URL freshly entering the composer clears a stale hard miss (null) so it
  // refetches, instead of riding the cached blank.
  const now = 1_000;
  let calls = 0;
  let nextResult = null;
  const loader = __linkPreviewMetadataTest.createMetadataLoader({
    fetcher: async () => {
      calls += 1;
      return nextResult;
    },
    now: () => now,
  });

  assert.equal((await loader.load(preview.href)).metadata, null);
  assert.equal(calls, 1);

  loader.invalidateNegative(preview.href);
  nextResult = metadata();
  assert.deepEqual((await loader.load(preview.href)).metadata, metadata());
  assert.equal(calls, 2);
});

test("invalidateNegative drops a cached transient failure so the next load refetches", async () => {
  // A transient image failure is a NEGATIVE by contract (option docs + the
  // loader's own retry boundary), so re-entering the composer must refetch it
  // — not reuse the cached transient entry. Regression for the leak where
  // invalidateNegative only cleared hard `null` misses. (PR #5510)
  const now = 1_000;
  let calls = 0;
  const loader = __linkPreviewMetadataTest.createMetadataLoader({
    fetcher: async () => {
      calls += 1;
      return calls === 1
        ? metadata({
            imageFetchState: "transient_failure",
            imageRetryAfterMs: 10_000,
          })
        : metadata({
            imageDataUrl: "data:image/jpeg;base64,abc",
            imageDomain: "images.example.com",
            imageFetchState: "image",
          });
    },
    now: () => now,
  });

  assert.equal(
    (await loader.load(preview.href)).metadata?.imageFetchState,
    "transient_failure",
  );
  assert.equal(calls, 1);

  // Bust well before the retry boundary (now is frozen); the cache-bust — not
  // the cooldown — is what forces the refetch.
  loader.invalidateNegative(preview.href);
  assert.equal(
    (await loader.load(preview.href)).metadata?.imageFetchState,
    "image",
  );
  assert.equal(calls, 2);
});

test("invalidateNegative leaves a healthy cached hit untouched", async () => {
  // A settled positive (instant card, no redundant fetch) must survive a bust
  // so passive scroll re-renders keep riding the cache.
  const now = 1_000;
  let calls = 0;
  const loader = __linkPreviewMetadataTest.createMetadataLoader({
    fetcher: async () => {
      calls += 1;
      return metadata();
    },
    now: () => now,
  });

  assert.deepEqual((await loader.load(preview.href)).metadata, metadata());
  assert.equal(calls, 1);

  loader.invalidateNegative(preview.href);
  assert.deepEqual((await loader.load(preview.href)).metadata, metadata());
  assert.equal(calls, 1);
});

test("invalidateNegative leaves an in-flight fetch untouched", async () => {
  // A fetch still in flight is cached as a Promise, not a resolved entry.
  // Busting mid-flight must not cancel or duplicate it: the pending load
  // resolves normally and no second fetch is started.
  const now = 1_000;
  let calls = 0;
  let releaseFetch;
  const loader = __linkPreviewMetadataTest.createMetadataLoader({
    fetcher: () => {
      calls += 1;
      return new Promise((resolve) => {
        releaseFetch = () => resolve(metadata());
      });
    },
    now: () => now,
  });

  const pending = loader.load(preview.href);
  assert.equal(calls, 1);

  // Bust while the fetch is still in flight — the Promise entry is left alone.
  loader.invalidateNegative(preview.href);
  assert.equal(calls, 1, "no redundant fetch started by the bust");

  releaseFetch();
  assert.deepEqual((await pending).metadata, metadata());
  assert.equal(calls, 1, "the original in-flight fetch resolved, not a retry");
});

// ── Hook-level regression: retained resolved-state invalidation on re-entry ───
//
// The loader tests above prove `invalidateNegative` drops the SHARED loader
// cache entry. But `useResolvedLinkPreviews` also retains its OWN
// `resolvedMetadata` React state, and the render that scheduled the
// invalidating effect has already read the stale negative from it. Dropping the
// loader key alone leaves that local key in place, so a re-entered
// `transient_failure` still resolves to a `snapshotReady` fallback the composer
// can turn into a sendable snapshot tag from STALE metadata before the retry
// lands. This drives the REAL hook to prove the local key is cleared too, so
// the re-entry renders as pending (no `snapshotReady`) until the retry wins.
// (PR #5510) Regression: without the local-state clear the re-entry assertion
// below sees `snapshotReady: true` instead of pending.

const dom = new JSDOM("<!doctype html><html><body></body></html>", {
  url: "http://localhost",
});

before(() => {
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  // @tauri-apps/api/core reads window.__TAURI_INTERNALS__.invoke at call time.
  // A per-test handler map lets each test control the fetch resolution timing.
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

/** @type {Map<string, (args: unknown) => Promise<unknown>>} */
const ipcHandlers = new Map();

const hookPreview = {
  kind: "generic-link",
  href: "https://example.com/hook-re-entry",
  provider: "example.com",
  title: "example.com/hook-re-entry",
  typeLabel: "link",
};

test("hook clears retained resolved state so a re-entered transient failure renders pending until the retry wins", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { useResolvedLinkPreviews } = await import(
    "./useResolvedLinkPreviews.ts"
  );

  // Isolate from any loader state leaked by earlier tests in this process.
  resetLinkPreviewMetadataCache();
  ipcHandlers.clear();

  // Gate every fetch so we can observe the state between renders. Call 1 caches
  // a transient failure; call 2 (the re-entry retry) succeeds.
  let calls = 0;
  const releases = [];
  ipcHandlers.set("fetch_link_preview_metadata", () => {
    calls += 1;
    const attempt = calls;
    return new Promise((resolve) => {
      releases.push(() =>
        resolve(
          attempt === 1
            ? metadata({
                imageFetchState: "transient_failure",
                imageRetryAfterMs: 900_000,
              })
            : metadata({
                imageDataUrl: "data:image/jpeg;base64,abc",
                imageDomain: "images.example.com",
                imageFetchState: "image",
              }),
        ),
      );
    });
  });

  // scheduleAfterPaint uses requestAnimationFrame -> setTimeout(0); flush both
  // plus a microtask turn so the queued load() actually fires.
  const flushScheduledLoads = async () => {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  };

  const settle = async () => {
    await act(async () => {
      // Release any pending fetch and let its .then() commit setResolvedMetadata.
      while (releases.length > 0) releases.shift()();
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  };

  try {
    const { result, rerender, unmount } = renderHook(
      ({ previews }) =>
        useResolvedLinkPreviews(previews, { refetchNewNegatives: true }),
      { initialProps: { previews: [hookPreview] } },
    );

    // 1. Initial paste: pending until the fetch resolves to a transient failure.
    assert.equal(result.current[0].imageState, "pending");
    assert.equal(result.current[0].snapshotReady, undefined);

    await flushScheduledLoads();
    await settle();

    // Transient failure cached: a sendable fallback (this is the state that must
    // NOT survive re-entry as ready).
    assert.equal(result.current[0].imageState, "fallback");
    assert.equal(result.current[0].snapshotReady, true);
    assert.equal(calls, 1);

    // 2. URL removed from the composer.
    rerender({ previews: [] });
    assert.deepEqual(result.current, []);

    // 3. URL re-entered. The invalidation effect must drop BOTH the loader entry
    //    and the retained local key, so this settles to pending with no
    //    snapshotReady — not the stale fallback. Without the local-state clear
    //    the hook returns snapshotReady: true here.
    rerender({ previews: [hookPreview] });
    await act(async () => {});
    assert.equal(
      result.current[0].imageState,
      "pending",
      "re-entered transient failure must render pending, not a stale fallback",
    );
    assert.equal(
      result.current[0].snapshotReady,
      undefined,
      "no snapshotReady before the retry resolves — nothing sendable from stale metadata",
    );

    // 4. Successful retry wins.
    await flushScheduledLoads();
    await settle();
    assert.equal(result.current[0].imageState, "image");
    assert.equal(result.current[0].snapshotReady, true);
    assert.equal(calls, 2, "the re-entry triggered a fresh fetch");

    unmount();
  } finally {
    cleanup();
    ipcHandlers.clear();
  }
});

// ── Hook-level regression: in-flight shared entry defeats the local clear ─────
//
// The shared metadataLoader is intentionally shared across every hook instance
// (composer + message list). `invalidateNegative` deliberately leaves an
// in-flight Promise entry alone — but that means when a URL re-enters the
// composer WHILE another instance has a fetch in flight for the same canonical
// URL, the shared drop is a no-op. Gating the local-state clear on that drop
// (the earlier fix) leaves this hook's retained `transient_failure` in place,
// so it keeps returning `snapshotReady: true` — a sendable tag built from stale
// metadata — until that shared fetch resolves. This drives the real hook
// through that exact interleaving to prove the local negative is cleared on
// re-entry regardless of the shared entry's shape. (PR #5510, follow-up.)

const otherHookPreview = {
  kind: "generic-link",
  href: "https://example.com/hook-inflight-re-entry",
  provider: "example.com",
  title: "example.com/hook-inflight-re-entry",
  typeLabel: "link",
};

test("hook clears a re-entered negative even when the shared cache holds an in-flight fetch, not a settled entry", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { useResolvedLinkPreviews } = await import(
    "./useResolvedLinkPreviews.ts"
  );

  resetLinkPreviewMetadataCache();
  ipcHandlers.clear();

  // Gate every fetch. Call 1 (composer's initial paste) resolves to a transient
  // failure. Call 2 is the SHARED in-flight fetch a message-list instance starts
  // after the shared cache is dropped; it is left unreleased so it is still a
  // Promise in the shared cache when the composer re-enters.
  let calls = 0;
  const releases = [];
  ipcHandlers.set("fetch_link_preview_metadata", () => {
    calls += 1;
    const attempt = calls;
    return new Promise((resolve) => {
      releases.push(() =>
        resolve(
          attempt === 1
            ? metadata({
                imageFetchState: "transient_failure",
                imageRetryAfterMs: 900_000,
              })
            : metadata({
                imageDataUrl: "data:image/jpeg;base64,abc",
                imageDomain: "images.example.com",
                imageFetchState: "image",
              }),
        ),
      );
    });
  });

  const flushScheduledLoads = async () => {
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  };

  const releaseAll = async () => {
    await act(async () => {
      while (releases.length > 0) releases.shift()();
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  };

  const composer = renderHook(
    ({ previews }) =>
      useResolvedLinkPreviews(previews, { refetchNewNegatives: true }),
    { initialProps: { previews: [otherHookPreview] } },
  );
  // A passive message-list instance sharing the same loader (no refetch).
  const messageList = renderHook(
    ({ previews }) => useResolvedLinkPreviews(previews, {}),
    { initialProps: { previews: [] } },
  );

  try {
    // 1. Composer paste resolves to a transient failure -> sendable fallback.
    await flushScheduledLoads();
    await act(async () => {
      releases.shift()(); // release call 1 only
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    assert.equal(composer.result.current[0].imageState, "fallback");
    assert.equal(composer.result.current[0].snapshotReady, true);
    assert.equal(calls, 1);

    // 2. Composer URL leaves.
    composer.rerender({ previews: [] });
    assert.deepEqual(composer.result.current, []);

    // 3. The shared negative is dropped (simulating expiry/eviction), then a
    //    message-list instance starts a fresh fetch for the SAME canonical URL
    //    and it is left IN FLIGHT — a Promise, not a settled entry, in the
    //    shared cache.
    act(() => {
      resetLinkPreviewMetadataCache();
    });
    messageList.rerender({ previews: [otherHookPreview] });
    await flushScheduledLoads();
    assert.equal(calls, 2, "message-list started the shared in-flight fetch");

    // 4. Composer re-enters WHILE that fetch is in flight. invalidateNegative is
    //    a no-op (the shared entry is a Promise), so the earlier drop-gated clear
    //    would leave the retained transient failure — a sendable tag from stale
    //    metadata. The local negative must be cleared regardless: pending, no
    //    snapshotReady.
    composer.rerender({ previews: [otherHookPreview] });
    await act(async () => {});
    assert.equal(
      composer.result.current[0].imageState,
      "pending",
      "re-entry during a shared in-flight fetch must render pending, not a stale fallback",
    );
    assert.equal(
      composer.result.current[0].snapshotReady,
      undefined,
      "no snapshotReady while the shared fetch is in flight — nothing sendable from stale metadata",
    );
    // The composer coalesced onto the in-flight fetch — no third request.
    assert.equal(calls, 2, "re-entry coalesced onto the in-flight fetch");

    // 5. The shared fetch resolves successfully; both instances settle.
    await releaseAll();
    await flushScheduledLoads();
    assert.equal(composer.result.current[0].imageState, "image");
    assert.equal(composer.result.current[0].snapshotReady, true);
  } finally {
    composer.unmount();
    messageList.unmount();
    cleanup();
    ipcHandlers.clear();
  }
});
