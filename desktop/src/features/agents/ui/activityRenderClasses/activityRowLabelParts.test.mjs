import assert from "node:assert/strict";
import test from "node:test";

import {
  splitActivityRowCountedObject,
  splitActivityRowLabel,
} from "./ActivityRow.tsx";

test("splitActivityRowLabel splits verb-led summary labels", () => {
  assert.deepEqual(splitActivityRowLabel("Ran 16 tool calls"), {
    verb: "Ran",
    object: "16 tool calls",
  });
  assert.equal(splitActivityRowLabel("Thinking"), null);
});

test("splitActivityRowCountedObject splits a leading count", () => {
  assert.deepEqual(splitActivityRowCountedObject("16 tool calls"), {
    count: 16,
    rest: " tool calls",
  });
  assert.deepEqual(splitActivityRowCountedObject("3 files"), {
    count: 3,
    rest: " files",
  });
  assert.deepEqual(splitActivityRowCountedObject("12 Buzz relay ops"), {
    count: 12,
    rest: " Buzz relay ops",
  });
});

test("splitActivityRowCountedObject leaves non-counted objects alone", () => {
  assert.equal(splitActivityRowCountedObject("npm install"), null);
  assert.equal(splitActivityRowCountedObject("2 "), null);
  assert.equal(splitActivityRowCountedObject("16"), null);
  assert.equal(splitActivityRowCountedObject("file 16"), null);
});
