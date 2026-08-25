import assert from "node:assert/strict";
import test from "node:test";

import {
  buildBusinessDockCsp,
  desktopBuildChannel,
  normalizeBusinessOrigin,
  validateDesktopBuildEnvironment,
} from "../../../scripts/tauri-business-dock.mjs";

const baseCsp = "default-src 'self'; script-src 'self'; frame-src *";

test("business dock CSP adds only the exact configured origin", () => {
  const csp = buildBusinessDockCsp(baseCsp, "https://biz.example.com");
  assert.match(csp, /frame-src 'self' https:\/\/biz\.example\.com$/);
  assert.doesNotMatch(csp, /frame-src \*/);
});

test("business dock CSP stays self-only when the feature is unconfigured", () => {
  assert.match(buildBusinessDockCsp(baseCsp), /frame-src 'self'$/);
});

test("business dock CSP rejects non-origin and non-HTTP values", () => {
  for (const value of [
    "https://biz.example.com/embed/",
    "file:///tmp/business",
    "javascript:alert(1)",
  ]) {
    assert.throws(() => normalizeBusinessOrigin(value));
  }
});

const productionOidc = {
  VITE_OIDC_ISSUER: "https://login.example.com/application/o/workbench/",
  VITE_OIDC_CLIENT_ID: "workbench-production",
  VITE_OIDC_REDIRECT_URI: "buzz://auth/callback",
  VITE_OIDC_POST_LOGOUT_REDIRECT_URI: "buzz://auth/logout-callback",
  VITE_BUSINESS_APP_ORIGIN: "https://business.example.com",
};

test("Tauri commands select an explicit build channel", () => {
  assert.equal(desktopBuildChannel(["dev"]), "development");
  assert.equal(
    desktopBuildChannel(["build", "--config", "src-tauri/tauri.dev.conf.json"]),
    "development",
  );
  assert.equal(
    desktopBuildChannel([
      "build",
      "--config=src-tauri/tauri.release.conf.json",
    ]),
    "production",
  );
});

test("production build validation rejects every local POC escape hatch", () => {
  assert.doesNotThrow(() =>
    validateDesktopBuildEnvironment("production", productionOidc),
  );
  for (const override of [
    {
      VITE_OIDC_ISSUER:
        "https://auth.bizfin.localhost/application/o/workbench/",
    },
    { VITE_OIDC_REDIRECT_URI: "buzz-dev://auth/callback" },
    { VITE_OIDC_DESKTOP_PROXY_ORIGIN: "http://localhost" },
    { VITE_BUSINESS_APP_ORIGIN: "https://business.bizfin.test" },
  ]) {
    assert.throws(() =>
      validateDesktopBuildEnvironment("production", {
        ...productionOidc,
        ...override,
      }),
    );
  }
});

test("development build validation requires the isolated deep-link scheme", () => {
  assert.doesNotThrow(() =>
    validateDesktopBuildEnvironment("development", {
      ...productionOidc,
      VITE_OIDC_ISSUER:
        "https://auth.bizfin.localhost/application/o/workbench/",
      VITE_OIDC_REDIRECT_URI: "buzz-dev://auth/callback",
      VITE_OIDC_POST_LOGOUT_REDIRECT_URI: "buzz-dev://auth/logout-callback",
      VITE_OIDC_DESKTOP_PROXY_ORIGIN: "http://localhost",
      VITE_BUSINESS_APP_ORIGIN: "https://business.bizfin.localhost",
    }),
  );
  assert.throws(() =>
    validateDesktopBuildEnvironment("development", productionOidc),
  );
});
