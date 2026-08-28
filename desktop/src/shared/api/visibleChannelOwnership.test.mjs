import assert from "node:assert/strict";
import test from "node:test";

import { createVisibleChannelOwnership } from "./visibleChannelOwnership.ts";

test("releasing a newer surface restores the still-visible older surface", () => {
  const visible = [];
  const ownership = createVisibleChannelOwnership((channelId) =>
    visible.push(channelId),
  );

  const releaseMain = ownership.acquire("main");
  const releaseBestie = ownership.acquire("bestie");
  releaseBestie();
  releaseMain();

  assert.deepEqual(visible, ["main", "bestie", "main", null]);
});

test("same-channel consumers cannot clear each other's visible marker", () => {
  const visible = [];
  const ownership = createVisibleChannelOwnership((channelId) =>
    visible.push(channelId),
  );

  const releaseMain = ownership.acquire("bestie");
  const releasePopover = ownership.acquire("bestie");
  releaseMain();
  releasePopover();

  assert.deepEqual(visible, ["bestie", "bestie", "bestie", null]);
});
