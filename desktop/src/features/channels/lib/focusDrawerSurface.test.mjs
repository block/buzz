import assert from "node:assert/strict";
import test from "node:test";

import { resolveFocusDrawerSurface } from "./focusDrawerSurface.ts";

const SPLIT_WITH_AGENT = {
  channelManagementOpen: false,
  hasAgentSession: true,
  hasThread: false,
  isFocusPreferred: true,
  useSplitAuxiliaryPane: true,
};

test("the selected agent surface is focus-eligible", () => {
  assert.equal(resolveFocusDrawerSurface(SPLIT_WITH_AGENT), "agent-session");
});

test("split remains the default presentation for both surfaces", () => {
  assert.equal(
    resolveFocusDrawerSurface({
      ...SPLIT_WITH_AGENT,
      isFocusPreferred: false,
    }),
    null,
  );
  assert.equal(
    resolveFocusDrawerSurface({
      ...SPLIT_WITH_AGENT,
      hasAgentSession: false,
      hasThread: true,
      isFocusPreferred: false,
    }),
    null,
  );
});

test("narrow and overlay presentations are unchanged by focus mode", () => {
  assert.equal(
    resolveFocusDrawerSurface({
      ...SPLIT_WITH_AGENT,
      useSplitAuxiliaryPane: false,
    }),
    null,
  );
  assert.equal(
    resolveFocusDrawerSurface({
      ...SPLIT_WITH_AGENT,
      hasAgentSession: false,
      hasThread: true,
      useSplitAuxiliaryPane: false,
    }),
    null,
  );
});

test("no drawer without a surface to put in it", () => {
  assert.equal(
    resolveFocusDrawerSurface({
      ...SPLIT_WITH_AGENT,
      hasAgentSession: false,
    }),
    null,
  );
});

test("precedence matches the render chain so the drawer cannot disagree with it", () => {
  // ChannelPane renders management → thread → agent session. A thread and an
  // agent session can both be resolvable at once (the panel keeps its selected
  // agent while a thread opens), and only the thread renders.
  assert.equal(
    resolveFocusDrawerSurface({
      ...SPLIT_WITH_AGENT,
      hasThread: true,
    }),
    "thread",
  );
  // Channel management keeps its split pane in every view mode, so it outranks
  // both drawers rather than being routed into one.
  assert.equal(
    resolveFocusDrawerSurface({
      ...SPLIT_WITH_AGENT,
      channelManagementOpen: true,
      hasThread: true,
    }),
    null,
  );
  assert.equal(
    resolveFocusDrawerSurface({
      ...SPLIT_WITH_AGENT,
      channelManagementOpen: true,
    }),
    null,
  );
});
