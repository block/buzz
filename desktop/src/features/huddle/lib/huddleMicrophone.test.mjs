import assert from "node:assert/strict";
import test from "node:test";
import { acquireHuddleMicrophone } from "./huddleMicrophone.ts";

const failure = (name, message = name) =>
  Object.assign(new Error(message), { name });
function stream() {
  const track = {
    stopped: false,
    stop() {
      this.stopped = true;
    },
  };
  return { track, getAudioTracks: () => [track], getTracks: () => [track] };
}
function devices(
  results,
  supported = {
    echoCancellation: true,
    noiseSuppression: true,
    sampleRate: true,
  },
) {
  const requests = [];
  return {
    requests,
    getSupportedConstraints: () => supported,
    getUserMedia: async (request) => {
      requests.push(request);
      const result = results.shift();
      if (result instanceof Error) throw result;
      return result;
    },
  };
}

test("supported preferences retain the selected microphone", async () => {
  const captured = stream();
  const input = devices([captured]);
  assert.equal(
    await acquireHuddleMicrophone(input, "usb", () => true),
    captured,
  );
  assert.deepEqual(input.requests, [
    {
      audio: {
        echoCancellation: true,
        noiseSuppression: true,
        sampleRate: 48000,
        deviceId: { exact: "usb" },
      },
    },
  ]);
});

test("unsupported preferences are omitted", async () => {
  const input = devices([stream()], { echoCancellation: true });
  await acquireHuddleMicrophone(input, "", () => true);
  assert.deepEqual(input.requests, [{ audio: { echoCancellation: true } }]);
});

test("WebKit constraint failures retry once with minimal default input", async () => {
  for (const error of [
    failure("OverconstrainedError"),
    failure("ConstraintNotSatisfiedError"),
    failure("Error", "Invalid constraint"),
  ]) {
    const captured = stream();
    const input = devices([error, captured]);
    assert.equal(
      await acquireHuddleMicrophone(input, "", () => true),
      captured,
    );
    assert.deepEqual(input.requests[1], { audio: true });
    assert.equal(input.requests.length, 2);
  }
});

test("minimal retry never switches away from an explicitly selected input", async () => {
  const captured = stream();
  const input = devices([failure("OverconstrainedError"), captured]);
  assert.equal(
    await acquireHuddleMicrophone(input, "usb", () => true),
    captured,
  );
  assert.deepEqual(input.requests[1], {
    audio: { deviceId: { exact: "usb" } },
  });
});

test("missing or unusable inputs leave capture unavailable for a listen-only huddle", async () => {
  for (const results of [
    [failure("NotFoundError")],
    [failure("OverconstrainedError"), failure("NotFoundError")],
    [failure("OverconstrainedError"), failure("OverconstrainedError")],
    [
      failure("Error", "Invalid constraint"),
      failure("Error", "Invalid constraint"),
    ],
  ]) {
    const input = devices(results);
    assert.equal(await acquireHuddleMicrophone(input, "", () => true), null);
    assert.ok(input.requests.length <= 2);
  }
});

test("permission and device failures propagate without retry", async () => {
  for (const name of [
    "NotAllowedError",
    "SecurityError",
    "NotReadableError",
    "AbortError",
  ]) {
    const error = failure(name);
    const input = devices([error]);
    await assert.rejects(
      acquireHuddleMicrophone(input, "", () => true),
      error,
    );
    assert.equal(input.requests.length, 1);
  }
});

test("permission refusal during fallback still propagates", async () => {
  const denied = failure("NotAllowedError");
  const input = devices([failure("OverconstrainedError"), denied]);
  await assert.rejects(
    acquireHuddleMicrophone(input, "", () => true),
    denied,
  );
  assert.equal(input.requests.length, 2);
});

test("a retired huddle cannot retry an acquisition failure", async () => {
  let current = true;
  const input = devices([failure("OverconstrainedError")]);
  const original = input.getUserMedia;
  input.getUserMedia = async (request) => {
    try {
      return await original(request);
    } finally {
      current = false;
    }
  };
  await assert.rejects(
    acquireHuddleMicrophone(input, "", () => current),
    /superseded/,
  );
  assert.equal(input.requests.length, 1);
});

test("a late microphone stream is stopped when its huddle has ended", async () => {
  let current = true;
  const captured = stream();
  const input = devices([captured]);
  input.getUserMedia = async () => {
    current = false;
    return captured;
  };
  await assert.rejects(
    acquireHuddleMicrophone(input, "", () => current),
    /superseded/,
  );
  assert.equal(captured.track.stopped, true);
});
