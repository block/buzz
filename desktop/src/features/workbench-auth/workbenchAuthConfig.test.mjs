import assert from "node:assert/strict";
import test from "node:test";

import { readWorkbenchAuthConfig } from "./workbenchAuthConfig.ts";

const valid = {
  VITE_OIDC_ISSUER: "https://auth.bizfin.test/application/o/workbench/",
  VITE_OIDC_CLIENT_ID: "workbench-poc",
  VITE_OIDC_REDIRECT_URI: "https://workbench.bizfin.test/auth/callback",
  VITE_OIDC_POST_LOGOUT_REDIRECT_URI: "https://workbench.bizfin.test/",
};

test("OIDC config is disabled only when every value is absent", () => {
  assert.deepEqual(readWorkbenchAuthConfig({}), { config: null, error: null });
  assert.match(
    readWorkbenchAuthConfig({ VITE_OIDC_CLIENT_ID: "partial" }).error,
    /partially configured/,
  );
});

test("OIDC config accepts HTTPS and Buzz desktop callbacks", () => {
  assert.equal(
    readWorkbenchAuthConfig(valid).config?.clientId,
    "workbench-poc",
  );
  assert.equal(
    readWorkbenchAuthConfig({
      ...valid,
      VITE_OIDC_REDIRECT_URI: "buzz://auth/callback",
      VITE_OIDC_POST_LOGOUT_REDIRECT_URI: "buzz://auth/logout-callback",
    }).config?.redirectUri,
    "buzz://auth/callback",
  );
});

test("OIDC config accepts the isolated development callback scheme", () => {
  const result = readWorkbenchAuthConfig({
    ...valid,
    VITE_OIDC_REDIRECT_URI: "buzz-dev://auth/callback",
    VITE_OIDC_POST_LOGOUT_REDIRECT_URI: "buzz-dev://auth/logout-callback",
  });
  assert.equal(result.error, null);
  assert.equal(result.config?.redirectUri, "buzz-dev://auth/callback");
  assert.equal(
    result.config?.postLogoutRedirectUri,
    "buzz-dev://auth/logout-callback",
  );
});

test("OIDC config rejects insecure remote HTTP and non-HTTP issuers", () => {
  assert.match(
    readWorkbenchAuthConfig({
      ...valid,
      VITE_OIDC_ISSUER: "http://auth.example.test",
    }).error,
    /localhost/,
  );
  assert.match(
    readWorkbenchAuthConfig({ ...valid, VITE_OIDC_ISSUER: "buzz://auth" })
      .error,
    /HTTP\(S\) issuer/,
  );
});

test("Desktop proxy is restricted to an explicit HTTP loopback origin", () => {
  assert.equal(
    readWorkbenchAuthConfig({
      ...valid,
      VITE_OIDC_DESKTOP_PROXY_ORIGIN: "http://localhost",
    }).config?.desktopProxyOrigin,
    "http://localhost",
  );
  assert.match(
    readWorkbenchAuthConfig({
      ...valid,
      VITE_OIDC_DESKTOP_PROXY_ORIGIN: "https://proxy.example.test",
    }).error,
    /loopback/,
  );
});
