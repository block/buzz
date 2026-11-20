import assert from "node:assert/strict";
import { test } from "node:test";

import { act, cleanup, fireEvent, render } from "@testing-library/react";
import { createElement, StrictMode } from "react";

import { POOF_TRIGGER_CLASS, PoofBurstProvider } from "./PoofBurstProvider.tsx";

test("poof audio stays untouched until a poof, then plays while decoding", async (t) => {
  const originalGlobals = new Map(
    ["AudioContext", "Audio", "fetch", "Image"].map((key) => [
      key,
      Object.getOwnPropertyDescriptor(globalThis, key),
    ]),
  );
  t.after(() => {
    cleanup();
    for (const [key, descriptor] of originalGlobals) {
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else delete globalThis[key];
    }
  });

  const soundUrl = "/pow/plop.m4a";
  const decodedBuffer = {};
  let finishDecode;
  const decode = t.mock.fn(
    () => new Promise((resolve) => (finishDecode = resolve)),
  );
  const start = t.mock.fn();
  const play = t.mock.fn(async () => {});
  const audio = t.mock.fn();
  const context = t.mock.fn();
  const images = [];
  const fetchSound = t.mock.fn(async () => ({
    ok: true,
    arrayBuffer: async () => new ArrayBuffer(0),
  }));
  Object.assign(globalThis, {
    AudioContext: class {
      state = "running";
      constructor() {
        context();
      }
      decodeAudioData = decode;
      createBufferSource() {
        return { connect() {}, start };
      }
      createGain() {
        return { gain: {}, connect() {} };
      }
    },
    Audio: class {
      constructor(url) {
        audio(url);
      }
      play = play;
    },
    Image: class {
      set src(value) {
        images.push(value);
      }
    },
    fetch: fetchSound,
  });

  const view = render(
    createElement(
      StrictMode,
      null,
      createElement(
        PoofBurstProvider,
        null,
        createElement(
          "button",
          { className: POOF_TRIGGER_CLASS, type: "button" },
          "Poof",
        ),
      ),
    ),
  );
  assert.equal(new Set(images).size, 5);
  fireEvent.click(document.body);
  assert.equal(context.mock.callCount(), 0);
  assert.equal(audio.mock.callCount(), 0);
  assert.equal(fetchSound.mock.callCount(), 0);
  assert.equal(decode.mock.callCount(), 0);

  await act(async () => fireEvent.click(view.getByText("Poof")));
  assert.equal(context.mock.callCount(), 1);
  assert.deepEqual(audio.mock.calls[0].arguments, [soundUrl]);
  assert.equal(play.mock.callCount(), 1);
  assert.deepEqual(fetchSound.mock.calls[0].arguments, [soundUrl]);
  assert.equal(decode.mock.callCount(), 1);
  assert.equal(start.mock.callCount(), 0);

  await act(async () => finishDecode(decodedBuffer));
  fireEvent.click(view.getByText("Poof"));
  assert.equal(start.mock.callCount(), 1);
  assert.equal(context.mock.callCount(), 1);
  assert.equal(audio.mock.callCount(), 1);
  assert.equal(fetchSound.mock.callCount(), 1);
});
