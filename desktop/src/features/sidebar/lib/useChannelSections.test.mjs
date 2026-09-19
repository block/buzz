import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { JSDOM } from "jsdom";

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
});

after(() => dom.window.close());

async function withMockedRelay(run) {
  const { relayClient } = await import("@/shared/api/relayClient");
  const originalFetchEvents = relayClient.fetchEvents;
  const originalSubscribeLive = relayClient.subscribeLive;
  const originalSubscribeToReconnects = relayClient.subscribeToReconnects;
  const originalPublishEvent = relayClient.publishEvent;
  const publishCalls = [];
  relayClient.fetchEvents = async () => [];
  relayClient.subscribeLive = async () => async () => {};
  relayClient.subscribeToReconnects = () => () => {};
  relayClient.publishEvent = async (...args) => {
    publishCalls.push(args);
  };
  try {
    return await run({ publishCalls, relayClient });
  } finally {
    relayClient.fetchEvents = originalFetchEvents;
    relayClient.subscribeLive = originalSubscribeLive;
    relayClient.subscribeToReconnects = originalSubscribeToReconnects;
    relayClient.publishEvent = originalPublishEvent;
  }
}

test("assignChannel refreshes an existing assignment before the next eviction", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { MAX_CHANNEL_SECTION_ASSIGNMENTS, storageKey } = await import(
    "./channelSectionsStorage.ts"
  );
  const { useChannelSections } = await import("./useChannelSections.ts");

  await withMockedRelay(async () => {
    const pubkey = "pk-at-capacity";
    const relayUrl = "wss://relay.example";
    const assignments = Object.fromEntries(
      Array.from({ length: MAX_CHANNEL_SECTION_ASSIGNMENTS }, (_, index) => [
        `chan-${String(index).padStart(4, "0")}`,
        "section-1",
      ]),
    );
    window.localStorage.setItem(
      storageKey(pubkey, relayUrl),
      JSON.stringify({
        version: 1,
        sections: [
          { id: "section-1", name: "One", order: 0 },
          { id: "section-2", name: "Two", order: 1 },
        ],
        assignments,
      }),
    );

    try {
      const { result, unmount } = renderHook(() =>
        useChannelSections(pubkey, relayUrl),
      );

      act(() => result.current.assignChannel("chan-0000", "section-2"));
      act(() => result.current.assignChannel("chan-new", "section-1"));

      assert.equal(result.current.assignments["chan-0000"], "section-2");
      assert.equal(result.current.assignments["chan-new"], "section-1");
      assert.equal(result.current.assignments["chan-0001"], undefined);
      assert.equal(
        Object.keys(result.current.assignments).length,
        MAX_CHANNEL_SECTION_ASSIGNMENTS,
      );
      unmount();
    } finally {
      cleanup();
    }
  });
});

// Regression for #7207: switching the active relay must not persist the
// previous community's section assignment map under the new community's key,
// and must not queue a publish payload containing foreign channel ids.
test("relayUrl switch resets in-memory store and never writes prior community assignments into the new scope", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { storageKey } = await import("./channelSectionsStorage.ts");
  const { ChannelSectionSyncManager } = await import(
    "./channelSectionsSync.ts"
  );
  const { useChannelSections } = await import("./useChannelSections.ts");

  await withMockedRelay(async () => {
    const pubkey = "pk-7207";
    const relayB = "wss://relay-b.example";
    const relayA = "wss://relay-a.example";
    const sectionB = { id: "section-b", name: "From B", order: 0 };
    const channelOnlyOnB = "channel-only-on-b";
    const queuedPublishes = [];
    const originalPublishSections =
      ChannelSectionSyncManager.prototype.publishSections;
    ChannelSectionSyncManager.prototype.publishSections =
      function publishSections(store) {
        queuedPublishes.push({
          relayUrl: this.relayUrl,
          assignments: { ...store.assignments },
          sectionIds: store.sections.map((section) => section.id),
        });
        return originalPublishSections.call(this, store);
      };

    window.localStorage.setItem(
      storageKey(pubkey, relayB),
      JSON.stringify({
        version: 1,
        sections: [sectionB],
        assignments: { [channelOnlyOnB]: sectionB.id },
      }),
    );
    window.localStorage.setItem(
      storageKey(pubkey, relayA),
      JSON.stringify({
        version: 1,
        sections: [],
        assignments: {},
      }),
    );

    try {
      const { result, rerender, unmount } = renderHook(
        ({ relayUrl, known, ready }) =>
          useChannelSections(pubkey, relayUrl, known, ready),
        {
          initialProps: {
            relayUrl: relayB,
            known: new Set([channelOnlyOnB]),
            ready: true,
          },
        },
      );

      assert.equal(result.current.assignments[channelOnlyOnB], sectionB.id);
      assert.equal(result.current.sections[0]?.id, sectionB.id);

      // Switch to community A. The same-render scope reset must drop B's
      // layout before any A-scoped persist/publish can see it.
      act(() => {
        rerender({
          relayUrl: relayA,
          known: new Set(["channel-only-on-a"]),
          ready: true,
        });
      });

      assert.deepEqual(result.current.assignments, {});
      assert.deepEqual(result.current.sections, []);

      const rawA = window.localStorage.getItem(storageKey(pubkey, relayA));
      assert.ok(rawA);
      const parsedA = JSON.parse(rawA);
      assert.equal(parsedA.assignments[channelOnlyOnB], undefined);
      assert.deepEqual(parsedA.assignments, {});

      const publishesBeforeMutation = queuedPublishes.length;

      // A post-switch mutation must only persist/publish A's known channels.
      act(() => {
        const section = result.current.createSection("A section");
        assert.ok(section);
        result.current.assignChannel(channelOnlyOnB, section.id);
        result.current.assignChannel("channel-only-on-a", section.id);
      });

      assert.equal(result.current.assignments[channelOnlyOnB], undefined);
      assert.equal(
        result.current.assignments["channel-only-on-a"],
        result.current.sections[0]?.id,
      );

      const rawAAfter = JSON.parse(
        window.localStorage.getItem(storageKey(pubkey, relayA)),
      );
      assert.equal(rawAAfter.assignments[channelOnlyOnB], undefined);
      assert.equal(
        rawAAfter.assignments["channel-only-on-a"],
        result.current.sections[0]?.id,
      );

      // B's scoped store must remain untouched by the A-side edits.
      const rawB = JSON.parse(
        window.localStorage.getItem(storageKey(pubkey, relayB)),
      );
      assert.equal(rawB.assignments[channelOnlyOnB], sectionB.id);

      const publishesForA = queuedPublishes.slice(publishesBeforeMutation);
      assert.ok(
        publishesForA.length > 0,
        "post-switch mutations must queue at least one publish",
      );
      for (const publish of publishesForA) {
        assert.equal(publish.relayUrl, relayA);
        assert.equal(
          publish.assignments[channelOnlyOnB],
          undefined,
          "publish queued for A must not carry B-only channel ids",
        );
        assert.ok(
          !publish.sectionIds.includes(sectionB.id),
          "publish queued for A must not carry B-only section ids",
        );
      }

      unmount();
    } finally {
      ChannelSectionSyncManager.prototype.publishSections =
        originalPublishSections;
      cleanup();
      window.localStorage.clear();
    }
  });
});

test("defers bootstrap seed-publish until channelsReady and scopes the seeded blob", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { storageKey } = await import("./channelSectionsStorage.ts");
  const { ChannelSectionSyncManager } = await import(
    "./channelSectionsSync.ts"
  );
  const { useChannelSections } = await import("./useChannelSections.ts");

  await withMockedRelay(async () => {
    const pubkey = "pk-7207-seed";
    const relayUrl = "wss://relay-a.example";
    const foreignChannel = "channel-only-on-b";
    const localChannel = "channel-only-on-a";
    const foreignSection = { id: "section-from-b", name: "From B", order: 0 };
    const bootstrapCalls = [];
    const queuedPublishes = [];
    const originalBootstrap = ChannelSectionSyncManager.prototype.bootstrap;
    const originalPublishSections =
      ChannelSectionSyncManager.prototype.publishSections;
    ChannelSectionSyncManager.prototype.bootstrap = async function bootstrap(
      localStore,
    ) {
      bootstrapCalls.push({
        relayUrl: this.relayUrl,
        assignments: { ...localStore.assignments },
        sectionIds: localStore.sections.map((section) => section.id),
      });
      return originalBootstrap.call(this, localStore);
    };
    ChannelSectionSyncManager.prototype.publishSections =
      function publishSections(store) {
        queuedPublishes.push({
          assignments: { ...store.assignments },
          sectionIds: store.sections.map((section) => section.id),
        });
        return originalPublishSections.call(this, store);
      };

    // Polluted A local (the issue's post-contamination state) with zero
    // watermark so first-sync would otherwise seed-publish the dirty blob.
    window.localStorage.setItem(
      storageKey(pubkey, relayUrl),
      JSON.stringify({
        version: 1,
        sections: [foreignSection],
        assignments: { [foreignChannel]: foreignSection.id },
      }),
    );

    try {
      const { result, rerender, unmount } = renderHook(
        ({ known, ready }) =>
          useChannelSections(pubkey, relayUrl, known, ready),
        {
          initialProps: {
            known: new Set(),
            ready: false,
          },
        },
      );

      await act(async () => {
        await Promise.resolve();
      });
      assert.equal(
        bootstrapCalls.length,
        0,
        "bootstrap must wait until channelsReady",
      );

      act(() => {
        rerender({ known: new Set([localChannel]), ready: true });
      });
      await act(async () => {
        await Promise.resolve();
        await Promise.resolve();
      });

      assert.ok(
        bootstrapCalls.length >= 1,
        "bootstrap runs once allowlist ready",
      );
      for (const call of bootstrapCalls) {
        assert.equal(call.assignments[foreignChannel], undefined);
        assert.ok(!call.sectionIds.includes(foreignSection.id));
      }
      // Display and healed storage drop the foreign layout.
      assert.deepEqual(result.current.assignments, {});
      assert.ok(
        !result.current.sections.some(
          (section) => section.id === foreignSection.id,
        ),
      );
      const raw = JSON.parse(
        window.localStorage.getItem(storageKey(pubkey, relayUrl)),
      );
      assert.equal(raw.assignments[foreignChannel], undefined);
      assert.ok(
        !raw.sections.some((section) => section.id === foreignSection.id),
      );

      for (const publish of queuedPublishes) {
        assert.equal(publish.assignments[foreignChannel], undefined);
        assert.ok(!publish.sectionIds.includes(foreignSection.id));
      }

      unmount();
    } finally {
      ChannelSectionSyncManager.prototype.bootstrap = originalBootstrap;
      ChannelSectionSyncManager.prototype.publishSections =
        originalPublishSections;
      cleanup();
      window.localStorage.clear();
    }
  });
});

test("channelsReady heals polluted storage and drops orphan foreign sections", async () => {
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const { storageKey } = await import("./channelSectionsStorage.ts");
  const { ChannelSectionSyncManager } = await import(
    "./channelSectionsSync.ts"
  );
  const { useChannelSections } = await import("./useChannelSections.ts");

  await withMockedRelay(async () => {
    const pubkey = "pk-7207-heal";
    const relayUrl = "wss://relay.example";
    const queuedPublishes = [];
    const originalPublishSections =
      ChannelSectionSyncManager.prototype.publishSections;
    ChannelSectionSyncManager.prototype.publishSections =
      function publishSections(store) {
        queuedPublishes.push({
          assignments: { ...store.assignments },
          sectionIds: store.sections.map((section) => section.id),
        });
        return originalPublishSections.call(this, store);
      };

    window.localStorage.setItem(
      storageKey(pubkey, relayUrl),
      JSON.stringify({
        version: 1,
        sections: [
          { id: "s-local", name: "Local", order: 0 },
          { id: "s-foreign", name: "From B", order: 1 },
        ],
        assignments: {
          "chan-local": "s-local",
          "chan-foreign": "s-foreign",
        },
      }),
    );

    try {
      const { result, unmount } = renderHook(() =>
        useChannelSections(pubkey, relayUrl, new Set(["chan-local"]), true),
      );

      await act(async () => {
        await Promise.resolve();
      });

      assert.deepEqual(result.current.assignments, { "chan-local": "s-local" });
      assert.deepEqual(
        result.current.sections.map((section) => section.id),
        ["s-local"],
      );
      const raw = JSON.parse(
        window.localStorage.getItem(storageKey(pubkey, relayUrl)),
      );
      assert.deepEqual(raw.assignments, { "chan-local": "s-local" });
      assert.deepEqual(
        raw.sections.map((section) => section.id),
        ["s-local"],
      );
      assert.ok(
        queuedPublishes.some(
          (publish) =>
            publish.assignments["chan-foreign"] === undefined &&
            !publish.sectionIds.includes("s-foreign"),
        ),
        "heal must publish a cleaned kind:30078 payload",
      );
      unmount();
    } finally {
      ChannelSectionSyncManager.prototype.publishSections =
        originalPublishSections;
      cleanup();
      window.localStorage.clear();
    }
  });
});
