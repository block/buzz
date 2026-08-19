import assert from "node:assert/strict";
import test from "node:test";

import {
  fromRawRegisteredAgentReference,
  toRawRegisterExistingAgentInput,
} from "./tauriRegisteredAgents.ts";

const PUBKEY = `${"ABCDEF".repeat(10)}ABCD`;

test("maps registered agent references from Tauri snake_case to TS camelCase without runtime fields", () => {
  const mapped = fromRawRegisteredAgentReference({
    pubkey: PUBKEY,
    label: "Existing Goose",
    role_summary: "reviewer",
    created_at: "2026-08-18T12:00:00Z",
    updated_at: "2026-08-18T12:30:00Z",
    private_key_nsec: "redacted-test-value",
    agent_command: "goose",
    status: "running",
    pid: 123,
  });

  assert.deepEqual(mapped, {
    pubkey: PUBKEY.toLowerCase(),
    label: "Existing Goose",
    roleSummary: "reviewer",
    createdAt: "2026-08-18T12:00:00Z",
    updatedAt: "2026-08-18T12:30:00Z",
  });
  assert.equal("privateKeyNsec" in mapped, false);
  assert.equal("agentCommand" in mapped, false);
  assert.equal("status" in mapped, false);
  assert.equal("pid" in mapped, false);
});

test("register input sends only pubkey, label, and role summary with omitted blanks as null", () => {
  const raw = toRawRegisterExistingAgentInput({
    pubkey: `  ${PUBKEY}  `,
    label: "  Existing Goose  ",
    roleSummary: "   ",
  });

  assert.deepEqual(raw, {
    pubkey: PUBKEY.toLowerCase(),
    label: "Existing Goose",
    roleSummary: null,
  });
});

test("malformed registered agent store entries fail visibly instead of being filtered", () => {
  assert.throws(
    () =>
      fromRawRegisteredAgentReference({
        pubkey: "not-a-key",
        label: "Bad",
        role_summary: null,
        created_at: "2026-08-18T12:00:00Z",
        updated_at: "2026-08-18T12:30:00Z",
      }),
    /registered agent pubkey/i,
  );
});
