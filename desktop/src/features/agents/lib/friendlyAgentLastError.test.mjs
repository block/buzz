import assert from "node:assert/strict";
import test from "node:test";

import {
  friendlyAgentLastError,
  friendlyTurnErrorCopy,
  MODEL_NOT_FOUND_COPY,
  RELAY_MESH_DENIED_COPY,
} from "./friendlyAgentLastError.ts";

test("null lastError → null", () => {
  assert.equal(friendlyAgentLastError(null), null);
});

test("empty/whitespace lastError → null", () => {
  assert.equal(friendlyAgentLastError(""), null);
  assert.equal(friendlyAgentLastError("   "), null);
});

test("buzz-acp wrapped auth failure → denied copy", () => {
  const result = friendlyAgentLastError(
    "Agent reported error: llm auth: 401 unauthorized: ...",
  );
  assert.deepEqual(result, {
    severity: "denied",
    copy: RELAY_MESH_DENIED_COPY,
  });
});

test("unwrapped buzz-agent prefix → denied copy", () => {
  // buzz-agent's AgentError::LlmAuth Display is "llm auth: <body>"; if the
  // desktop ever picks that up directly (no AcpError wrapper), we should
  // still recognize it as denial.
  const result = friendlyAgentLastError("llm auth: 403 forbidden");
  assert.deepEqual(result, {
    severity: "denied",
    copy: RELAY_MESH_DENIED_COPY,
  });
});

test("generic harness exit message → passthrough", () => {
  const result = friendlyAgentLastError("harness exited with status code 137");
  assert.deepEqual(result, {
    severity: "generic",
    copy: "harness exited with status code 137",
  });
});

test("trims whitespace before matching", () => {
  const result = friendlyAgentLastError(
    "  Agent reported error: llm auth: nope\n",
  );
  assert.equal(result?.severity, "denied");
  assert.equal(result?.copy, RELAY_MESH_DENIED_COPY);
});

test("substring 'llm auth:' that isn't at start is NOT treated as denial", () => {
  // Some other failure that happens to mention 'llm auth:' deep in a message
  // — we only promote when the failure *is* an auth failure, signalled by
  // the prefix. Anything else stays passthrough so we don't lie about the
  // cause of an unrelated crash.
  const result = friendlyAgentLastError(
    "harness exited with status code 1: stderr mentions llm auth: misleadingly",
  );
  assert.equal(result?.severity, "generic");
  assert.ok(result?.copy.startsWith("harness exited"));
});

test("non-auth Agent reported error stays generic", () => {
  const result = friendlyAgentLastError(
    "Agent reported error: llm: 500 internal server error",
  );
  assert.equal(result?.severity, "generic");
  assert.equal(
    result?.copy,
    "Agent reported error: llm: 500 internal server error",
  );
});

test("code -32002 → model-not-found copy (severity: denied)", () => {
  const result = friendlyAgentLastError(
    "Agent reported error: llm model not found: (goose-claude-opus-4-8) 404 Not Found: ...",
    -32002,
  );
  assert.deepEqual(result, {
    severity: "denied",
    copy: MODEL_NOT_FOUND_COPY,
  });
});

test("code -32001 → relay mesh denied copy (structured path)", () => {
  const result = friendlyAgentLastError("any error text", -32001);
  assert.deepEqual(result, {
    severity: "denied",
    copy: RELAY_MESH_DENIED_COPY,
  });
});

test("code null falls through to legacy string matching", () => {
  const result = friendlyAgentLastError(
    "Agent reported error: llm auth: 401 unauthorized",
    null,
  );
  assert.deepEqual(result, {
    severity: "denied",
    copy: RELAY_MESH_DENIED_COPY,
  });
});

test("code undefined falls through to legacy string matching", () => {
  const result = friendlyAgentLastError(
    "Agent reported error: llm auth: 403 forbidden",
    undefined,
  );
  assert.deepEqual(result, {
    severity: "denied",
    copy: RELAY_MESH_DENIED_COPY,
  });
});

test("unknown code falls through to generic", () => {
  const result = friendlyAgentLastError("some error", -99999);
  assert.deepEqual(result, {
    severity: "generic",
    copy: "some error",
  });
});

test("friendlyTurnErrorCopy: numeric code -32002 → model-not-found copy", () => {
  assert.equal(
    friendlyTurnErrorCopy("raw error", -32002),
    MODEL_NOT_FOUND_COPY,
  );
});

test("friendlyTurnErrorCopy: string-encoded code coerces to number", () => {
  assert.equal(
    friendlyTurnErrorCopy("raw error", "-32001"),
    RELAY_MESH_DENIED_COPY,
  );
});

test("friendlyTurnErrorCopy: missing code falls back to raw text", () => {
  assert.equal(friendlyTurnErrorCopy("raw error", undefined), "raw error");
  assert.equal(friendlyTurnErrorCopy("raw error", null), "raw error");
});

test("friendlyTurnErrorCopy: unknown code passes raw text through", () => {
  assert.equal(friendlyTurnErrorCopy("raw error", 12345), "raw error");
});
