import assert from "node:assert/strict";
import test from "node:test";

import { coerceConfigValues } from "./ProviderConfigFields.tsx";

test("coerceConfigValues casts integer and boolean schema types", () => {
  const schema = {
    properties: {
      size: { type: "integer" },
      enabled: { type: "boolean" },
      region: { type: "string" },
    },
  };
  assert.deepEqual(
    coerceConfigValues(
      { size: "3", enabled: "true", region: "us" },
      schema,
    ),
    { size: 3, enabled: true, region: "us" },
  );
});

test("coerceConfigValues keeps empty enum-style strings", () => {
  const schema = {
    properties: {
      provider: {
        type: "string",
        enum: ["", "hetzner", "aws"],
      },
    },
  };
  assert.deepEqual(coerceConfigValues({ provider: "" }, schema), {
    provider: "",
  });
});

test("coerceConfigValues without schema returns shallow copy of strings", () => {
  const input = { a: "1", b: "true" };
  assert.deepEqual(coerceConfigValues(input, undefined), input);
});
