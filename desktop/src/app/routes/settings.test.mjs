import assert from "node:assert/strict";
import test from "node:test";

// Import the real validateSettingsSearch so tests exercise actual production
// route-validation logic, not a mounted component (mounting SettingsView
// directly bypasses validateSettingsSearch and is what let the earlier gap
// slip past the existing test).
const { validateSettingsSearch } = await import("./settings.tsx");

test("?section=moderation migrates to relay-admin", () => {
  const result = validateSettingsSearch({ section: "moderation" });
  assert.equal(
    result.section,
    "relay-admin",
    "legacy ?section=moderation must redirect to relay-admin, not fall through to undefined",
  );
});

test("?section=relay-admin is accepted as-is", () => {
  const result = validateSettingsSearch({ section: "relay-admin" });
  assert.equal(result.section, "relay-admin");
});

test("?section=doctor still migrates to agents", () => {
  const result = validateSettingsSearch({ section: "doctor" });
  assert.equal(result.section, "agents");
});

test("valid section passes through unchanged", () => {
  const result = validateSettingsSearch({ section: "profile" });
  assert.equal(result.section, "profile");
});

test("unknown section resolves to undefined (falls back to default)", () => {
  const result = validateSettingsSearch({ section: "totally-unknown-value" });
  assert.equal(result.section, undefined);
});

test("missing section resolves to undefined", () => {
  const result = validateSettingsSearch({});
  assert.equal(result.section, undefined);
});
