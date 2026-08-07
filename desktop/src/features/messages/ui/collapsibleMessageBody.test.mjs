import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  COLLAPSED_MESSAGE_MAX_HEIGHT_PX,
  messageBodyNeedsClamp,
  shouldForceExpandMessageBody,
} from "./collapsibleMessageBody.ts";

describe("collapsibleMessageBody", () => {
  it("clamps only when content exceeds the max by more than 1px", () => {
    assert.equal(messageBodyNeedsClamp(COLLAPSED_MESSAGE_MAX_HEIGHT_PX), false);
    assert.equal(
      messageBodyNeedsClamp(COLLAPSED_MESSAGE_MAX_HEIGHT_PX + 1),
      false,
    );
    assert.equal(
      messageBodyNeedsClamp(COLLAPSED_MESSAGE_MAX_HEIGHT_PX + 2),
      true,
    );
  });

  it("force-expands for route highlights and non-empty search", () => {
    assert.equal(shouldForceExpandMessageBody({ highlighted: true }), true);
    assert.equal(
      shouldForceExpandMessageBody({ searchQuery: "  checkout  " }),
      true,
    );
    assert.equal(shouldForceExpandMessageBody({ searchQuery: "   " }), false);
    assert.equal(shouldForceExpandMessageBody({}), false);
  });
});
