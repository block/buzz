import assert from "node:assert/strict";
import test from "node:test";

import {
  createWorkbenchCallbackReplayGuard,
  getValidWorkbenchUser,
  isWorkbenchAuthCallback,
  shouldRefreshWorkbenchUser,
} from "./workbenchAuthClient.ts";

const config = {
  issuer: "https://auth.bizfin.localhost/application/o/workbench",
  clientId: "workbench-poc",
  redirectUri: "buzz://auth/callback",
  postLogoutRedirectUri: "buzz://auth/logout-callback",
};

test("desktop OIDC callback matches only the registered exact route", () => {
  assert.equal(
    isWorkbenchAuthCallback(
      "buzz://auth/callback?code=opaque&state=state",
      config,
    ),
    true,
  );
  assert.equal(
    isWorkbenchAuthCallback("buzz://evil/callback?code=opaque", config),
    false,
  );
  assert.equal(
    isWorkbenchAuthCallback(
      "buzz://auth/business-bootstrap?code=opaque",
      config,
    ),
    false,
  );
});

test("duplicate desktop callbacks are consumed once", () => {
  const guard = createWorkbenchCallbackReplayGuard();
  const callback = "buzz://auth/callback?code=opaque&state=state";
  assert.equal(guard.accept(callback), true);
  assert.equal(guard.accept(callback), false);
  assert.equal(guard.accept(`${callback}-new`), true);
});

test("Workbench tokens refresh before expiry without opening sign-in", async () => {
  const now = 2_000_000_000;
  assert.equal(
    shouldRefreshWorkbenchUser({ expired: false, expires_at: now + 121 }, now),
    false,
  );
  assert.equal(
    shouldRefreshWorkbenchUser({ expired: false, expires_at: now + 120 }, now),
    true,
  );
  let refreshes = 0;
  const refreshed = {
    expired: false,
    expires_at: now + 3600,
    access_token: "fresh",
  };
  const user = await getValidWorkbenchUser({
    getUser: async () => ({ expired: true, expires_at: now - 1 }),
    signinSilent: async () => {
      refreshes += 1;
      return refreshed;
    },
  });
  assert.equal(refreshes, 1);
  assert.equal(user, refreshed);
});

test("failed silent refresh fails closed", async () => {
  const user = await getValidWorkbenchUser({
    getUser: async () => ({ expired: true, expires_at: 1 }),
    signinSilent: async () => {
      throw new Error("refresh revoked");
    },
  });
  assert.equal(user, null);
});
