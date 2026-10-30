import assert from "node:assert/strict";
import test from "node:test";

import { visibleStackedAvatarCount } from "./ProjectsOverviewRail.tsx";

test("shows every stacked avatar when they fit in two rows", () => {
  assert.deepEqual(
    visibleStackedAvatarCount({
      avatarSize: 24,
      containerWidth: 288,
      overlap: 6,
      total: 30,
    }),
    { columns: 15, remaining: 0, visible: 30 },
  );
});

test("reserves the last stacked slot for a +N chip when avatars overflow two rows", () => {
  assert.deepEqual(
    visibleStackedAvatarCount({
      avatarSize: 24,
      containerWidth: 288,
      overlap: 6,
      total: 31,
    }),
    { columns: 15, remaining: 2, visible: 29 },
  );
});

test("waits for a measured width before clamping", () => {
  assert.deepEqual(
    visibleStackedAvatarCount({
      avatarSize: 24,
      containerWidth: 0,
      overlap: 6,
      total: 40,
    }),
    { columns: 40, remaining: 0, visible: 40 },
  );
});
