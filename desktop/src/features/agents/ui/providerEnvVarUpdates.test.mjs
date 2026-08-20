import assert from "node:assert/strict";
import test from "node:test";

import { envVarsPreservingProviderState } from "./providerEnvVarUpdates.ts";

test("provider changes preserve shared credentials and routing state", () => {
  const current = {
    ANTHROPIC_API_KEY: "anthropic-key",
    OPENAI_API_KEY: "openai-key",
    OPENAI_COMPAT_API_KEY: "compat-key",
    OPENAI_COMPAT_BASE_URL: "http://localhost:11434/v1",
  };

  assert.equal(
    envVarsPreservingProviderState(current, "openai-compat", "openai"),
    current,
  );
  assert.equal(
    envVarsPreservingProviderState(current, "openai", "anthropic"),
    current,
  );
  assert.equal(
    envVarsPreservingProviderState(current, "anthropic", ""),
    current,
  );
});
