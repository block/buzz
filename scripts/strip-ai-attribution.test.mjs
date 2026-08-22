import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";

const script = fileURLToPath(
  new URL("./strip-ai-attribution.sh", import.meta.url),
);

/** Run the hook over `message` and return the rewritten text. */
function strip(message) {
  const dir = mkdtempSync(join(tmpdir(), "strip-ai-"));
  try {
    const file = join(dir, "COMMIT_EDITMSG");
    writeFileSync(file, message, "utf8");
    execFileSync("bash", [script, file], { stdio: "pipe" });
    return readFileSync(file, "utf8");
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

test("removes an assistant co-author trailer", () => {
  const out = strip(
    "feat: thing\n\nCo-Authored-By: Claude Opus 5 <noreply@anthropic.com>\n",
  );
  assert.ok(!/claude/i.test(out), out);
  assert.ok(!/co-authored-by/i.test(out), out);
});

test("removes an assistant trailer identified only by its noreply address", () => {
  const out = strip(
    "feat: thing\n\nCo-authored-by: Some Bot <noreply@anthropic.com>\n",
  );
  assert.ok(!/co-authored-by/i.test(out), out);
});

test("removes a Generated with footer, with or without the emoji", () => {
  const withEmoji = strip(
    "feat: thing\n\n\u{1F916} Generated with [Claude Code](https://claude.com/claude-code)\n",
  );
  assert.ok(!/generated with/i.test(withEmoji), withEmoji);

  const plain = strip("feat: thing\n\nGenerated with Some Tool\n");
  assert.ok(!/generated with/i.test(plain), plain);
});

// The whole point of matching on identity rather than on the word "AI": eating a
// human's pair-programming credit would be a worse failure than leaving a bot
// trailer in.
test("keeps a human co-author trailer", () => {
  const out = strip(
    "feat: thing\n\nCo-authored-by: Jane Developer <jane@example.com>\n",
  );
  assert.match(out, /Co-authored-by: Jane Developer <jane@example\.com>/);
});

// Dropping this would fail the repository's required DCO Check.
test("keeps the DCO signoff", () => {
  const out = strip(
    "feat: thing\n\nCo-Authored-By: Claude <noreply@anthropic.com>\nSigned-off-by: Real Person <real@example.com>\n",
  );
  assert.match(out, /Signed-off-by: Real Person <real@example\.com>/);
  assert.ok(!/claude/i.test(out), out);
});

test("leaves an unrelated message untouched apart from normalization", () => {
  const out = strip("fix: unrelated\n\nA normal body.\n");
  assert.equal(out, "fix: unrelated\n\nA normal body.\n");
});

test("collapses the blank run a stripped footer leaves behind", () => {
  const out = strip(
    "feat: thing\n\nBody.\n\n\u{1F916} Generated with [Claude Code](https://claude.com/claude-code)\n\nCo-Authored-By: Claude <noreply@anthropic.com>\nSigned-off-by: Real Person <real@example.com>\n",
  );
  assert.ok(!/\n\n\n/.test(out), `unexpected blank run:\n${out}`);
  assert.match(out, /Signed-off-by: Real Person/);
});
