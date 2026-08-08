import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  clampComposerMaxHeight,
  DEFAULT_COMPOSER_MAX_HEIGHT_PX,
} from "./composerMaxHeight.ts";

describe("clampComposerMaxHeight", () => {
  it("never goes below the default 128px cap", () => {
    assert.equal(
      clampComposerMaxHeight(40, 800),
      DEFAULT_COMPOSER_MAX_HEIGHT_PX,
    );
    assert.equal(
      clampComposerMaxHeight(DEFAULT_COMPOSER_MAX_HEIGHT_PX, 800),
      DEFAULT_COMPOSER_MAX_HEIGHT_PX,
    );
  });

  it("allows raising the cap up to 60% of the pane", () => {
    assert.equal(clampComposerMaxHeight(300, 800), 300);
    assert.equal(clampComposerMaxHeight(900, 800), 480); // 0.6 * 800
  });

  it("uses the default as the upper bound when the pane is tiny", () => {
    assert.equal(
      clampComposerMaxHeight(400, 100),
      DEFAULT_COMPOSER_MAX_HEIGHT_PX,
    );
  });
});
