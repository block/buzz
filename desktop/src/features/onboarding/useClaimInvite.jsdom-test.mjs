import assert from "node:assert/strict";
import test from "node:test";
import { act, cleanup, render, waitFor } from "@testing-library/react";
import React from "react";
import { createRoot } from "react-dom/client";

import {
  CommunityOnboardingProvider,
  loadCommunityOnboardingTransaction,
  startCommunityOnboarding,
  useCommunityOnboarding,
} from "./communityOnboarding.tsx";
import { useClaimInvite } from "./useClaimInvite.ts";

let controls;
function Claim() {
  controls = useCommunityOnboarding();
  useClaimInvite();
  return null;
}
const policy = {
  version: "v1",
  terms_markdown: "Terms",
  age_attestation_required: false,
};
const tauri = {
  invoke: async (command) => {
    if (command === "sign_event")
      return JSON.stringify({ id: "signed", tags: [] });
    throw new Error(`Unexpected command ${command}`);
  },
};
window.__TAURI_INTERNALS__ = tauri;
globalThis.__TAURI_INTERNALS__ = tauri;

for (const configured of [true, false]) {
  test(`receipt-less invite ${configured ? "waits for native consent" : "claims a policy-free relay"}`, async () => {
    localStorage.clear();
    const requests = [];
    const original = globalThis.fetch;
    globalThis.fetch = async (url) => {
      requests.push(String(url));
      if (String(url).endsWith("/api/join-policy"))
        return Response.json(configured ? { policy } : {});
      return Response.json({
        status: "joined",
        community_id: "id",
        host: "relay.example",
        role: "member",
      });
    };
    try {
      startCommunityOnboarding({
        source: "deep-link-join",
        relayUrl: "wss://relay.example",
        inviteCode: "code",
      });
      render(
        React.createElement(
          CommunityOnboardingProvider,
          null,
          React.createElement(Claim),
        ),
      );
      await waitFor(() =>
        assert.equal(
          controls.transaction.stage,
          configured ? "invite-consent" : "connecting",
        ),
      );
      assert.equal(
        requests.filter((url) => url.endsWith("/claim")).length,
        configured ? 0 : 1,
      );
    } finally {
      cleanup();
      globalThis.fetch = original;
    }
  });
}

test("rejected receipt returns to native consent and clears the stale proof", async () => {
  localStorage.clear();
  const original = globalThis.fetch;
  globalThis.fetch = async () =>
    Response.json({ error: "join_policy_required" }, { status: 403 });
  try {
    startCommunityOnboarding({
      source: "deep-link-join",
      relayUrl: "wss://relay.example",
      inviteCode: "code",
      policyReceipt: "expired",
    });
    render(
      React.createElement(
        CommunityOnboardingProvider,
        null,
        React.createElement(Claim),
      ),
    );
    await waitFor(() =>
      assert.equal(controls.transaction.stage, "invite-consent"),
    );
    assert.equal(controls.transaction.policyReceipt, undefined);
    assert.equal(controls.transaction.error, undefined);
  } finally {
    cleanup();
    globalThis.fetch = original;
  }
});

test("policy lookup failure never claims and can be retried", async () => {
  localStorage.clear();
  const original = globalThis.fetch;
  let failed = true;
  let claims = 0;
  globalThis.fetch = async (url) => {
    if (String(url).endsWith("/api/join-policy")) {
      if (failed) throw new Error("offline");
      return Response.json({ policy });
    }
    claims++;
    throw new Error("unexpected claim");
  };
  try {
    startCommunityOnboarding({
      source: "deep-link-join",
      relayUrl: "wss://relay.example",
      inviteCode: "code",
    });
    render(
      React.createElement(
        CommunityOnboardingProvider,
        null,
        React.createElement(Claim),
      ),
    );
    await waitFor(() => assert.equal(controls.transaction.error, "offline"));
    failed = false;
    await act(async () => controls.update({ error: undefined }));
    await waitFor(() =>
      assert.equal(controls.transaction.stage, "invite-consent"),
    );
    assert.equal(claims, 0);
  } finally {
    cleanup();
    globalThis.fetch = original;
  }
});

test("cancelled policy discovery cannot claim after its response arrives", async () => {
  localStorage.clear();
  const original = globalThis.fetch;
  let resolvePolicy;
  let started;
  const lookupStarted = new Promise((resolve) => {
    started = resolve;
  });
  const response = new Promise((resolve) => {
    resolvePolicy = resolve;
  });
  let claims = 0;
  globalThis.fetch = async (url) => {
    if (String(url).endsWith("/api/join-policy")) {
      started();
      return response;
    }
    claims++;
    throw new Error("unexpected claim");
  };
  try {
    startCommunityOnboarding({
      source: "deep-link-join",
      relayUrl: "wss://relay.example",
      inviteCode: "code",
    });
    render(
      React.createElement(
        CommunityOnboardingProvider,
        null,
        React.createElement(Claim),
      ),
    );
    await lookupStarted;
    await act(async () => controls.clear());
    await act(async () => resolvePolicy(Response.json({})));
    assert.equal(controls.transaction, null);
    assert.equal(claims, 0);
  } finally {
    cleanup();
    globalThis.fetch = original;
  }
});

for (const [sentinel, guidance] of [
  [
    "invite_restricted",
    "This account, or the account that owns it, is restricted in this community. Contact a community administrator. If the restriction is temporary, try again after it ends.",
  ],
  [
    "invite_owner_mismatch",
    "This account does not match its recorded owner. Use the correct account or owner identity to join this community.",
  ],
]) {
  test(`${sentinel} parks the claim with recovery guidance until an explicit retry`, async () => {
    localStorage.clear();
    startCommunityOnboarding({
      source: "deep-link-join",
      relayUrl: "wss://relay.example",
      inviteCode: "same-code",
    });
    const previousFetch = globalThis.fetch;
    const previousTauri = window.__TAURI_INTERNALS__;
    window.__TAURI_INTERNALS__ = {
      async invoke(command) {
        assert.equal(command, "sign_event");
        return JSON.stringify({ id: "signed-event", pubkey: "claimant" });
      },
    };
    let attempts = 0;
    let claimObserved;
    const claim = new Promise((resolve) => {
      claimObserved = resolve;
    });
    let retryObserved;
    const retryClaim = new Promise((resolve) => {
      retryObserved = resolve;
    });
    globalThis.fetch = async (url, options) => {
      // Receipt-less claims first discover the policy; this relay has none.
      if (String(url).endsWith("/api/join-policy")) return Response.json({});
      assert.equal(url, "https://relay.example/api/invites/claim");
      assert.equal(JSON.parse(options.body).code, "same-code");
      attempts++;
      if (attempts === 1) claimObserved();
      if (attempts === 2) retryObserved();
      return new Response(
        JSON.stringify(
          attempts === 1 ? { error: sentinel } : { status: "joined" },
        ),
        { status: attempts === 1 ? 403 : 200 },
      );
    };
    let onboarding;
    function Claim() {
      useClaimInvite();
      onboarding = useCommunityOnboarding();
      return React.createElement(
        "p",
        { role: "alert", "data-stage": onboarding.transaction?.stage },
        onboarding.transaction?.error,
      );
    }
    const container = document.createElement("div");
    const root = createRoot(container);
    try {
      await act(async () => {
        root.render(
          React.createElement(
            CommunityOnboardingProvider,
            null,
            React.createElement(Claim),
          ),
        );
      });
      await act(async () => {
        await claim;
      });
      assert.equal(container.textContent, guidance);
      assert.equal(loadCommunityOnboardingTransaction().stage, "claiming");
      assert.equal(loadCommunityOnboardingTransaction().error, guidance);
      // Re-render with the persisted error: it must not start a retry loop.
      await act(async () => {
        root.render(
          React.createElement(
            CommunityOnboardingProvider,
            null,
            React.createElement(Claim),
          ),
        );
      });
      assert.equal(attempts, 1);
      await act(async () => {
        onboarding.update({ error: undefined });
      });
      // Signing awaits WebCrypto/native IPC before fetch. Observe that second
      // request explicitly, then let React commit its successful transition.
      await act(async () => {
        await retryClaim;
      });
      assert.equal(attempts, 2);
      assert.equal(container.querySelector("p").dataset.stage, "connecting");
      assert.equal(loadCommunityOnboardingTransaction().stage, "connecting");
      assert.equal(loadCommunityOnboardingTransaction().error, undefined);
    } finally {
      await act(async () => root.unmount());
      globalThis.fetch = previousFetch;
      window.__TAURI_INTERNALS__ = previousTauri;
      localStorage.clear();
    }
  });
}
