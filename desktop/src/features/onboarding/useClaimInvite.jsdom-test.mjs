import assert from "node:assert/strict";
import test from "node:test";
import React, { act } from "react";
import { createRoot } from "react-dom/client";

import {
  CommunityOnboardingProvider,
  loadCommunityOnboardingTransaction,
  startCommunityOnboarding,
  useCommunityOnboarding,
} from "./communityOnboarding.tsx";
import { useClaimInvite } from "./useClaimInvite.ts";

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
