import assert from "node:assert/strict";
import test from "node:test";

import {
  applyReservedCommunityMentionRouting,
  HULA_RESERVED_COMMUNITY_BOT_PUBKEYS,
  reservedCommunityBotRoutes,
  rewriteSelectedMentionsForReservedRoutes,
} from "./reservedCommunityMentionRouting.ts";

const ORPHAN = "c".repeat(64);
const MO = HULA_RESERVED_COMMUNITY_BOT_PUBKEYS.Mo.toLowerCase();

test("routes prefer channel over catalog over Hula fallback", () => {
  const catalogMo = "a".repeat(64);
  const channelMo = "b".repeat(64);
  const routes = reservedCommunityBotRoutes({
    catalogBots: [{ name: "Mo", pubkey: catalogMo }],
    channelBots: [{ name: "Mo", pubkey: channelMo }],
  });
  assert.equal(routes.get("mo"), channelMo);

  const channelOnly = reservedCommunityBotRoutes({
    channelBots: [{ name: "Mo", pubkey: channelMo }],
    useHulaFallback: false,
  });
  assert.equal(channelOnly.get("mo"), channelMo);

  const fallback = reservedCommunityBotRoutes({});
  assert.equal(fallback.get("mo"), MO);
  assert.equal(
    fallback.get("captain"),
    HULA_RESERVED_COMMUNITY_BOT_PUBKEYS.Captain.toLowerCase(),
  );
});

test("routing drops orphan competitors for reserved exact labels", () => {
  const routes = reservedCommunityBotRoutes({});
  const filtered = applyReservedCommunityMentionRouting(
    [
      { displayName: "Mo", pubkey: ORPHAN },
      { displayName: "Mo", pubkey: MO },
      { displayName: "Mo Local", pubkey: ORPHAN },
      { displayName: "Ada", pubkey: "d".repeat(64) },
    ],
    routes,
  );
  assert.equal(filtered.length, 3);
  assert.equal(filtered.find((c) => c.displayName === "Mo")?.pubkey, MO);
  assert.ok(filtered.some((c) => c.displayName === "Mo Local"));
  assert.ok(filtered.some((c) => c.displayName === "Ada"));
  assert.ok(
    !filtered.some((c) => c.displayName === "Mo" && c.pubkey === ORPHAN),
  );
});

test("selected mention rewrite forces reserved labels onto community bots", () => {
  const routes = reservedCommunityBotRoutes({});
  const rewritten = rewriteSelectedMentionsForReservedRoutes(
    new Map([
      ["Mo", ORPHAN],
      ["Ada", "d".repeat(64)],
    ]),
    routes,
  );
  assert.equal(rewritten.get("Mo"), MO);
  assert.equal(rewritten.get("Ada"), "d".repeat(64));
});
