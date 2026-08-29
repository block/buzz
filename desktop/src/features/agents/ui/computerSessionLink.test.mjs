import assert from "node:assert/strict";
import { test } from "node:test";
import {
  buildComputerSessionLink,
  extractComputerSessionDescriptor,
  parseComputerSessionDescriptor,
} from "./computerSessionLink.ts";

const valid = {
  version: 1,
  sessionId: "mesh-session_1",
  endpoint: "https://mac.example-tailnet.ts.net:8443",
};

test("remote computer descriptor creates a credential-free exact-session link", () => {
  assert.deepEqual(parseComputerSessionDescriptor(valid), valid);
  const link = buildComputerSessionLink(valid);
  assert.equal(
    link,
    "mesh-computer://session/mesh-session_1?endpoint=https%3A%2F%2Fmac.example-tailnet.ts.net%3A8443",
  );
  assert.ok(!link.toLowerCase().includes("token"));
  assert.ok(!link.toLowerCase().includes("password"));
});

test("descriptor rejects public origins, plaintext, credentials, unknown fields, and bad ids", () => {
  for (const value of [
    { ...valid, endpoint: "https://public.example.com" },
    { ...valid, endpoint: "http://mac.example-tailnet.ts.net:8443" },
    {
      ...valid,
      endpoint: "https://user:secret@mac.example-tailnet.ts.net:8443",
    },
    { ...valid, endpoint: "https://mac.example-tailnet.ts.net:8443/path" },
    { ...valid, token: "secret" },
    { ...valid, sessionId: "../other" },
  ])
    assert.equal(parseComputerSessionDescriptor(value), null);
});

test("descriptor is read only from the terminal tool rawOutput contract", () => {
  assert.deepEqual(
    extractComputerSessionDescriptor({ rawOutput: { computerSession: valid } }),
    valid,
  );
  assert.equal(
    extractComputerSessionDescriptor({ computerSession: valid }),
    null,
  );
});
