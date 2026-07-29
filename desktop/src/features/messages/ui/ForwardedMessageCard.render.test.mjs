import assert from "node:assert/strict";
import test from "node:test";

import React from "react";
import { renderToStaticMarkup } from "react-dom/server";

import { ForwardedMessageCard } from "./ForwardedMessageCard.tsx";

const ORIGINAL_PUBKEY =
  "953d5b1c9f0c1d4e8a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f7081";

function render(props = {}) {
  return renderToStaticMarkup(
    React.createElement(
      ForwardedMessageCard,
      {
        authorAvatarUrl: null,
        authorDisplayName: "Alice",
        originalCreatedAt: 1753600000,
        originalPubkey: ORIGINAL_PUBKEY,
        sourceChannelName: "general",
        sourceType: "channel",
        testId: "forwarded-message-test",
        ...props,
      },
      props.children ??
        React.createElement("p", null, "original body text here"),
    ),
  );
}

test("renders attribution, author, and the quoted original body", () => {
  const html = render();
  assert.match(html, /data-testid="forwarded-message-test"/);
  assert.match(html, /Forwarded from/);
  assert.match(html, /data-testid="forwarded-from-channel"/);
  assert.match(html, /#general/);
  assert.match(html, /Alice/);
  assert.match(html, /original body text here/);
});

test("renders the note above the card when present, omits it otherwise", () => {
  const withNote = render({
    note: React.createElement("p", null, "look at this note"),
  });
  assert.match(withNote, /look at this note/);
  // Note must precede the attribution row in document order.
  assert.ok(
    withNote.indexOf("look at this note") < withNote.indexOf("Forwarded from"),
  );

  const withoutNote = render();
  assert.doesNotMatch(withoutNote, /look at this note/);
});

test("open-channel attribution is a button when onOpenSource is provided", () => {
  const linkable = render({ onOpenSource: () => {} });
  assert.match(
    linkable,
    /<button[^>]*data-testid="forwarded-from-channel"[^>]*>#general/,
  );

  const plain = render();
  assert.doesNotMatch(
    plain,
    /<button[^>]*data-testid="forwarded-from-channel"/,
  );
  assert.match(plain, /data-testid="forwarded-from-channel"/);
});

test("private and dm sources get a non-linkable generic label", () => {
  const dm = render({ sourceType: "dm", sourceChannelName: null });
  assert.match(dm, /data-testid="forwarded-from-private"/);
  assert.match(dm, /Forwarded from(?:<!-- -->)?\s*a direct message/);
  assert.doesNotMatch(dm, /forwarded-from-channel/);
  assert.doesNotMatch(dm, /#general/);

  const priv = render({ sourceType: "private", sourceChannelName: null });
  assert.match(priv, /Forwarded from(?:<!-- -->)?\s*a private channel/);
});

test("falls back to a truncated pubkey when no profile is available", () => {
  const html = render({ authorDisplayName: null });
  // truncatePubkey: first 8 chars + ellipsis + last 4.
  assert.match(html, /953d5b1c…7081/);
});
