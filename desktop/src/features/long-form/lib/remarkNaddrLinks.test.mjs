import assert from "node:assert/strict";
import test from "node:test";

import { nip19 } from "nostr-tools";

import remarkNaddrLinks from "./remarkNaddrLinks.ts";

const PUBKEY = "1".repeat(64);

function naddr(kind = 30023, identifier = "hello") {
  return `nostr:${nip19.naddrEncode({ identifier, kind, pubkey: PUBKEY })}`;
}

function runPlugin(tree) {
  remarkNaddrLinks()(tree);
  return tree;
}

function paragraph(...children) {
  return { type: "root", children: [{ type: "paragraph", children }] };
}

function text(value) {
  return { type: "text", value };
}

test("remarkNaddrLinks: bare valid long-form naddr becomes a Markdown link", () => {
  const value = naddr();
  const tree = runPlugin(paragraph(text(value)));
  const kids = tree.children[0].children;

  assert.equal(kids.length, 1);
  assert.equal(kids[0].type, "link");
  assert.equal(kids[0].url, value);
  assert.deepEqual(kids[0].children, [{ type: "text", value }]);
});

test("remarkNaddrLinks: mid-sentence naddr splits surrounding text", () => {
  const value = naddr();
  const tree = runPlugin(paragraph(text(`see ${value} here`)));
  const kids = tree.children[0].children;

  assert.equal(kids.length, 3);
  assert.equal(kids[0].value, "see ");
  assert.equal(kids[1].type, "link");
  assert.equal(kids[1].url, value);
  assert.equal(kids[2].value, " here");
});

test("remarkNaddrLinks: trailing sentence punctuation stays outside the URL", () => {
  const value = naddr();
  for (const punctuation of [".", ",", ";", ":", "!", "?"]) {
    const tree = runPlugin(paragraph(text(`read ${value}${punctuation}`)));
    const kids = tree.children[0].children;

    assert.equal(kids.length, 3, punctuation);
    assert.equal(kids[1].type, "link", punctuation);
    assert.equal(kids[1].url, value, punctuation);
    assert.equal(kids[2].value, punctuation, punctuation);
  }
});

test("remarkNaddrLinks: URL inside parens keeps closing paren outside", () => {
  const value = naddr();
  const tree = runPlugin(paragraph(text(`read (${value}) today`)));
  const kids = tree.children[0].children;

  assert.equal(kids.length, 3);
  assert.equal(kids[0].value, "read (");
  assert.equal(kids[1].type, "link");
  assert.equal(kids[1].url, value);
  assert.equal(kids[2].value, ") today");
});

test("remarkNaddrLinks: unsupported Nostr entities are left as text", () => {
  const nsec = `nostr:${nip19.nsecEncode(new Uint8Array(32).fill(1))}`;
  const tree = runPlugin(paragraph(text(nsec)));
  const kids = tree.children[0].children;

  assert.equal(kids.length, 1);
  assert.equal(kids[0].type, "text");
  assert.equal(kids[0].value, nsec);
});

test("remarkNaddrLinks: non-long-form naddr is left as text", () => {
  const value = naddr(30024);
  const tree = runPlugin(paragraph(text(value)));
  const kids = tree.children[0].children;

  assert.equal(kids.length, 1);
  assert.equal(kids[0].type, "text");
  assert.equal(kids[0].value, value);
});

test("remarkNaddrLinks: inline and fenced code are left alone", () => {
  const value = naddr();
  const tree = {
    type: "root",
    children: [
      {
        type: "paragraph",
        children: [{ type: "inlineCode", value }],
      },
      { type: "code", value },
    ],
  };
  runPlugin(tree);

  assert.equal(tree.children[0].children[0].type, "inlineCode");
  assert.equal(tree.children[0].children[0].value, value);
  assert.equal(tree.children[1].type, "code");
  assert.equal(tree.children[1].value, value);
});
