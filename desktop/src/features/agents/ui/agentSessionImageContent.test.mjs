import assert from "node:assert/strict";
import test from "node:test";

import { buildImageContent } from "./agentSessionImageContent.ts";

const imageDescriptor = {
  renderClass: "image",
  label: "Viewed image",
  preview: "screenshot.png",
  groupKey: "view_image",
};

function makeTool(overrides = {}) {
  return {
    id: "tool:1",
    type: "tool",
    title: "view_image",
    toolName: "view_image",
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "",
    isError: false,
    timestamp: "2026-06-14T19:00:00.000Z",
    startedAt: "2026-06-14T19:00:00.000Z",
    completedAt: "2026-06-14T19:00:01.000Z",
    descriptor: imageDescriptor,
    ...overrides,
  };
}

test("buildImageContent returns null for non image render class", () => {
  assert.equal(
    buildImageContent(makeTool(), {
      ...imageDescriptor,
      renderClass: "generic",
    }),
    null,
  );
});

test("buildImageContent accepts http and data image sources", () => {
  const http = buildImageContent(
    makeTool({
      args: { source: "https://example.com/image.png" },
    }),
    imageDescriptor,
  );
  assert.deepEqual(http, {
    src: "https://example.com/image.png",
    title: "screenshot.png",
  });

  const data = buildImageContent(
    makeTool({
      args: { source: "data:image/png;base64,abc" },
    }),
    imageDescriptor,
  );
  assert.equal(data?.src, "data:image/png;base64,abc");
});

test("buildImageContent rejects local filesystem paths", () => {
  assert.equal(
    buildImageContent(
      makeTool({ args: { source: "desktop/assets/screenshot.png" } }),
      imageDescriptor,
    ),
    null,
  );
});
