import assert from "node:assert/strict";
import test from "node:test";

import { buildProjectConnectionSecretChanges } from "./projectConnectionSecrets.ts";

test("preserves new secret values byte-for-byte", () => {
  assert.deepEqual(
    buildProjectConnectionSecretChanges(
      [{ key: " API_TOKEN ", value: "  value with spaces  " }],
      [],
      [],
    ),
    {
      ok: true,
      env: {
        API_TOKEN: "  value with spaces  ",
      },
      removeEnvKeys: [],
    },
  );
});

test("a new secret requires a value", () => {
  assert.deepEqual(
    buildProjectConnectionSecretChanges(
      [{ key: "API_TOKEN", value: "" }],
      [],
      [],
    ),
    { ok: false, error: "Enter a value for API_TOKEN." },
  );
});

test("rejects duplicate and malformed secret names", () => {
  assert.equal(
    buildProjectConnectionSecretChanges(
      [
        { key: "API_TOKEN", value: "first" },
        { key: "API_TOKEN", value: "second" },
      ],
      [],
      [],
    ).ok,
    false,
  );
  assert.equal(
    buildProjectConnectionSecretChanges(
      [{ key: "api-token", value: "value" }],
      [],
      [],
    ).ok,
    false,
  );
});

test("a blank existing secret retains its saved value", () => {
  assert.deepEqual(
    buildProjectConnectionSecretChanges(
      [{ key: "API_TOKEN", value: "" }],
      ["API_TOKEN"],
      [],
    ),
    { ok: true, env: {}, removeEnvKeys: [] },
  );
});

test("explicit removals stay separate from replacement values", () => {
  assert.deepEqual(
    buildProjectConnectionSecretChanges(
      [{ key: "API_TOKEN", value: "replacement" }],
      ["API_TOKEN"],
      ["API_TOKEN", "OLD_TOKEN"],
    ),
    {
      ok: true,
      env: { API_TOKEN: "replacement" },
      removeEnvKeys: ["OLD_TOKEN"],
    },
  );
});
