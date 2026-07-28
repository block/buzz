import assert from "node:assert/strict";
import test from "node:test";

const calls = [];
let response = null;
globalThis.window = globalThis;
globalThis.__TAURI_INTERNALS__ = {
  invoke: async (command, args) => {
    calls.push({ command, args });
    return response;
  },
  transformCallback: () => 1,
};

const { interpretBattleRhythmDocument, pickBattleRhythmDocument } =
  await import("./tauriBattleRhythm.ts");

test("picker returns a strict extracted planning document", async () => {
  calls.length = 0;
  response = {
    filename: "Shortcast.xlsx",
    extension: "xlsx",
    sha256: "a".repeat(64),
    sizeBytes: 2048,
    blocks: [
      {
        kind: "spreadsheet_cell",
        location: "Shortcast!B2",
        sheet: "Shortcast",
        coordinate: "B2",
        value: "Navigation brief",
      },
    ],
    pages: [],
    sheets: [{ name: "Shortcast", maximumRow: 2, maximumColumn: 2 }],
    truncated: false,
  };

  const result = await pickBattleRhythmDocument();

  assert.deepEqual(calls, [
    { command: "pick_battle_rhythm_document", args: {} },
  ]);
  assert.equal(result?.filename, "Shortcast.xlsx");
  assert.equal(result?.blocks[0].location, "Shortcast!B2");
});

test("picker preserves cancellation and rejects unknown native fields", async () => {
  response = null;
  assert.equal(await pickBattleRhythmDocument(), null);

  response = {
    filename: "bad.docx",
    extension: "docx",
    sha256: "b".repeat(64),
    sizeBytes: 1,
    blocks: [],
    pages: [],
    sheets: [],
    truncated: false,
    unexpected: true,
  };
  await assert.rejects(
    pickBattleRhythmDocument(),
    /invalid extracted document/,
  );
});

test("structured interpretation uses the provider-neutral native command", async () => {
  calls.length = 0;
  const document = {
    filename: "Shortcast.docx",
    extension: "docx",
    sha256: "c".repeat(64),
    sizeBytes: 20,
    blocks: [],
    pages: [],
    sheets: [],
    truncated: false,
  };
  response = { schemaVersion: 1 };

  const result = await interpretBattleRhythmDocument(document, "shortcast", {
    start: "2026-07-01T00:00:00+10:00",
    end: "2026-08-01T00:00:00+10:00",
  });

  assert.deepEqual(result, { schemaVersion: 1 });
  assert.deepEqual(calls, [
    {
      command: "interpret_battle_rhythm_document",
      args: {
        request: {
          document,
          sourceType: "shortcast",
          proposedCoverage: {
            start: "2026-07-01T00:00:00+10:00",
            end: "2026-08-01T00:00:00+10:00",
          },
        },
      },
    },
  ]);
});
