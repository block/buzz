import assert from "node:assert/strict";
import { Readable } from "node:stream";
import { test } from "node:test";
import { JsonRpcError, NdjsonWriter, readNdjson } from "../dist/index.js";
import { silentLogger } from "./helpers.mjs";

test("NDJSON reader handles split chunks and CRLF", async () => {
  const stream = Readable.from([
    Buffer.from('{"a":'),
    Buffer.from('1}\r\n{"b":2}\n'),
  ]);
  const lines = [];
  for await (const line of readNdjson(stream, 100)) lines.push(line);
  assert.deepEqual(lines, ['{"a":1}', '{"b":2}']);
});

test("NDJSON reader stays linear across one-byte chunks and trailing frames", async () => {
  const boundaryLine = "x".repeat(8_192);
  const framed = Buffer.from(`${boundaryLine}\nshort\n\n`);
  const stream = Readable.from(
    Array.from(framed, (byte) => Buffer.from([byte])),
  );
  const lines = [];
  for await (const line of readNdjson(stream, 8_192)) lines.push(line);
  assert.deepEqual(lines, [boundaryLine, "short", ""]);
});

test("NDJSON reader enforces the exact byte boundary across tiny segments", async () => {
  const exact = Readable.from([
    ...Array.from(Buffer.from("界界"), (byte) => Buffer.from([byte])),
    Buffer.from("\n"),
  ]);
  const lines = [];
  for await (const line of readNdjson(exact, 6)) lines.push(line);
  assert.deepEqual(lines, ["界界"]);

  const oversized = Readable.from(
    Array.from(Buffer.from("1234567\n"), (byte) => Buffer.from([byte])),
  );
  await assert.rejects(
    async () => {
      for await (const _line of readNdjson(oversized, 6)) {
        // no-op
      }
    },
    (error) =>
      error instanceof JsonRpcError && /exceeds 6 bytes/.test(error.message),
  );
});

test("NDJSON reader rejects oversized unterminated input before EOF", async () => {
  const stream = Readable.from([Buffer.alloc(101, 0x61)]);
  await assert.rejects(
    async () => {
      for await (const _line of readNdjson(stream, 100)) {
        // no-op
      }
    },
    (error) =>
      error instanceof JsonRpcError && /exceeds 100 bytes/.test(error.message),
  );
});

test("NDJSON reader rejects an unterminated frame", async () => {
  await assert.rejects(async () => {
    for await (const _line of readNdjson(Readable.from(['{"a":1}']), 100)) {
      // no-op
    }
  }, /unterminated NDJSON frame/);
});

test("NDJSON writer waits for each transport drain and preserves strict order", async () => {
  const lines = [];
  const releases = [];
  const writer = new NdjsonWriter(
    (line) => {
      lines.push(JSON.parse(line));
      return new Promise((resolve) => releases.push(resolve));
    },
    silentLogger,
    { maxQueuedMessages: 4, maxQueuedBytes: 1_024 },
  );

  writer.write({ sequence: 1 });
  writer.write({ sequence: 2 });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(lines, [{ sequence: 1 }]);
  releases.shift()();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(lines, [{ sequence: 1 }, { sequence: 2 }]);
  const ending = writer.end();
  releases.shift()();
  await ending;
});

test("NDJSON writer poisons instead of growing beyond its bounded backlog", async () => {
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const failures = [];
  const writer = new NdjsonWriter(() => gate, silentLogger, {
    maxQueuedMessages: 2,
    maxQueuedBytes: 1_024,
    onFatal: (error) => failures.push(error),
  });

  writer.write({ sequence: 1 });
  writer.write({ sequence: 2 });
  writer.write({ sequence: 3 });
  assert.equal(failures.length, 1);
  assert.match(failures[0].message, /queue saturated/);
  await assert.rejects(() => writer.end(), /queue saturated/);
  release();
});
