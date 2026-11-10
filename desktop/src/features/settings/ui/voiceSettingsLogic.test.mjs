import assert from "node:assert/strict";
import test from "node:test";

import {
  selectedVoiceForBackend,
  voiceOptionLabel,
  voicesForBackend,
} from "./voiceSettingsLogic.ts";

const voice = (key, displayName, fallbackKey = "pocket:mary") => ({
  key,
  displayName,
  backend: "pocket",
  backendName: "Pocket TTS",
  availability: "bundled",
  fallbackKey,
  referenceFile: `${key}.wav`,
  provenance: {
    source: "bundled",
    contentHash: null,
    license: null,
    sourceUrl: null,
  },
});

test("Pocket-only V1 filters the shared registry by backend", () => {
  const registry = [
    voice("pocket:mary", "Mary", null),
    { ...voice("siri:aaron", "Aaron"), backend: "siri" },
  ];
  assert.deepEqual(
    voicesForBackend(registry, "pocket").map((entry) => entry.key),
    ["pocket:mary"],
  );
});

test("local selection uses the first compatible qualified preference", () => {
  const voices = [
    voice("pocket:mary", "Mary", null),
    voice("pocket:eve", "Eve"),
  ];
  assert.equal(
    selectedVoiceForBackend(["siri:aaron", "pocket:eve", "pocket:mary"], voices)
      ?.key,
    "pocket:eve",
  );
});

test("duplicate display labels remain distinct by content-derived key", () => {
  const voices = [
    voice("pocket:imported:aaa", "Jim"),
    voice("pocket:imported:bbb", "Jim"),
  ];
  assert.equal(
    selectedVoiceForBackend(["pocket:imported:bbb"], voices)?.key,
    "pocket:imported:bbb",
  );
  assert.equal(voiceOptionLabel(voices[0], voices), "Jim · aaa");
  assert.equal(voiceOptionLabel(voices[1], voices), "Jim · bbb");
});
