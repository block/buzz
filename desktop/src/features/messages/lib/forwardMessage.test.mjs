import assert from "node:assert/strict";
import test from "node:test";

import {
  FORWARDABLE_SOURCE_KINDS,
  MAX_FWD_TAG_BYTES,
  buildForwardTags,
  canForwardMessageKind,
  forwardSourceTypeForChannel,
  getEventChannelId,
  parseForwardEnvelope,
  resolveForwardOriginal,
} from "./forwardMessage.ts";

const SOURCE_CHANNEL = "f570339f-8f8a-4e08-a779-8d954aa44109";
const ORIGINAL_ID =
  "b04819ffc1f7c8ffb49c6d30b5899f470198264680d02e78894a658e30a9059f";
const ORIGINAL_PUBKEY =
  "953d5b1c9f0c1d4e8a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f7081";
const SIG = "ab".repeat(64);

function makeOriginal(overrides = {}) {
  return {
    id: ORIGINAL_ID,
    pubkey: ORIGINAL_PUBKEY,
    created_at: 1753600000,
    kind: 9,
    tags: [["h", SOURCE_CHANNEL]],
    content: "hello world",
    sig: SIG,
    ...overrides,
  };
}

test("canForwardMessageKind allows the source allowlist plus 40009", () => {
  for (const kind of [9, 40002, 45001, 45003, 40009]) {
    assert.equal(canForwardMessageKind(kind), true, `kind ${kind}`);
  }
  for (const kind of [undefined, 1, 7, 40008, 40099, 39000]) {
    assert.equal(canForwardMessageKind(kind), false, `kind ${kind}`);
  }
});

test("forwardSourceTypeForChannel maps dm/open/closed correctly", () => {
  assert.equal(
    forwardSourceTypeForChannel({ channelType: "dm", visibility: "private" }),
    "dm",
  );
  assert.equal(
    forwardSourceTypeForChannel({
      channelType: "channel",
      visibility: "open",
    }),
    "channel",
  );
  assert.equal(
    forwardSourceTypeForChannel({
      channelType: "channel",
      visibility: "private",
    }),
    "private",
  );
});

test("getEventChannelId reads the h tag and rejects empty values", () => {
  assert.equal(getEventChannelId(makeOriginal()), SOURCE_CHANNEL);
  assert.equal(getEventChannelId({ tags: [] }), null);
  assert.equal(getEventChannelId({ tags: [["h", ""]] }), null);
});

test("buildForwardTags emits fwd/k/fwd-src and q for open sources", () => {
  const original = makeOriginal();
  const tags = buildForwardTags({ original, sourceType: "channel" });

  const fwd = tags.filter((tag) => tag[0] === "fwd");
  assert.equal(fwd.length, 1);
  const embedded = JSON.parse(fwd[0][1]);
  // Exactly the signed NIP-01 fields, nothing local.
  assert.deepEqual(Object.keys(embedded).sort(), [
    "content",
    "created_at",
    "id",
    "kind",
    "pubkey",
    "sig",
    "tags",
  ]);
  assert.deepEqual(embedded, {
    id: ORIGINAL_ID,
    pubkey: ORIGINAL_PUBKEY,
    created_at: 1753600000,
    kind: 9,
    tags: [["h", SOURCE_CHANNEL]],
    content: "hello world",
    sig: SIG,
  });

  assert.deepEqual(
    tags.find((tag) => tag[0] === "k"),
    ["k", "9"],
  );
  // fwd-src uuid comes from the ORIGINAL's own h tag.
  assert.deepEqual(
    tags.find((tag) => tag[0] === "fwd-src"),
    ["fwd-src", SOURCE_CHANNEL, "channel"],
  );
  assert.deepEqual(
    tags.find((tag) => tag[0] === "q"),
    ["q", ORIGINAL_ID, "", ORIGINAL_PUBKEY],
  );
});

test("buildForwardTags never leaks local-only fields into the snapshot", () => {
  const original = makeOriginal({ localKey: "local-1", pending: true });
  const tags = buildForwardTags({ original, sourceType: "channel" });
  const embedded = JSON.parse(tags.find((tag) => tag[0] === "fwd")[1]);
  assert.equal("localKey" in embedded, false);
  assert.equal("pending" in embedded, false);
});

test("buildForwardTags omits q for private and dm sources", () => {
  for (const sourceType of ["private", "dm"]) {
    const tags = buildForwardTags({ original: makeOriginal(), sourceType });
    assert.equal(
      tags.some((tag) => tag[0] === "q"),
      false,
      sourceType,
    );
    assert.deepEqual(
      tags.find((tag) => tag[0] === "fwd-src"),
      ["fwd-src", SOURCE_CHANNEL, sourceType],
    );
  }
});

test("buildForwardTags copies imeta tags verbatim", () => {
  const imeta = [
    "imeta",
    "url https://relay.example/media/abc.png",
    "m image/png",
    "dim 640x480",
  ];
  const original = makeOriginal({
    tags: [["h", SOURCE_CHANNEL], imeta, ["p", ORIGINAL_PUBKEY]],
  });
  const tags = buildForwardTags({ original, sourceType: "channel" });
  const copied = tags.filter((tag) => tag[0] === "imeta");
  assert.equal(copied.length, 1);
  assert.deepEqual(copied[0], imeta);
  assert.notEqual(copied[0], imeta, "must be a copy, not the same array");
  // Non-imeta original tags (p/h/…) must NOT be copied to the top level.
  assert.equal(
    tags.some((tag) => tag[0] === "p"),
    false,
  );
  assert.equal(
    tags.some((tag) => tag[0] === "h"),
    false,
  );
});

test("buildForwardTags rejects non-allowlisted kinds", () => {
  assert.throws(
    () =>
      buildForwardTags({
        original: makeOriginal({ kind: 40009 }),
        sourceType: "channel",
      }),
    /cannot be forwarded/,
  );
  assert.throws(
    () =>
      buildForwardTags({
        original: makeOriginal({ kind: 1 }),
        sourceType: "channel",
      }),
    /cannot be forwarded/,
  );
});

test("buildForwardTags rejects a missing signature", () => {
  assert.throws(
    () =>
      buildForwardTags({
        original: makeOriginal({ sig: "" }),
        sourceType: "channel",
      }),
    /missing its signature/,
  );
});

test("buildForwardTags rejects an original without a source channel", () => {
  assert.throws(
    () =>
      buildForwardTags({
        original: makeOriginal({ tags: [] }),
        sourceType: "channel",
      }),
    /no source channel/,
  );
});

test("buildForwardTags rejects oversize originals (64 KiB ceiling)", () => {
  const original = makeOriginal({
    content: "x".repeat(MAX_FWD_TAG_BYTES + 1),
  });
  assert.throws(
    () => buildForwardTags({ original, sourceType: "channel" }),
    /too large to forward/,
  );
});

test("parseForwardEnvelope round-trips buildForwardTags output", () => {
  const original = makeOriginal();
  const tags = buildForwardTags({ original, sourceType: "private" });
  const envelope = parseForwardEnvelope(tags);
  assert.ok(envelope);
  assert.deepEqual(envelope.original, original);
  assert.equal(envelope.sourceChannelId, SOURCE_CHANNEL);
  assert.equal(envelope.sourceType, "private");
});

test("parseForwardEnvelope rejects malformed tag sets", () => {
  const good = buildForwardTags({
    original: makeOriginal(),
    sourceType: "channel",
  });

  // No fwd tag at all.
  assert.equal(
    parseForwardEnvelope(good.filter((tag) => tag[0] !== "fwd")),
    null,
  );
  // Two fwd tags.
  assert.equal(
    parseForwardEnvelope([...good, good.find((tag) => tag[0] === "fwd")]),
    null,
  );
  // Unparseable embedded JSON.
  assert.equal(
    parseForwardEnvelope([
      ["fwd", "{not json"],
      ["fwd-src", SOURCE_CHANNEL, "channel"],
    ]),
    null,
  );
  // Embedded event missing the signature field.
  const unsigned = { ...makeOriginal() };
  delete unsigned.sig;
  assert.equal(
    parseForwardEnvelope([
      ["fwd", JSON.stringify(unsigned)],
      ["fwd-src", SOURCE_CHANNEL, "channel"],
    ]),
    null,
  );
  // Missing fwd-src.
  assert.equal(
    parseForwardEnvelope(good.filter((tag) => tag[0] !== "fwd-src")),
    null,
  );
  // fwd-src with an unknown visibility label.
  assert.equal(
    parseForwardEnvelope([
      good.find((tag) => tag[0] === "fwd"),
      ["fwd-src", SOURCE_CHANNEL, "public"],
    ]),
    null,
  );
});

test("resolveForwardOriginal flattens a 40009 to its embedded original", () => {
  const original = makeOriginal();
  const forwardEvent = {
    id: "c1".repeat(32),
    pubkey: "d2".repeat(32),
    created_at: 1753600100,
    kind: 40009,
    tags: buildForwardTags({ original, sourceType: "channel" }),
    content: "check this out",
    sig: "ef".repeat(64),
  };
  assert.deepEqual(resolveForwardOriginal(forwardEvent), original);
});

test("resolveForwardOriginal returns allowlisted events as-is, null otherwise", () => {
  const original = makeOriginal({ kind: 45001 });
  assert.equal(resolveForwardOriginal(original), original);
  assert.equal(resolveForwardOriginal(makeOriginal({ kind: 40008 })), null);
  // A 40009 with a malformed envelope cannot be re-forwarded.
  assert.equal(
    resolveForwardOriginal(makeOriginal({ kind: 40009, tags: [] })),
    null,
  );
});

test("FORWARDABLE_SOURCE_KINDS matches the relay allowlist", () => {
  assert.deepEqual(
    [...FORWARDABLE_SOURCE_KINDS].sort((a, b) => a - b),
    [9, 40002, 45001, 45003],
  );
});
