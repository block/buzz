import assert from "node:assert/strict";
import test from "node:test";

import {
  getProviderBaseUrlFieldState,
  isValidProviderBaseUrl,
  normalizeProviderBaseUrl,
  OPENAI_COMPAT_BASE_URL_ENV_KEY,
  providerOwnsBaseUrlField,
} from "./providerBaseUrlFieldState.ts";

test("OPENAI_COMPAT_BASE_URL_ENV_KEY is the canonical env key", () => {
  assert.equal(OPENAI_COMPAT_BASE_URL_ENV_KEY, "OPENAI_COMPAT_BASE_URL");
});

test("isValidProviderBaseUrl_blank_isValidForNativeDefault", () => {
  assert.equal(isValidProviderBaseUrl(""), true);
  assert.equal(isValidProviderBaseUrl("   "), true);
});

test("isValidProviderBaseUrl_httpsWithPath_isAccepted", () => {
  assert.equal(isValidProviderBaseUrl("https://api.openai.com/v1"), true);
});

test("isValidProviderBaseUrl_httpLocalhost_isAccepted", () => {
  assert.equal(isValidProviderBaseUrl("http://127.0.0.1:9337/v1"), true);
  assert.equal(isValidProviderBaseUrl("http://localhost:8080/v1/"), true);
});

test("isValidProviderBaseUrl_malformedAndNonHttp_areRejected", () => {
  assert.equal(isValidProviderBaseUrl("not-a-url"), false);
  assert.equal(isValidProviderBaseUrl("ftp://example.com/v1"), false);
  assert.equal(isValidProviderBaseUrl("https://"), false);
  assert.equal(isValidProviderBaseUrl("://missing-scheme"), false);
});

test("normalizeProviderBaseUrl_trimsSurroundingWhitespace", () => {
  assert.equal(
    normalizeProviderBaseUrl("  https://api.example.com/v1  "),
    "https://api.example.com/v1",
  );
  assert.equal(normalizeProviderBaseUrl("   "), "");
});

test("providerOwnsBaseUrlField_onlyOpenaiCompat", () => {
  assert.equal(providerOwnsBaseUrlField("openai-compat"), true);
  assert.equal(providerOwnsBaseUrlField("OpenAI-Compat"), true);
  assert.equal(providerOwnsBaseUrlField("openai"), false);
  assert.equal(providerOwnsBaseUrlField("anthropic"), false);
  assert.equal(providerOwnsBaseUrlField("openrouter"), false);
  assert.equal(providerOwnsBaseUrlField(""), false);
});

test("providerBaseUrlFieldState_openaiCompat_absentLocal_isValidAndNotInherited", () => {
  const state = getProviderBaseUrlFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {},
    envVars: {},
    globalEnvVars: {},
    provider: "openai-compat",
  });

  assert.equal(state.envKey, OPENAI_COMPAT_BASE_URL_ENV_KEY);
  assert.equal(state.visible, true);
  assert.equal(state.value, "");
  assert.equal(state.isInherited, false);
  assert.equal(state.inheritedLabel, "");
  assert.equal(state.isValid, true);
  assert.equal(state.isInvalid, false);
});

test("providerBaseUrlFieldState_openaiCompat_validLocalUrl_wins", () => {
  const state = getProviderBaseUrlFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {
      OPENAI_COMPAT_BASE_URL: "https://global.example/v1",
    },
    envVars: {
      OPENAI_COMPAT_BASE_URL: "https://local.example/v1",
    },
    globalEnvVars: {
      OPENAI_COMPAT_BASE_URL: "https://global.example/v1",
    },
    provider: "openai-compat",
  });

  assert.equal(state.value, "https://local.example/v1");
  assert.equal(state.isInherited, false);
  assert.equal(state.isValid, true);
  assert.equal(state.isInvalid, false);
});

test("providerBaseUrlFieldState_openaiCompat_malformedLocal_isInvalid", () => {
  const state = getProviderBaseUrlFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {},
    envVars: { OPENAI_COMPAT_BASE_URL: "not-a-url" },
    globalEnvVars: {},
    provider: "openai-compat",
  });

  assert.equal(state.isValid, false);
  assert.equal(state.isInvalid, true);
  assert.equal(state.value, "not-a-url");
});

test("providerBaseUrlFieldState_openaiCompat_globalInherited_showsLabelWithoutLocalCopy", () => {
  const state = getProviderBaseUrlFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {},
    envVars: {},
    globalEnvVars: {
      OPENAI_COMPAT_BASE_URL: "https://global.example/v1",
    },
    provider: "openai-compat",
  });

  assert.equal(state.value, "");
  assert.equal(state.isInherited, true);
  assert.equal(state.inheritedLabel, "Inherited from global config");
  assert.equal(state.isValid, true);
});

test("providerBaseUrlFieldState_openaiCompat_personaInherited", () => {
  const state = getProviderBaseUrlFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {
      OPENAI_COMPAT_BASE_URL: "https://persona.example/v1",
    },
    envVars: {},
    globalEnvVars: {},
    personaSatisfied: true,
    provider: "openai-compat",
  });

  assert.equal(state.isInherited, true);
  assert.equal(state.inheritedLabel, "Inherited from agent profile");
  assert.equal(state.value, "");
  assert.equal(state.isValid, true);
});

test("providerBaseUrlFieldState_openaiCompat_localEmptyShadowsInherited", () => {
  const state = getProviderBaseUrlFieldState({
    bakedEnvKeys: ["OPENAI_COMPAT_BASE_URL"],
    effectiveEnvVars: { OPENAI_COMPAT_BASE_URL: "" },
    envVars: { OPENAI_COMPAT_BASE_URL: "" },
    globalEnvVars: {
      OPENAI_COMPAT_BASE_URL: "https://global.example/v1",
    },
    provider: "openai-compat",
  });

  assert.equal(state.value, "");
  assert.equal(state.isInherited, false);
  assert.equal(state.inheritedLabel, "");
  assert.equal(state.isValid, true);
});

test("providerBaseUrlFieldState_openaiCompat_fileInherited", () => {
  const state = getProviderBaseUrlFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {},
    envVars: {},
    fileSatisfiedEnvKeys: ["OPENAI_COMPAT_BASE_URL"],
    globalEnvVars: {},
    provider: "openai-compat",
  });

  assert.equal(state.isInherited, true);
  assert.equal(state.inheritedLabel, "Set in runtime config");
  assert.equal(state.isValid, true);
});

test("providerBaseUrlFieldState_openaiCompat_buildInherited", () => {
  const state = getProviderBaseUrlFieldState({
    bakedEnvKeys: ["OPENAI_COMPAT_BASE_URL"],
    effectiveEnvVars: {},
    envVars: {},
    globalEnvVars: {},
    provider: "openai-compat",
  });

  assert.equal(state.isInherited, true);
  assert.equal(state.inheritedLabel, "Inherited from build");
  assert.equal(state.isValid, true);
});

test("providerBaseUrlFieldState_openai_doesNotOwnStructuredField", () => {
  const state = getProviderBaseUrlFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {},
    envVars: {
      OPENAI_COMPAT_BASE_URL: "https://should-not-surface.example/v1",
    },
    globalEnvVars: {},
    provider: "openai",
  });

  assert.equal(state.envKey, null);
  assert.equal(state.visible, false);
  assert.equal(state.isValid, true);
  assert.equal(state.isInvalid, false);
});

test("providerBaseUrlFieldState_whitespaceOnlyLocal_isValidAfterNormalize", () => {
  const state = getProviderBaseUrlFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {},
    envVars: { OPENAI_COMPAT_BASE_URL: "   " },
    globalEnvVars: {},
    provider: "openai-compat",
  });

  // Local key is present but blank after trim → valid native default.
  assert.equal(state.isValid, true);
  assert.equal(state.isInvalid, false);
  assert.equal(state.isInherited, false);
});

test("providerBaseUrlFieldState_advancedHideKeys_onlyWhenStructuredFieldOwns", () => {
  const owned = getProviderBaseUrlFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {},
    envVars: {},
    globalEnvVars: {},
    provider: "openai-compat",
  });
  const notOwned = getProviderBaseUrlFieldState({
    bakedEnvKeys: [],
    effectiveEnvVars: {},
    envVars: { OPENAI_COMPAT_BASE_URL: "https://raw.example/v1" },
    globalEnvVars: {},
    provider: "anthropic",
  });

  // Advanced should hide the key only while the structured field owns it.
  assert.deepEqual(
    [
      ...(owned.envKey ? [owned.envKey] : []),
      ...(notOwned.envKey ? [notOwned.envKey] : []),
    ],
    [OPENAI_COMPAT_BASE_URL_ENV_KEY],
  );
  assert.equal(notOwned.envKey, null);
});
