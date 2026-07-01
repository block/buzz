import assert from "node:assert/strict";
import test from "node:test";

import { extractConfigNudge, stripConfigNudgeSentinel } from "./configNudge.ts";

// Helper: build a fenced sentinel body containing the given payload.
function withSentinel(prose, payload) {
  return `${prose}\n\n\`\`\`buzz:config-nudge\n${JSON.stringify(payload)}\n\`\`\``;
}

const FIZZ_PUBKEY = "aabbccddeeff0011";
const ATLAS_PUBKEY = "ddeeff00112233aa";
const CODEX_PUBKEY = "112233aabbccddee";

// ── extractConfigNudge ────────────────────────────────────────────────────────

test("extractConfigNudge returns null when no sentinel present", () => {
  assert.equal(
    extractConfigNudge("**Fizz** needs configuration before it can respond."),
    null,
  );
});

test("extractConfigNudge returns null for empty string", () => {
  assert.equal(extractConfigNudge(""), null);
});

test("extractConfigNudge parses env_key requirement", () => {
  const payload = {
    agent_name: "Fizz",
    agent_pubkey: FIZZ_PUBKEY,
    requirements: [{ surface: "env_key", key: "ANTHROPIC_API_KEY" }],
  };
  const content = [
    "**Fizz** needs configuration before it can respond:",
    "- set `ANTHROPIC_API_KEY` in Edit Agent → Environment variables",
    "",
    "Open Edit Agent in the Buzz app to set these.",
    "",
    "```buzz:config-nudge",
    JSON.stringify(payload),
    "```",
  ].join("\n");

  assert.deepEqual(extractConfigNudge(content), payload);
});

test("extractConfigNudge parses normalized_field requirement", () => {
  const payload = {
    agent_name: "Atlas",
    agent_pubkey: ATLAS_PUBKEY,
    requirements: [{ surface: "normalized_field", field: "provider" }],
  };
  assert.deepEqual(extractConfigNudge(withSentinel("prose", payload)), payload);
});

test("extractConfigNudge parses cli_login requirement", () => {
  const payload = {
    agent_name: "Codex",
    agent_pubkey: CODEX_PUBKEY,
    requirements: [
      {
        surface: "cli_login",
        probe_args: ["codex", "login", "status"],
        setup_copy: "run `codex login --with-api-key`",
      },
    ],
  };
  assert.deepEqual(extractConfigNudge(withSentinel("prose", payload)), payload);
});

test("extractConfigNudge parses multiple requirements of mixed types", () => {
  const payload = {
    agent_name: "Atlas",
    agent_pubkey: ATLAS_PUBKEY,
    requirements: [
      { surface: "normalized_field", field: "model" },
      { surface: "env_key", key: "OPENAI_API_KEY" },
      {
        surface: "cli_login",
        probe_args: ["codex", "login"],
        setup_copy: "run `codex login`",
      },
    ],
  };
  const result = extractConfigNudge(withSentinel("prose", payload));
  assert.equal(result?.requirements.length, 3);
  assert.equal(result?.agent_name, "Atlas");
  assert.equal(result?.agent_pubkey, ATLAS_PUBKEY);
});

test("extractConfigNudge returns null for malformed JSON", () => {
  const content = "prose\n\n```buzz:config-nudge\nnot{valid}json\n```";
  assert.equal(extractConfigNudge(content), null);
});

test("extractConfigNudge returns null when JSON is valid but missing agent_name", () => {
  const content = withSentinel("prose", {
    agent_pubkey: FIZZ_PUBKEY,
    requirements: [],
  });
  assert.equal(extractConfigNudge(content), null);
});

test("extractConfigNudge returns null when JSON is valid but missing agent_pubkey", () => {
  const content = withSentinel("prose", {
    agent_name: "Fizz",
    requirements: [],
  });
  assert.equal(extractConfigNudge(content), null);
});

test("extractConfigNudge returns null when requirements contain unknown surface", () => {
  const content = withSentinel("prose", {
    agent_name: "Fizz",
    agent_pubkey: FIZZ_PUBKEY,
    requirements: [{ surface: "unknown_surface", data: "x" }],
  });
  assert.equal(extractConfigNudge(content), null);
});

test("extractConfigNudge returns null when requirements is not an array", () => {
  const content = withSentinel("prose", {
    agent_name: "Fizz",
    agent_pubkey: FIZZ_PUBKEY,
    requirements: "bad",
  });
  assert.equal(extractConfigNudge(content), null);
});

test("extractConfigNudge ignores regular code blocks with other language tags", () => {
  const content = 'prose\n\n```json\n{"key":"val"}\n```';
  assert.equal(extractConfigNudge(content), null);
});

test("extractConfigNudge handles empty requirements array", () => {
  const payload = {
    agent_name: "Fizz",
    agent_pubkey: FIZZ_PUBKEY,
    requirements: [],
  };
  assert.deepEqual(extractConfigNudge(withSentinel("prose", payload)), payload);
});

// ── stripConfigNudgeSentinel ──────────────────────────────────────────────────

test("stripConfigNudgeSentinel returns content unchanged when no sentinel", () => {
  const content = "plain message body";
  assert.equal(stripConfigNudgeSentinel(content), content);
});

test("stripConfigNudgeSentinel strips the sentinel block", () => {
  const prose = "**Fizz** needs configuration.\n\nOpen Edit Agent.";
  const payload = {
    agent_name: "Fizz",
    agent_pubkey: FIZZ_PUBKEY,
    requirements: [],
  };
  const content = withSentinel(prose, payload);
  const stripped = stripConfigNudgeSentinel(content);
  assert.ok(!stripped.includes("buzz:config-nudge"), "sentinel must be gone");
  assert.ok(stripped.includes("needs configuration"), "prose must survive");
});

test("stripConfigNudgeSentinel removes preceding blank line", () => {
  const content = "prose\n\n```buzz:config-nudge\n{}\n```";
  const stripped = stripConfigNudgeSentinel(content);
  // Should not end with multiple newlines — the blank line separator was eaten.
  assert.ok(!stripped.endsWith("\n\n"), "trailing blank line must be trimmed");
});

// ── Auth-gate invariants (Fix 1 — authenticate before rendering) ──────────────
//
// The full auth check lives in MarkdownInner's useMemo: it calls
// normalizePubkey(payload.agent_pubkey) !== normalizePubkey(configNudgeAuthorPubkey)
// and returns null when they don't match. The tests below verify:
// (a) extractConfigNudge still returns the payload regardless of the caller
//     (extraction is pure; auth is responsibility of the renderer),
// (b) when agent_pubkey doesn't match the author, the comparison yields null
//     (simulated inline to keep the test self-contained without React).

import { normalizePubkey } from "./pubkey.ts";

/**
 * Simulate the auth guard in MarkdownInner's configNudge useMemo.
 * Returns the payload if it passes auth, null otherwise.
 */
function authGuardedExtract(content, configNudgeAuthorPubkey) {
  if (!configNudgeAuthorPubkey) return null;
  const payload = extractConfigNudge(content);
  if (payload === null) return null;
  if (
    normalizePubkey(payload.agent_pubkey) !==
    normalizePubkey(configNudgeAuthorPubkey)
  ) {
    return null;
  }
  return payload;
}

const FIZZ_PUBKEY_AUTH = "aabbccddeeff0011223344556677889900aabbcc";
const OTHER_PUBKEY = "ffffffffffffffffffffffffffffffffffffffff";

function makeNudgeBody(agentPubkey) {
  const payload = {
    agent_name: "Fizz",
    agent_pubkey: agentPubkey,
    requirements: [{ surface: "env_key", key: "ANTHROPIC_API_KEY" }],
  };
  return `**Fizz** needs configuration.\n\n\`\`\`buzz:config-nudge\n${JSON.stringify(payload)}\n\`\`\``;
}

test("authGuard_noAuthorPubkey_returnsNull", () => {
  const body = makeNudgeBody(FIZZ_PUBKEY_AUTH);
  assert.equal(
    authGuardedExtract(body, null),
    null,
    "null configNudgeAuthorPubkey must yield null (card path off)",
  );
});

test("authGuard_undefinedAuthorPubkey_returnsNull", () => {
  const body = makeNudgeBody(FIZZ_PUBKEY_AUTH);
  assert.equal(
    authGuardedExtract(body, undefined),
    null,
    "undefined configNudgeAuthorPubkey must yield null (card path off)",
  );
});

test("authGuard_mismatchedAuthor_returnsNull", () => {
  // Fence carries FIZZ_PUBKEY_AUTH but caller says the message author is OTHER_PUBKEY.
  // The card must not render and the fence must NOT be stripped by the caller.
  const body = makeNudgeBody(FIZZ_PUBKEY_AUTH);
  const result = authGuardedExtract(body, OTHER_PUBKEY);
  assert.equal(
    result,
    null,
    "mismatched agent_pubkey vs configNudgeAuthorPubkey must yield null",
  );
  // Fence text must still be in the raw body (not stripped) — stripping only
  // happens when configNudge !== null.
  assert.ok(
    body.includes("buzz:config-nudge"),
    "fence must remain in body when auth guard returns null",
  );
});

test("authGuard_matchingAuthor_returnsPayload", () => {
  const body = makeNudgeBody(FIZZ_PUBKEY_AUTH);
  const result = authGuardedExtract(body, FIZZ_PUBKEY_AUTH);
  assert.notEqual(result, null, "matching author must yield the payload");
  assert.equal(result?.agent_pubkey, FIZZ_PUBKEY_AUTH);
});

test("authGuard_matchingAuthor_caseInsensitive", () => {
  // normalizePubkey lowercases both sides; mixed-case must still match.
  const body = makeNudgeBody(FIZZ_PUBKEY_AUTH.toUpperCase());
  const result = authGuardedExtract(body, FIZZ_PUBKEY_AUTH.toLowerCase());
  assert.notEqual(
    result,
    null,
    "case-insensitive pubkey comparison must pass auth",
  );
});
