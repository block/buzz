import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  TERSE_REGISTER_ENV_KEY,
  isTerseRegisterEnabled,
  withTerseRegister,
} from "./terseRegisterLogic.ts";

describe("terseRegisterLogic", () => {
  it("is disabled for an empty config", () => {
    assert.equal(isTerseRegisterEnabled({ env_vars: {} }), false);
  });

  it("is enabled only for the exact value 'true'", () => {
    assert.equal(
      isTerseRegisterEnabled({
        env_vars: { [TERSE_REGISTER_ENV_KEY]: "true" },
      }),
      true,
    );
    // Any other value — including clap-truthy-looking ones — reads as off in
    // the UI; the switch repairs it to the canonical shape on next toggle.
    assert.equal(
      isTerseRegisterEnabled({ env_vars: { [TERSE_REGISTER_ENV_KEY]: "1" } }),
      false,
    );
    assert.equal(
      isTerseRegisterEnabled({
        env_vars: { [TERSE_REGISTER_ENV_KEY]: "false" },
      }),
      false,
    );
  });

  it("enable writes the canonical 'true' value", () => {
    const next = withTerseRegister({ env_vars: {} }, true);
    assert.equal(next.env_vars[TERSE_REGISTER_ENV_KEY], "true");
  });

  it("disable removes the key entirely", () => {
    const next = withTerseRegister(
      { env_vars: { [TERSE_REGISTER_ENV_KEY]: "true" } },
      false,
    );
    assert.equal(TERSE_REGISTER_ENV_KEY in next.env_vars, false);
  });

  it("preserves unrelated env vars and config fields", () => {
    const config = {
      env_vars: { OTHER: "keep", [TERSE_REGISTER_ENV_KEY]: "true" },
      provider: "anthropic",
      model: null,
    };
    const next = withTerseRegister(config, false);
    assert.equal(next.env_vars.OTHER, "keep");
    assert.equal(next.provider, "anthropic");
    assert.equal(next.model, null);
  });

  it("does not mutate the input config", () => {
    const config = { env_vars: {} };
    withTerseRegister(config, true);
    assert.deepEqual(config.env_vars, {});
  });
});
