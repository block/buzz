import assert from "node:assert/strict";
import test from "node:test";

import { agentDefaultsSaveErrorMessage } from "./agentDefaultsSaveError.ts";

test("agent defaults save preserves a normalized Tauri error message", () => {
  const message =
    "the following keys must be set via the structured provider/model fields, not as env vars: GOOSE_PROVIDER";

  assert.equal(agentDefaultsSaveErrorMessage(new Error(message)), message);
});

test("agent defaults save preserves a legacy string rejection", () => {
  assert.equal(
    agentDefaultsSaveErrorMessage("provider is required"),
    "provider is required",
  );
});

test("agent defaults save falls back for blank or unknown failures", () => {
  assert.equal(
    agentDefaultsSaveErrorMessage(new Error("  ")),
    "Couldn't save.",
  );
  assert.equal(
    agentDefaultsSaveErrorMessage({ message: "ignored" }),
    "Couldn't save.",
  );
});
