import assert from "node:assert/strict";
import test from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { truncatePubkey } from "../lib/pubkey.ts";
import { createMarkdownComponents } from "./markdown.tsx";
import { renderCachedMarkdown } from "./markdown/nodeCache.ts";
import { MarkdownRuntimeContext } from "./markdown/runtimeContext.ts";

const KEY = `150b20bd${"a".repeat(52)}15dc`;

for (const agent of [false, true]) {
  test(`rendered ${agent ? "agent" : "human"} abbreviates a bound key without changing its metadata`, () => {
    const label = `Scout (${KEY}) 2`;
    const name = label.toLowerCase();
    const html = renderToStaticMarkup(
      React.createElement(
        MarkdownRuntimeContext.Provider,
        {
          value: {
            channels: [],
            mentionPubkeysByName: { [name]: KEY },
            agentMentionPubkeysByName: agent ? { [name]: KEY } : {},
          },
        },
        renderCachedMarkdown({
          content: `Ask @${label}`,
          mentionNames: [label],
          components: createMarkdownComponents(false, false),
          variant: `compact-mention-${agent}`,
        }),
      ),
    );
    assert.equal(
      html.replace(/<[^>]+>/g, ""),
      `Ask Scout (${truncatePubkey(KEY)}) 2`,
    );
    assert.ok(html.includes(`data-mention-label="${label}"`));
    assert.ok(html.includes(`data-mention-pubkey="${KEY}"`));
    assert.ok(html.includes(`title="${label}"`));
    assert.ok(html.includes(`aria-label="${label}"`));
    assert.match(
      html,
      new RegExp(
        `inline-chip-leading-fragment[^>]*inline-chip-icon-${agent ? "agent" : "human"}`,
      ),
    );
  });
}

test("an unresolved qualified mention stays literal rather than claiming an abbreviated identity", () => {
  const label = `Scout (${KEY})`;
  const html = renderToStaticMarkup(
    React.createElement(
      MarkdownRuntimeContext.Provider,
      {
        value: { channels: [], mentionPubkeysByName: {} },
      },
      renderCachedMarkdown({
        content: `Ask @${label}`,
        mentionNames: [label],
        components: createMarkdownComponents(false, false),
        variant: "unresolved-compact-mention",
      }),
    ),
  );
  assert.ok(html.includes(`@${label}`));
  assert.doesNotMatch(html, /data-mention=/);
});
