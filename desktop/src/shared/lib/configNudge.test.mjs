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

test("extractConfigNudge parses git_bash requirement", () => {
  const payload = {
    agent_name: "Buzz Agent",
    agent_pubkey: ATLAS_PUBKEY,
    requirements: [{ surface: "git_bash" }],
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
        setup_copy: "run `codex login`",
        availability: "available",
      },
    ],
  };
  assert.deepEqual(extractConfigNudge(withSentinel("prose", payload)), payload);
});

test("extractConfigNudge rejects retired adapter_outdated availability", () => {
  // The codex version gate was retired when the ACP bridges started shipping
  // bundled with the app — "adapter_outdated" is no longer a valid
  // availability. Stale nudge JSON emitted by an older app version must not
  // parse into a card the current UI has no rendering for.
  const payload = {
    agent_name: "Codex",
    agent_pubkey: CODEX_PUBKEY,
    requirements: [
      {
        surface: "cli_login",
        probe_args: ["codex", "login", "status"],
        setup_copy: "run `codex login`",
        availability: "adapter_outdated",
      },
    ],
  };
  assert.equal(
    extractConfigNudge(withSentinel("prose", payload)),
    null,
    "retired adapter_outdated availability must be rejected by the validator",
  );
});

test("extractConfigNudge returns null for cli_login without availability", () => {
  // availability is required — old-format payloads (no availability field)
  // must not parse so stale nudge JSON from before the Doctor-CTA update
  // does not silently render a broken card.
  const payload = {
    agent_name: "Codex",
    agent_pubkey: CODEX_PUBKEY,
    requirements: [
      {
        surface: "cli_login",
        probe_args: ["codex", "login", "status"],
        setup_copy: "run `codex login`",
        // no availability field
      },
    ],
  };
  assert.equal(extractConfigNudge(withSentinel("prose", payload)), null);
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
        availability: "not_installed",
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

// ── Signer-vs-tag-attribution regression ─────────────────────────────────────
//
// Pass 2 finding: message.pubkey (display author) can be overridden by actor/p
// tags, so a human-signed event can carry the agent pubkey as its attributed
// author. The auth guard must check the RAW EVENT SIGNER (signerPubkey from
// formatTimelineMessages), not the display author.
//
// We simulate this by passing the HUMAN pubkey as configNudgeAuthorPubkey
// while the payload's agent_pubkey is the AGENT pubkey — as MessageRow now
// does (passes signerPubkey, not pubkey).
//
// If the guard were still using the tag-attributed pubkey (display author =
// agent pubkey), it would pass auth and return the payload — a forged card.
// With the signer check, the human signer != agent payload key → null.

const HUMAN_PUBKEY =
  "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const AGENT_PUBKEY_FOR_SPOOF =
  "aabbccddeeff00112233445566778899aabbccddeeff00112233445566778899";

test("authGuard_signerIsHuman_tagAttributedToAgent_returnsNull", () => {
  // The event was signed by HUMAN_PUBKEY but its `actor` or first `p` tag
  // attributes it to AGENT_PUBKEY_FOR_SPOOF (display author = agent).
  // The payload's agent_pubkey matches the ATTRIBUTED pubkey, not the signer.
  // MessageRow now passes signerPubkey (HUMAN) as configNudgeAuthorPubkey;
  // the auth guard must reject since HUMAN != AGENT.
  const body = makeNudgeBody(AGENT_PUBKEY_FOR_SPOOF);

  // Simulate what MessageRow does: pass the RAW SIGNER, not the display author.
  const result = authGuardedExtract(body, HUMAN_PUBKEY);
  assert.equal(
    result,
    null,
    "raw signer is human; even if actor-tag attributes to agent, must yield null",
  );
  // Fence must remain — not stripped — when auth fails.
  assert.ok(
    body.includes("buzz:config-nudge"),
    "fence must remain in body when signer auth fails",
  );
});

// ── Render-suppression contract ───────────────────────────────────────────────
//
// When `extractConfigNudge` returns a non-null payload, `markdown.tsx` MUST
// suppress the prose markdown node and render only the card:
//
//   {configNudge === null ? markdownNode : null}
//
// This test pins that contract at the parse level. A revert to `{markdownNode}`
// (rendering both prose and card) would violate the invariants checked here:
// the prose text is still in the raw wire body and would re-appear alongside
// the card. If you break these assertions by changing `markdown.tsx` to render
// both, these tests must be updated AND the visual duplication re-approved.

test("nudgePresent_extractNonNull_and_stripRemovesSentinel", () => {
  const prose =
    "**Fizz** needs configuration before it can respond:\n- set `ANTHROPIC_API_KEY` in Edit Agent → Environment variables\n\nOpen Edit Agent in the Buzz app to set these.";
  const payload = {
    agent_name: "Fizz",
    agent_pubkey: FIZZ_PUBKEY,
    requirements: [{ surface: "env_key", key: "ANTHROPIC_API_KEY" }],
  };
  const body = withSentinel(prose, payload);

  // 1. extractConfigNudge sees the sentinel → card renders, prose must be
  //    suppressed. If this returns null the card never renders at all.
  const nudge = extractConfigNudge(body);
  assert.notEqual(
    nudge,
    null,
    "sentinel present → extractConfigNudge must return payload",
  );

  // 2. stripConfigNudgeSentinel removes the fence so whatever markdown node
  //    IS rendered doesn't also show the raw JSON block. The stripped string
  //    must NOT contain the sentinel open-fence marker.
  const stripped = stripConfigNudgeSentinel(body);
  assert.ok(
    !stripped.includes("buzz:config-nudge"),
    "stripped content must not contain the fence marker",
  );

  // 3. The prose IS still in the stripped string — that is intentional: the
  //    stripped string is what non-card clients display. On desktop the prose
  //    is suppressed by the `configNudge === null ? markdownNode : null` guard
  //    in markdown.tsx, NOT by stripping it from the string. If a future
  //    change strips the prose from the string here, that would break CLI
  //    fallback — this assertion guards against that regression too.
  assert.ok(
    stripped.includes("ANTHROPIC_API_KEY"),
    "prose content must remain in stripped string for non-card client fallback",
  );
});

test("nudgeAbsent_extractNull_markdownNodeShown", () => {
  // When there is no sentinel, extractConfigNudge returns null, meaning
  // markdown.tsx renders `markdownNode` normally (no card, no suppression).
  const plainBody = "Hello from an agent without any configuration sentinel.";
  assert.equal(
    extractConfigNudge(plainBody),
    null,
    "no sentinel → extractConfigNudge must return null so markdownNode is shown",
  );
  // stripConfigNudgeSentinel on sentinel-free content is a no-op.
  assert.equal(
    stripConfigNudgeSentinel(plainBody),
    plainBody,
    "no sentinel → stripConfigNudgeSentinel must return content unchanged",
  );
});
