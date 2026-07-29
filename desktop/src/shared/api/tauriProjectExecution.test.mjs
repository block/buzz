import assert from "node:assert/strict";
import test from "node:test";

globalThis.window = globalThis;
globalThis.__TAURI_INTERNALS__ = {
  invoke: async () => null,
  transformCallback: () => 1,
};

const {
  generateTaskArtifact,
  parseArtifactWriteResult,
  parseTaskExecutionResult,
} = await import("./tauriProjectExecution.ts");

test("strictly parses native artefact and execution results", () => {
  assert.equal(
    parseArtifactWriteResult({
      fileName: "brief.docx",
      path: "/tmp/brief.docx",
      format: "docx",
      storageState: "icloud",
      sha256: "a".repeat(64),
      sizeBytes: 42,
    }).format,
    "docx",
  );
  assert.equal(
    parseTaskExecutionResult({
      summary: "Ready",
      body: "Draft complete",
      missingInputs: ["Port confirmation"],
      assumptions: [],
      provider: "automatic provider route",
      model: null,
      outputType: "response",
    }).missingInputs[0],
    "Port confirmation",
  );
  assert.throws(() =>
    parseArtifactWriteResult({
      fileName: "bad.pdf",
      path: "relative.pdf",
      format: "pdf",
      storageState: "icloud",
      sha256: "a".repeat(64),
      sizeBytes: 42,
    }),
  );
});

test("calls the exact artifact command", async () => {
  const calls = [];
  const result = await generateTaskArtifact(
    {
      projectTitle: "Deployment",
      taskTitle: "Prepare brief",
      format: "pdf",
      title: "Brief",
      body: "Content",
    },
    async (command, input) => {
      calls.push({ command, input });
      return {
        fileName: "brief.pdf",
        path: "/tmp/brief.pdf",
        format: "pdf",
        storageState: "local_pending_icloud",
        sha256: "b".repeat(64),
        sizeBytes: 100,
      };
    },
  );
  assert.equal(calls[0].command, "generate_task_artifact");
  assert.equal(result.storageState, "local_pending_icloud");
});
