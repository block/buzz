import assert from "node:assert/strict";
import test from "node:test";

import { ollamaEndpointSecurityWarning } from "./ollamaEndpointSecurity.ts";

test("ordinary loopback Ollama HTTP endpoints do not warn", () => {
  assert.equal(ollamaEndpointSecurityWarning("http://127.0.0.1:11434"), null);
  assert.equal(ollamaEndpointSecurityWarning("http://localhost:11434"), null);
  assert.equal(ollamaEndpointSecurityWarning("http://[::1]:11434"), null);
});

test("cleartext network Ollama endpoints warn but HTTPS does not", () => {
  assert.match(
    ollamaEndpointSecurityWarning("http://192.168.1.20:11434") ?? "",
    /unencrypted HTTP/,
  );
  assert.equal(
    ollamaEndpointSecurityWarning("https://ollama.example.com"),
    null,
  );
});
