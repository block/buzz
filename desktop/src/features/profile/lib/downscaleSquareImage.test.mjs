import assert from "node:assert/strict";
import test from "node:test";

import { downscaleSquareImageToDataUrl } from "./downscaleSquareImage.ts";

async function withImageEnvironment({ bitmap, context, toDataURL }, run) {
  const bitmapDescriptor = Object.getOwnPropertyDescriptor(
    globalThis,
    "createImageBitmap",
  );
  const documentDescriptor = Object.getOwnPropertyDescriptor(
    globalThis,
    "document",
  );
  const canvas = {
    height: 0,
    width: 0,
    getContext: () => context,
    toDataURL,
  };
  Object.defineProperty(globalThis, "createImageBitmap", {
    configurable: true,
    value: async () => bitmap,
  });
  Object.defineProperty(globalThis, "document", {
    configurable: true,
    value: {
      createElement: (tag) => {
        assert.equal(tag, "canvas");
        return canvas;
      },
    },
  });
  try {
    await run(canvas);
  } finally {
    if (bitmapDescriptor) {
      Object.defineProperty(globalThis, "createImageBitmap", bitmapDescriptor);
    } else {
      delete globalThis.createImageBitmap;
    }
    if (documentDescriptor) {
      Object.defineProperty(globalThis, "document", documentDescriptor);
    } else {
      delete globalThis.document;
    }
  }
}

test("center-crops and downsizes a landscape image to 128px WebP", async () => {
  const drawCalls = [];
  const encodes = [];
  let closed = false;
  const bitmap = {
    height: 200,
    width: 400,
    close: () => {
      closed = true;
    },
  };
  const context = {
    imageSmoothingQuality: "low",
    drawImage: (...args) => drawCalls.push(args),
  };

  await withImageEnvironment(
    {
      bitmap,
      context,
      toDataURL: (type, quality) => {
        encodes.push([type, quality]);
        return "data:image/webp;base64,processed";
      },
    },
    async (canvas) => {
      assert.equal(
        await downscaleSquareImageToDataUrl({ name: "avatar.png" }),
        "data:image/webp;base64,processed",
      );
      assert.equal(canvas.width, 128);
      assert.equal(canvas.height, 128);
    },
  );

  assert.equal(context.imageSmoothingQuality, "high");
  assert.deepEqual(drawCalls, [[bitmap, 100, 0, 200, 200, 0, 0, 128, 128]]);
  assert.deepEqual(encodes, [["image/webp", 0.85]]);
  assert.equal(closed, true);
});

test("falls back to PNG when the WebView cannot encode WebP", async () => {
  const formats = [];
  const bitmap = { height: 120, width: 120, close() {} };
  const context = { drawImage() {}, imageSmoothingQuality: "low" };

  await withImageEnvironment(
    {
      bitmap,
      context,
      toDataURL: (type) => {
        formats.push(type);
        return type === "image/png"
          ? "data:image/png;base64,processed"
          : "data:image/png;base64,webp-unsupported";
      },
    },
    async () => {
      assert.equal(
        await downscaleSquareImageToDataUrl({ name: "avatar.png" }),
        "data:image/png;base64,processed",
      );
    },
  );

  assert.deepEqual(formats, ["image/webp", "image/png"]);
});

test("closes the decoded bitmap when canvas processing fails", async () => {
  let closed = false;
  const bitmap = {
    height: 120,
    width: 120,
    close: () => {
      closed = true;
    },
  };

  await withImageEnvironment(
    {
      bitmap,
      context: null,
      toDataURL: () => "",
    },
    async () => {
      await assert.rejects(
        downscaleSquareImageToDataUrl({ name: "avatar.png" }),
        /Could not process that image/,
      );
    },
  );

  assert.equal(closed, true);
});
