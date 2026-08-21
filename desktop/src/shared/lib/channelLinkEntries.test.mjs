import assert from "node:assert/strict";
import { test } from "node:test";

import {
  collectChannelLinkEntries,
  extractMessageLinkLabels,
  extractMessageLinks,
  linkHost,
  normalizeLinkKey,
  readableLinkName,
} from "./channelLinkEntries.mjs";

/** Build a source message with sensible defaults. */
function message(overrides = {}) {
  return {
    eventId: "e1",
    pubkey: "pub1",
    createdAt: 1000,
    content: "",
    hasAttachment: false,
    supersedes: null,
    ...overrides,
  };
}

// --- extraction -------------------------------------------------------------

test("finds a bare url in prose", () => {
  assert.deepEqual(
    extractMessageLinks("see https://example.com/report for detail"),
    ["https://example.com/report"],
  );
});

test("strips a sentence-ending full stop but keeps a file extension", () => {
  assert.deepEqual(extractMessageLinks("read https://example.com/a."), [
    "https://example.com/a",
  ]);
  assert.deepEqual(extractMessageLinks("read https://example.com/a.pdf"), [
    "https://example.com/a.pdf",
  ]);
});

test("does not swallow the closing paren of markdown link syntax", () => {
  assert.deepEqual(
    extractMessageLinks("[report](https://example.com/q3.pdf) is up"),
    ["https://example.com/q3.pdf"],
  );
});

test("returns each distinct url once, in order", () => {
  assert.deepEqual(
    extractMessageLinks(
      "https://b.com/two https://a.com/one https://b.com/two",
    ),
    ["https://b.com/two", "https://a.com/one"],
  );
});

test("ignores non-http schemes and plain text", () => {
  assert.deepEqual(extractMessageLinks("mailto:a@b.com and ftp://x.y/z"), []);
  assert.deepEqual(extractMessageLinks("no links here at all"), []);
  assert.deepEqual(extractMessageLinks(undefined), []);
});

// --- naming -----------------------------------------------------------------

test("names google surfaces by kind rather than by opaque id", () => {
  const cases = [
    ["https://docs.google.com/document/d/1a2B3c4D5e6F7g8H/edit", "Google Doc"],
    [
      "https://docs.google.com/spreadsheets/d/1a2B3c4D5e6F7g8H/edit#gid=0",
      "Google Sheet",
    ],
    [
      "https://docs.google.com/presentation/d/1a2B3c4D5e6F7g8H/edit",
      "Google Slides",
    ],
    [
      "https://drive.google.com/file/d/1a2B3c4D5e6F7g8H/view",
      "Google Drive file",
    ],
    [
      "https://drive.google.com/drive/folders/1a2B3c4D5e6F7g8H",
      "Google Drive folder",
    ],
  ];
  for (const [url, expected] of cases) {
    assert.equal(readableLinkName(url), expected, url);
  }
});

test("never returns a bare drive id", () => {
  const name = readableLinkName(
    "https://drive.google.com/file/d/1a2B3c4D5e6F7g8H9i/view",
  );
  assert.equal(name, "Google Drive file");
  assert.ok(!name.includes("1a2B3c"));
});

test("falls back to the last meaningful path segment", () => {
  assert.equal(
    readableLinkName("https://example.com/reports/q3-2026.pdf"),
    "q3-2026.pdf",
  );
  assert.equal(
    readableLinkName("https://www.notion.so/team/Q3%20Planning"),
    "Q3 Planning",
  );
});

test("skips opaque id segments in favour of a readable one", () => {
  assert.equal(
    readableLinkName("https://app.example.com/docs/Budget/8f14e45fceea167a"),
    "Budget",
  );
});

test("falls back to the host when the path says nothing", () => {
  assert.equal(readableLinkName("https://www.example.com/"), "example.com");
  assert.equal(
    readableLinkName("https://example.com/1a2B3c4D5e6F7g8H9i0j"),
    "example.com",
  );
});

test("short id-ish slugs are kept, not discarded as opaque", () => {
  assert.equal(readableLinkName("https://example.com/q3-plan"), "q3-plan");
});

test("returns null for anything unparseable", () => {
  assert.equal(readableLinkName("not a url"), null);
  assert.equal(readableLinkName(""), null);
  assert.equal(linkHost("not a url"), null);
});

test("host drops www", () => {
  assert.equal(linkHost("https://www.example.com/a"), "example.com");
});

// --- sender-supplied labels -------------------------------------------------

test("a markdown label is read as the link's name", () => {
  const [entry] = collectChannelLinkEntries({
    messages: [
      message({
        content:
          "here it is [Q3 Budget.xlsx](https://drive.google.com/file/d/1a2B3c4D5e6F7g8H/view)",
      }),
    ],
    excludedUrls: [],
  });
  assert.equal(entry.filename, "Q3 Budget.xlsx");
});

test("the label beats the Google-surface fallback", () => {
  const url = "https://docs.google.com/document/d/1a2B3c4D5e6F7g8H/edit";
  assert.equal(readableLinkName(url), "Google Doc");
  const [entry] = collectChannelLinkEntries({
    messages: [message({ content: `[Board notes](${url})` })],
    excludedUrls: [],
  });
  assert.equal(entry.filename, "Board notes");
});

test("an empty label falls back rather than producing a blank row", () => {
  const [entry] = collectChannelLinkEntries({
    messages: [message({ content: "[](https://example.com/reports/q3.pdf)" })],
    excludedUrls: [],
  });
  assert.equal(entry.filename, "q3.pdf");
});

test("labels are keyed by normalized url, so a trailing slash still matches", () => {
  const labels = extractMessageLinkLabels("[Plan](https://example.com/doc/)");
  assert.equal(labels.get(normalizeLinkKey("https://example.com/doc")), "Plan");
});

test("the first label wins when one url is labelled twice", () => {
  const labels = extractMessageLinkLabels(
    "[First](https://example.com/x) and [Second](https://example.com/x)",
  );
  assert.equal(labels.get(normalizeLinkKey("https://example.com/x")), "First");
});

test("a label on a later message does not rename an earlier bare link", () => {
  const [entry] = collectChannelLinkEntries({
    messages: [
      message({
        eventId: "first",
        createdAt: 1000,
        content: "https://example.com/reports/q3.pdf",
      }),
      message({
        eventId: "later",
        createdAt: 2000,
        content: "[Renamed](https://example.com/reports/q3.pdf)",
      }),
    ],
    excludedUrls: [],
  });
  assert.equal(entry.eventId, "first");
  assert.equal(entry.filename, "q3.pdf");
});

// --- identity ---------------------------------------------------------------

test("fragment, trailing slash and host case do not create a second entry", () => {
  const key = normalizeLinkKey("https://example.com/doc");
  assert.equal(normalizeLinkKey("https://EXAMPLE.com/doc/"), key);
  assert.equal(normalizeLinkKey("https://www.example.com/doc#section"), key);
});

test("the query string does distinguish two links", () => {
  assert.notEqual(
    normalizeLinkKey("https://example.com/s?gid=1"),
    normalizeLinkKey("https://example.com/s?gid=2"),
  );
});

// --- collection -------------------------------------------------------------

test("one entry per unique link, dated at its earliest appearance", () => {
  const entries = collectChannelLinkEntries({
    messages: [
      message({
        eventId: "later",
        createdAt: 2000,
        content: "again https://example.com/doc",
      }),
      message({
        eventId: "first",
        createdAt: 1000,
        content: "here https://example.com/doc",
      }),
    ],
    excludedUrls: [],
  });
  assert.equal(entries.length, 1);
  assert.equal(entries[0].eventId, "first");
  assert.equal(entries[0].uploadedAt, 1000);
});

test("a url already shared as an uploaded file does not become a link row", () => {
  const entries = collectChannelLinkEntries({
    messages: [
      message({ content: "![](https://relay.example/media/abc.png) look" }),
    ],
    excludedUrls: ["https://relay.example/media/abc.png"],
  });
  assert.deepEqual(entries, []);
});

test("exclusion ignores fragment and trailing-slash differences", () => {
  const entries = collectChannelLinkEntries({
    messages: [message({ content: "https://relay.example/media/abc/" })],
    excludedUrls: ["https://relay.example/media/abc"],
  });
  assert.deepEqual(entries, []);
});

test("entries carry the link shape the files list expects", () => {
  const [entry] = collectChannelLinkEntries({
    messages: [
      message({
        content: "https://example.com/reports/q3.pdf",
        pubkey: "author",
      }),
    ],
    excludedUrls: [],
  });
  assert.deepEqual(entry, {
    kind: "link",
    eventId: "e1",
    uploadedBy: "author",
    uploadedAt: 1000,
    filename: "q3.pdf",
    sha256: null,
    size: null,
    mime: null,
    url: "https://example.com/reports/q3.pdf",
    supersedes: null,
    supersededBy: null,
  });
});

test("a lone link carries its message's supersedes tag", () => {
  const [entry] = collectChannelLinkEntries({
    messages: [
      message({ content: "https://example.com/v2", supersedes: "older" }),
    ],
    excludedUrls: [],
  });
  assert.equal(entry.supersedes, "older");
});

test("two links in one message claim no supersedes between them", () => {
  const entries = collectChannelLinkEntries({
    messages: [
      message({
        content: "https://example.com/a and https://example.com/b",
        supersedes: "older",
      }),
    ],
    excludedUrls: [],
  });
  assert.equal(entries.length, 2);
  for (const entry of entries) assert.equal(entry.supersedes, null);
});

test("a link beside an attachment does not steal the file's supersedes tag", () => {
  const [entry] = collectChannelLinkEntries({
    messages: [
      message({
        content: "context https://example.com/ref",
        hasAttachment: true,
        supersedes: "older",
      }),
    ],
    excludedUrls: [],
  });
  assert.equal(entry.supersedes, null);
});

test("ordering of the input does not change which event wins a tie", () => {
  const forward = collectChannelLinkEntries({
    messages: [
      message({ eventId: "aaa", content: "https://example.com/x" }),
      message({ eventId: "bbb", content: "https://example.com/x" }),
    ],
    excludedUrls: [],
  });
  const reversed = collectChannelLinkEntries({
    messages: [
      message({ eventId: "bbb", content: "https://example.com/x" }),
      message({ eventId: "aaa", content: "https://example.com/x" }),
    ],
    excludedUrls: [],
  });
  assert.equal(forward[0].eventId, "aaa");
  assert.equal(reversed[0].eventId, "aaa");
});

test("empty and malformed input is tolerated", () => {
  assert.deepEqual(
    collectChannelLinkEntries({ messages: undefined, excludedUrls: undefined }),
    [],
  );
  assert.deepEqual(
    collectChannelLinkEntries({
      messages: [null, {}, message({ content: "no links" })],
      excludedUrls: [null],
    }),
    [],
  );
});
