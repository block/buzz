import assert from "node:assert/strict";
import test from "node:test";

import {
  buildAgentSnapshotClipboardHtml,
  handleAgentSnapshotPaste,
  parseAgentSnapshotClipboardHtml,
} from "./agentSnapshotClipboard.ts";

const SHA256 = "a".repeat(64);
const URL = `https://relay.example/media/${SHA256}.png`;

function buildHtml(overrides = {}) {
  return buildAgentSnapshotClipboardHtml({
    attachment: {
      filename: "animation-auditor.agent.png",
      sha256: SHA256,
      size: 1234,
      type: "image/png",
      uploaded: 1,
      url: URL,
      ...overrides,
    },
    displayName: "Animation Auditor",
  });
}

test("copied agent HTML restores a labeled snapshot attachment", () => {
  assert.deepEqual(parseAgentSnapshotClipboardHtml(buildHtml()), {
    displayLabel: "Animation Auditor",
    filename: "animation-auditor.agent.png",
    sha256: SHA256,
    size: 1234,
    type: "image/png",
    uploaded: 0,
    url: URL,
  });
});

test("copied agent HTML escapes its visible link and name", () => {
  const html = buildAgentSnapshotClipboardHtml({
    attachment: {
      filename: "research.agent.png",
      sha256: SHA256,
      size: 1234,
      type: "image/png",
      uploaded: 1,
      url: "https://relay.example/media/a.png?x=1&y=2",
    },
    displayName: 'Research <Agent> "One"',
  });

  assert.match(html, /Research &lt;Agent&gt; &quot;One&quot;/);
  assert.match(html, /x=1&amp;y=2/);
});

test("invalid copied snapshot metadata falls through to normal paste", () => {
  const invalid = [
    buildHtml({ sha256: "short" }),
    buildHtml({ filename: "not-an-agent.png" }),
    buildHtml({ size: 11 * 1024 * 1024 }),
    buildHtml({ type: "text/html" }),
    buildHtml({ url: "javascript:alert(1)" }),
  ];

  for (const html of invalid) {
    assert.equal(parseAgentSnapshotClipboardHtml(html), null);
  }
});

test("ordinary clipboard HTML is ignored", () => {
  assert.equal(
    parseAgentSnapshotClipboardHtml('<a href="https://example.com">link</a>'),
    null,
  );
});

test("snapshot paste adds one attachment and prevents the raw link paste", () => {
  let attachments = [];
  let preventDefaultCount = 0;
  const event = {
    clipboardData: { getData: () => buildHtml() },
    preventDefault: () => preventDefaultCount++,
  };
  const setPending = (update) => {
    attachments = update(attachments);
  };

  assert.equal(handleAgentSnapshotPaste(event, setPending), true);
  assert.equal(handleAgentSnapshotPaste(event, setPending), true);
  assert.equal(attachments.length, 1);
  assert.equal(preventDefaultCount, 2);
});
