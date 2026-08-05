import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import {
  combineObserverIngestionAgents,
  projectObserverIngestionAgents,
} from "./useAgentObserverIngestion.ts";
import { useUsersBatchQuery } from "@/features/profile/hooks.ts";
import { CommunitiesProvider } from "@/features/communities/useCommunities.tsx";

const ME = "aaaa1234aaaa1234aaaa1234aaaa1234aaaa1234aaaa1234aaaa1234aaaa1234";
const OTHER =
  "bbbb4321bbbb4321bbbb4321bbbb4321bbbb4321bbbb4321bbbb4321bbbb4321";
const AGENT_LOCAL =
  "cccc1111cccc1111cccc1111cccc1111cccc1111cccc1111cccc1111cccc1111";
const AGENT_REMOTE =
  "dddd2222dddd2222dddd2222dddd2222dddd2222dddd2222dddd2222dddd2222";
const AGENT_FOREIGN =
  "eeee3333eeee3333eeee3333eeee3333eeee3333eeee3333eeee3333eeee3333";

describe("combineObserverIngestionAgents", () => {
  it("keeps managed agents with their real status", () => {
    const result = combineObserverIngestionAgents(
      [{ pubkey: AGENT_LOCAL, status: "running" }],
      [],
      new Map(),
      ME,
    );
    assert.deepEqual(result, [{ pubkey: AGENT_LOCAL, status: "running" }]);
  });

  it("adds declared-owned relay agents as deployed", () => {
    const result = combineObserverIngestionAgents(
      [],
      [AGENT_REMOTE],
      new Map([[AGENT_REMOTE, ME]]),
      ME,
    );
    assert.deepEqual(result, [{ pubkey: AGENT_REMOTE, status: "deployed" }]);
  });

  it("excludes relay agents owned by someone else", () => {
    const result = combineObserverIngestionAgents(
      [],
      [AGENT_FOREIGN],
      new Map([[AGENT_FOREIGN, OTHER]]),
      ME,
    );
    assert.deepEqual(result, []);
  });

  it("excludes relay agents with no declared owner", () => {
    const result = combineObserverIngestionAgents(
      [],
      [AGENT_REMOTE],
      new Map(),
      ME,
    );
    assert.deepEqual(result, []);
  });

  it("does not duplicate an agent that is both managed and on the relay", () => {
    const result = combineObserverIngestionAgents(
      [{ pubkey: AGENT_LOCAL, status: "stopped" }],
      [AGENT_LOCAL],
      new Map([[AGENT_LOCAL, ME]]),
      ME,
    );
    assert.deepEqual(result, [{ pubkey: AGENT_LOCAL, status: "stopped" }]);
  });

  it("matches ownership case-insensitively", () => {
    const result = combineObserverIngestionAgents(
      [],
      [AGENT_REMOTE.toUpperCase()],
      new Map([[AGENT_REMOTE, ME.toUpperCase()]]),
      ME,
    );
    assert.deepEqual(result, [
      { pubkey: AGENT_REMOTE.toUpperCase(), status: "deployed" },
    ]);
  });

  it("returns only managed agents when identity is not resolved yet", () => {
    const result = combineObserverIngestionAgents(
      [{ pubkey: AGENT_LOCAL, status: "running" }],
      [AGENT_REMOTE],
      new Map([[AGENT_REMOTE, ME]]),
      undefined,
    );
    assert.deepEqual(result, [{ pubkey: AGENT_LOCAL, status: "running" }]);
  });
});

describe("projectObserverIngestionAgents", () => {
  it("includes an owned agent profile from channel membership when listRelayAgents omits it", () => {
    const result = projectObserverIngestionAgents(
      [],
      [],
      [AGENT_REMOTE],
      {
        [AGENT_REMOTE]: {
          isAgent: true,
          ownerPubkey: ME,
        },
      },
      ME,
    );

    assert.deepEqual(result, [{ pubkey: AGENT_REMOTE, status: "deployed" }]);
  });

  it("keeps an owned agent when combined profile candidates exceed the relay query cap", async () => {
    const dom = new JSDOM("<!doctype html><html><body></body></html>", {
      url: "http://localhost",
    });
    const previousWindow = globalThis.window;
    const previousDocument = globalThis.document;
    const previousLocalStorage = globalThis.localStorage;
    const previousNavigatorDescriptor = Object.getOwnPropertyDescriptor(
      globalThis,
      "navigator",
    );
    globalThis.window = dom.window;
    globalThis.document = dom.window.document;
    globalThis.localStorage = dom.window.localStorage;
    Object.defineProperty(globalThis, "navigator", {
      value: dom.window.navigator,
      configurable: true,
    });
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;

    const ownedAgent = "f".repeat(64);
    const channelMembers = Array.from({ length: 1_001 }, (_, index) =>
      index.toString(16).padStart(64, "0"),
    );
    channelMembers.push(ownedAgent);
    const relayAgents = [ownedAgent.toUpperCase()];
    const requestedBatches = [];

    dom.window.__TAURI_INTERNALS__ = {
      invoke(command, args) {
        assert.equal(command, "get_users_batch");
        requestedBatches.push(args.pubkeys);
        const visiblePubkeys = args.pubkeys.slice(0, 1_000);
        const profiles = {};
        if (visiblePubkeys.includes(ownedAgent)) {
          profiles[ownedAgent] = {
            display_name: "Owned agent",
            avatar_url: null,
            nip05_handle: null,
            owner_pubkey: ME,
            is_agent: true,
          };
        }
        return Promise.resolve({
          profiles,
          missing: args.pubkeys.filter((pubkey) => !(pubkey in profiles)),
        });
      },
      transformCallback: () => 1,
    };

    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    let latestQuery;
    function Probe() {
      latestQuery = useUsersBatchQuery([...relayAgents, ...channelMembers]);
      return null;
    }

    const root = createRoot(dom.window.document.createElement("div"));
    try {
      await act(async () => {
        root.render(
          React.createElement(
            QueryClientProvider,
            { client: queryClient },
            React.createElement(
              CommunitiesProvider,
              null,
              React.createElement(Probe),
            ),
          ),
        );
      });
      for (let index = 0; index < 10 && latestQuery?.isFetching; index += 1) {
        await act(async () => {
          await new Promise((resolve) => setTimeout(resolve, 0));
        });
      }

      const result = projectObserverIngestionAgents(
        [],
        relayAgents,
        channelMembers,
        latestQuery?.data?.profiles,
        ME,
      );
      assert.deepEqual(result, [{ pubkey: ownedAgent, status: "deployed" }]);
      assert.ok(requestedBatches.length > 1);
      assert.ok(requestedBatches.every((batch) => batch.length <= 1_000));
    } finally {
      await act(async () => root.unmount());
      queryClient.clear();
      dom.window.close();
      globalThis.window = previousWindow;
      globalThis.document = previousDocument;
      globalThis.localStorage = previousLocalStorage;
      if (previousNavigatorDescriptor) {
        Object.defineProperty(
          globalThis,
          "navigator",
          previousNavigatorDescriptor,
        );
      } else {
        delete globalThis.navigator;
      }
    }
  });
});
