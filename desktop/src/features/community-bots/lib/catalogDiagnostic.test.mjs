import assert from "node:assert/strict";
import test from "node:test";

import { buildCatalogDiagnostic } from "./catalogDiagnostic.ts";

const ADMIN = "aa".repeat(32);
const ME = "bb".repeat(32);
const BOT = "cc".repeat(32);
const catalog = (pubkey, createdAt, bots) => ({
  id: "e".repeat(64),
  kind: 30624,
  pubkey,
  created_at: createdAt,
  tags: [["d", "buzz:community-bots"]],
  content: JSON.stringify({ version: 1, bots }),
  sig: "",
});
const members = {
  id: "m".repeat(64),
  kind: 13534,
  pubkey: ADMIN,
  created_at: 1,
  tags: [["member", ADMIN, "owner"], ["member", ME, "member"]],
  content: "",
  sig: "",
};

test("no catalog event", () => {
  const d = buildCatalogDiagnostic({ events: [], membershipEvent: members, myPubkey: ME });
  assert.equal(d.event, null);
  assert.equal(d.myRole, "member");
  assert.match(d.hint, /owner or an admin/);
});

test("latest catalog, author role, bot count", () => {
  const d = buildCatalogDiagnostic({
    events: [
      catalog(ADMIN, 1, []),
      catalog(ADMIN, 5, [{ id: "a", name: "A", pubkey: BOT, source: "openclaw" }, { id: "bad" }]),
    ],
    membershipEvent: members,
    myPubkey: ADMIN,
  });
  assert.equal(d.event.createdAt, 5);
  assert.equal(d.event.botCount, 1);
  assert.equal(d.event.rawBotEntries, 2);
  assert.equal(d.authorRole, "owner");
  assert.equal(d.myRole, "owner");
  assert.ok(d.event.authorNpub.startsWith("npub1"));
});

test("membership unreadable gives unknown roles", () => {
  const d = buildCatalogDiagnostic({ events: [catalog(ADMIN, 2, [])], membershipEvent: null, myPubkey: ME });
  assert.equal(d.authorRole, null);
  assert.equal(d.myRole, null);
  assert.equal(d.membershipReadable, false);
});
