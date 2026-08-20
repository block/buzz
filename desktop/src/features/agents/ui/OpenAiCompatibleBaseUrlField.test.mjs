import assert from "node:assert/strict";
import test from "node:test";

import { openAiCompatibleBaseUrlError } from "./OpenAiCompatibleBaseUrlField.tsx";

test("OpenAI-compatible base URL accepts only safe absolute HTTP(S) URLs", () => {
  const invalidMessage =
    "Enter an HTTP or HTTPS URL without credentials, query, or fragment.";

  assert.equal(openAiCompatibleBaseUrlError("http://localhost:11434/v1"), null);
  assert.equal(
    openAiCompatibleBaseUrlError(" https://models.example/v1/ "),
    null,
  );
  assert.equal(openAiCompatibleBaseUrlError(""), "Base URL is required.");
  for (const value of [
    "ftp://models.example/v1",
    "localhost:11434/v1",
    "https://user:secret@models.example/v1",
    "https://models.example/v1?tenant=x",
    "https://models.example/v1#fragment",
  ]) {
    assert.equal(openAiCompatibleBaseUrlError(value), invalidMessage, value);
  }
});
