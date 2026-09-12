import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

function processor() {
  let Audio;
  const events = [];
  const scope = {
    sampleRate: 24000,
    AudioWorkletProcessor: class {
      constructor() {
        this.port = { postMessage: (e) => events.push(e) };
      }
    },
    registerProcessor: (_, type) => {
      Audio = type;
    },
  };
  vm.runInNewContext(
    readFileSync(new URL("./audio-worklet.mjs", import.meta.url), "utf8"),
    scope,
  );
  const instance = new Audio();
  return {
    instance,
    events,
    send: (data) => instance.port.onmessage({ data }),
  };
}
const audio = (seconds, itemId = "one") => ({
  type: "audio",
  responseId: itemId,
  itemId,
  pcm: new Int16Array(24000 * seconds).fill(1000),
});

test("a valid fast response can queue beyond three seconds and remain interruptible", () => {
  const { instance, events, send } = processor();
  send(audio(10));
  assert.equal(events.length, 0);
  assert.equal(instance.queued, 240000);
  send({ type: "capture", enabled: true });
  const output = new Float32Array(2400);
  instance.process([[new Float32Array(2400)]], [[output]]);
  assert.ok(output[0] > 0);
  assert.ok(events.some((e) => e.type === "capture"));
  send({ type: "clear", requestId: 7 });
  assert.equal(instance.queued, 0);
  const stopped = events.find((e) => e.type === "stopped");
  assert.equal(stopped.playback.playedSamples, 2400);
  send(audio(1));
  assert.equal(instance.queued, 0);
  send(audio(1, "two"));
  assert.equal(instance.queued, 24000);
});

test("playback retains a hard thirty-second cap", () => {
  const { instance, events, send } = processor();
  send(audio(30));
  assert.equal(events.length, 0);
  send(audio(1));
  assert.ok(events.some((e) => e.type === "error"));
  assert.equal(instance.queued, 0);
});

test("idle playback does not report the previous response as a new first sound", () => {
  const { instance, events, send } = processor();
  const output = new Float32Array(2400);
  send(audio(0.1));
  instance.process([], [[output]]);
  for (let i = 0; i < 10; i++) instance.process([], [[output]]);
  assert.equal(events.filter((e) => e.type === "position").length, 1);
  send(audio(0.1, "two"));
  instance.process([], [[output]]);
  const positions = events.filter((e) => e.type === "position");
  assert.deepEqual(
    positions.map((e) => e.playback.itemId),
    ["one", "two"],
  );
  send({ type: "clear", requestId: 9 });
  assert.equal(events.find((e) => e.type === "stopped").playback.itemId, "two");
});

test("duplex capture reports exactly rendered audio, including underruns and backchannels", () => {
  const { instance, events, send } = processor();
  send({ type: "capture", enabled: true });
  send({
    type: "backchannel",
    id: "aside",
    pcm: new Int16Array(240).fill(3276),
  });
  const out = new Float32Array(480);
  instance.process([[new Float32Array(480).fill(0.2)]], [[out]]);
  const capture = events.find((e) => e.type === "capture");
  assert.equal(capture.pcm.length, 480);
  assert.equal(capture.playback.length, 480);
  assert.deepEqual(
    Array.from(capture.playback.slice(0, 240)),
    Array(240).fill(3276),
  );
  assert.deepEqual(Array.from(capture.playback.slice(240)), Array(240).fill(0));
  assert.ok(capture.pcm.every((x) => x === 6553));
  // A normal reply supersedes an aside and late aside frames stay suppressed.
  send({
    type: "backchannel",
    id: "aside",
    pcm: new Int16Array(480).fill(3276),
  });
  send(audio(1));
  send({
    type: "backchannel",
    id: "aside",
    pcm: new Int16Array(480).fill(3276),
  });
  instance.process([[new Float32Array(480)]], [[out]]);
  assert.ok(out.every((x) => x === 1000 / 32768));
});

test("playback gap diagnostics measure inserted silence without counting idle time between replies", () => {
  const { instance, events, send } = processor();
  const output = new Float32Array(480);
  send(audio(0.01));
  instance.process([], [[output]]);
  assert.equal(events.filter((e) => e.type === "playback_gap").length, 0);
  send(audio(0.02));
  instance.process([], [[output]]);
  const gaps = events.filter((e) => e.type === "playback_gap");
  assert.equal(gaps.length, 1);
  assert.equal(gaps[0].missingSamples, 240);
  assert.equal(gaps[0].playedSamples, 240);
  assert.ok(output.every((v) => v === 1000 / 32768));
  for (let i = 0; i < 10; i++) instance.process([], [[output]]);
  send(audio(0.02, "two"));
  instance.process([], [[output]]);
  send({ type: "clear", requestId: 3 });
  instance.process([], [[output]]);
  send(audio(0.02, "three"));
  instance.process([], [[output]]);
  assert.equal(events.filter((e) => e.type === "playback_gap").length, 1);
});
