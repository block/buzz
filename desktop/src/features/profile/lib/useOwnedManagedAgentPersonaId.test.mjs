import assert from "node:assert/strict";
import test from "node:test";
import { finalizeEvent, getPublicKey } from "nostr-tools/pure";
import { JSDOM } from "jsdom";

import { relayClient } from "@/shared/api/relayClient";
import {
  KIND_IA_ARCHIVE_REQUEST,
  KIND_MANAGED_AGENT,
} from "@/shared/constants/kinds";
import {
  personaIdFromOwnedManagedAgentArchive,
  personaIdFromOwnedManagedAgentEvent,
  useOwnedManagedAgentPersonaId,
} from "./useOwnedManagedAgentPersonaId.ts";

const OWNER_SECRET = new Uint8Array(32);
OWNER_SECRET[31] = 1;
const OTHER_SECRET = new Uint8Array(32);
OTHER_SECRET[31] = 2;
const OWNER = getPublicKey(OWNER_SECRET);
const AGENT = "a".repeat(64);

function managedAgentEvent({
  agentPubkey = AGENT,
  content = JSON.stringify({ persona_id: "persona-reviewer" }),
  secret = OWNER_SECRET,
} = {}) {
  return finalizeEvent(
    {
      created_at: 1,
      kind: KIND_MANAGED_AGENT,
      tags: [["d", agentPubkey]],
      content,
    },
    secret,
  );
}

function managedAgentArchive({
  agentPubkey = AGENT,
  personaId = "persona-reviewer",
  secret = OWNER_SECRET,
} = {}) {
  return finalizeEvent(
    {
      created_at: 2,
      kind: KIND_IA_ARCHIVE_REQUEST,
      tags: [["p", agentPubkey]],
      content: JSON.stringify({ persona_id: personaId }),
    },
    secret,
  );
}

test("resolves an owner-signed historical agent key to its persona", () => {
  assert.equal(
    personaIdFromOwnedManagedAgentEvent(managedAgentEvent(), OWNER, AGENT),
    "persona-reviewer",
  );
});

test("resolves a deleted historical agent key from its owner-signed archive request", () => {
  assert.equal(
    personaIdFromOwnedManagedAgentArchive(managedAgentArchive(), OWNER, AGENT),
    "persona-reviewer",
  );
});

test("rejects archive aliases with the wrong owner or target", () => {
  assert.equal(
    personaIdFromOwnedManagedAgentArchive(
      managedAgentArchive({ secret: OTHER_SECRET }),
      OWNER,
      AGENT,
    ),
    null,
  );
  assert.equal(
    personaIdFromOwnedManagedAgentArchive(
      managedAgentArchive({ agentPubkey: "b".repeat(64) }),
      OWNER,
      AGENT,
    ),
    null,
  );
});

test("rejects a managed-agent event from a different owner", () => {
  assert.equal(
    personaIdFromOwnedManagedAgentEvent(
      managedAgentEvent({ secret: OTHER_SECRET }),
      OWNER,
      AGENT,
    ),
    null,
  );
});

test("rejects a managed-agent event for a different agent key", () => {
  assert.equal(
    personaIdFromOwnedManagedAgentEvent(
      managedAgentEvent({ agentPubkey: "b".repeat(64) }),
      OWNER,
      AGENT,
    ),
    null,
  );
});

test("rejects empty or malformed persona ids", () => {
  assert.equal(
    personaIdFromOwnedManagedAgentEvent(
      managedAgentEvent({ content: JSON.stringify({ persona_id: "" }) }),
      OWNER,
      AGENT,
    ),
    null,
  );
  assert.equal(
    personaIdFromOwnedManagedAgentEvent(
      managedAgentEvent({ content: "not-json" }),
      OWNER,
      AGENT,
    ),
    null,
  );
});

test("still resolves the live record when the archive lookup fails", async () => {
  const dom = new JSDOM("<!doctype html><html><body></body></html>", {
    url: "http://localhost",
  });
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const originalFetchFirstEvent = relayClient.fetchFirstEvent;
  relayClient.fetchFirstEvent = async (filter) => {
    if (filter.kinds?.includes(KIND_IA_ARCHIVE_REQUEST)) {
      throw new Error("archive lookup unavailable");
    }
    return managedAgentEvent();
  };

  try {
    const { result } = renderHook(() =>
      useOwnedManagedAgentPersonaId({
        agentPubkey: AGENT,
        enabled: true,
        ownerPubkey: OWNER,
      }),
    );
    await act(async () => {
      await Promise.resolve();
    });
    assert.equal(result.current, "persona-reviewer");
  } finally {
    cleanup();
    relayClient.fetchFirstEvent = originalFetchFirstEvent;
    dom.window.close();
  }
});

test("does not expose a persona result for stale lookup inputs", async () => {
  const dom = new JSDOM("<!doctype html><html><body></body></html>", {
    url: "http://localhost",
  });
  Object.assign(globalThis, {
    document: dom.window.document,
    HTMLElement: dom.window.HTMLElement,
    IS_REACT_ACT_ENVIRONMENT: true,
    window: dom.window,
  });
  const { act, cleanup, renderHook } = await import("@testing-library/react");
  const originalFetchFirstEvent = relayClient.fetchFirstEvent;
  relayClient.fetchFirstEvent = async (filter) =>
    filter.kinds?.includes(KIND_MANAGED_AGENT) ? managedAgentEvent() : null;
  const observed = [];

  try {
    const { result, rerender, unmount } = renderHook(
      (props) => {
        const personaId = useOwnedManagedAgentPersonaId(props);
        observed.push(personaId);
        return personaId;
      },
      {
        initialProps: {
          agentPubkey: AGENT,
          enabled: true,
          ownerPubkey: OWNER,
        },
      },
    );
    await act(async () => {
      await Promise.resolve();
    });
    assert.equal(result.current, "persona-reviewer");

    const switchIndex = observed.length;
    rerender({
      agentPubkey: "b".repeat(64),
      enabled: false,
      ownerPubkey: OWNER,
    });
    assert.deepEqual(observed.slice(switchIndex), [null]);
    assert.equal(result.current, null);
    unmount();
  } finally {
    cleanup();
    relayClient.fetchFirstEvent = originalFetchFirstEvent;
    dom.window.close();
  }
});
