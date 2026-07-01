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
