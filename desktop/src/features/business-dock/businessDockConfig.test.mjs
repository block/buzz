import assert from "node:assert/strict";
import test from "node:test";

import {
  isAllowedBusinessUrl,
  readBusinessDockConfig,
  resolveAllowedBusinessUrl,
} from "./businessDockConfig.ts";

const validEnv = {
  VITE_BUSINESS_APP_ORIGIN: "https://biz.example.com",
  VITE_BUSINESS_APP_URL: "https://biz.example.com/embed/",
};

test("business dock config accepts an HTTP(S) home URL on the configured origin", () => {
  assert.deepEqual(readBusinessDockConfig(validEnv), {
    config: {
      homeUrl: "https://biz.example.com/embed/",
      origin: "https://biz.example.com",
    },
    error: null,
  });
});

test("business dock config fails closed when values are missing", () => {
  assert.equal(readBusinessDockConfig({}).config, null);
});

test("business dock config rejects malformed and non-HTTP origins", () => {
  for (const origin of [
    "not a URL",
    "file:///tmp/business",
    "javascript:alert(1)",
  ]) {
    assert.equal(
      readBusinessDockConfig({ ...validEnv, VITE_BUSINESS_APP_ORIGIN: origin })
        .config,
      null,
    );
  }
});

test("business dock URL allowlist accepts relative and same-origin URLs", () => {
  const result = readBusinessDockConfig(validEnv);
  assert.ok(result.config);
  assert.equal(
    resolveAllowedBusinessUrl("orders/42", result.config),
    "https://biz.example.com/embed/orders/42",
  );
  assert.equal(
    isAllowedBusinessUrl("https://biz.example.com/sales", result.config),
    true,
  );
});

test("business dock URL allowlist rejects other origins and active URL schemes", () => {
  const result = readBusinessDockConfig(validEnv);
  assert.ok(result.config);
  for (const url of [
    "https://example.org/",
    "javascript:alert(1)",
    "data:text/html,hello",
    "file:///tmp/business.html",
  ]) {
    assert.equal(isAllowedBusinessUrl(url, result.config), false, url);
  }
});
