import assert from "node:assert/strict";
import test from "node:test";

import { selectInlineVisibleCount } from "./transcriptRoots.ts";

const AGENT = "agent-pubkey";
const HUMAN = "human-pubkey";

function reply(pubkey) {
  return { pubkey };
}

const isAgent = (pubkey) => pubkey === AGENT;

test("selectInlineVisibleCount_showsTheWholeLeadingAgentRun", () => {
  const replies = [reply(AGENT), reply(AGENT), reply(AGENT)];
  assert.equal(selectInlineVisibleCount(replies, isAgent), 3);
});

test("selectInlineVisibleCount_collapsesFromTheFirstHumanReply", () => {
  const replies = [reply(AGENT), reply(AGENT), reply(HUMAN), reply(AGENT)];
  assert.equal(selectInlineVisibleCount(replies, isAgent), 2);
});

test("selectInlineVisibleCount_humanFirstThreadStillShowsItsFirstReply", () => {
  const replies = [reply(HUMAN), reply(AGENT)];
  assert.equal(selectInlineVisibleCount(replies, isAgent), 1);
});

test("selectInlineVisibleCount_emptyThreadShowsNothing", () => {
  assert.equal(selectInlineVisibleCount([], isAgent), 0);
});
