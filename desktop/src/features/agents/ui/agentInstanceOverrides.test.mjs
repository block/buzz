import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  MODEL_OVERRIDE_ENV_KEY,
  PROVIDER_OVERRIDE_ENV_KEY,
  applyLinkedModelProviderOverrides,
} from "./agentInstanceOverrides.ts";

describe("applyLinkedModelProviderOverrides", () => {
  it("stores explicit linked-instance choices without disturbing secrets", () => {
    assert.deepEqual(
      applyLinkedModelProviderOverrides({
        envVars: { ANTHROPIC_API_KEY: "secret" },
        linked: true,
        model: "claude-opus-4-7",
        modelTouched: true,
        provider: "anthropic",
        providerTouched: true,
      }),
      {
        ANTHROPIC_API_KEY: "secret",
        [MODEL_OVERRIDE_ENV_KEY]: "claude-opus-4-7",
        [PROVIDER_OVERRIDE_ENV_KEY]: "anthropic",
      },
    );
  });

  it("removes only the override the user reset to inherit", () => {
    assert.deepEqual(
      applyLinkedModelProviderOverrides({
        envVars: {
          [MODEL_OVERRIDE_ENV_KEY]: "old-model",
          [PROVIDER_OVERRIDE_ENV_KEY]: "anthropic",
        },
        linked: true,
        model: null,
        modelTouched: true,
        provider: "anthropic",
        providerTouched: false,
      }),
      { [PROVIDER_OVERRIDE_ENV_KEY]: "anthropic" },
    );
  });

  it("leaves definition-less agents and untouched linked agents alone", () => {
    const envVars = { KEEP: "yes" };
    assert.equal(
      applyLinkedModelProviderOverrides({
        envVars,
        linked: false,
        model: "m",
        modelTouched: true,
        provider: "p",
        providerTouched: true,
      }),
      envVars,
    );
    assert.equal(
      applyLinkedModelProviderOverrides({
        envVars,
        linked: true,
        model: "m",
        modelTouched: false,
        provider: "p",
        providerTouched: false,
      }),
      envVars,
    );
  });
});
