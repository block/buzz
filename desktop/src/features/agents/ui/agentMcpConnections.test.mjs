import assert from "node:assert/strict";
import { describe, it } from "node:test";

import {
  MCP_PROFILE_ENV_KEY,
  parseAgentMcpConnections,
  validateAgentMcpConnections,
  writeAgentMcpConnections,
} from "./agentMcpConnections.ts";

const connection = {
  id: "one",
  name: "crm",
  url: "https://mcp.example.com/mcp",
  transport: "http-only",
  authType: "bearer",
  bearerToken: "account-token",
  allowedTools: "search_contacts, create_note",
};

describe("agent MCP connection profile", () => {
  it("round-trips a scoped bearer connection without embedding the token", () => {
    const written = writeAgentMcpConnections({
      baseEnvVars: { KEEP: "yes" },
      connections: [connection],
      previous: parseAgentMcpConnections({}),
    });
    assert.equal(written.KEEP, "yes");
    assert.ok(!written[MCP_PROFILE_ENV_KEY].includes("account-token"));
    assert.equal(written.BUZZ_MCP_CRM_1_AUTH_HEADER, "Bearer account-token");

    const parsed = parseAgentMcpConnections(written);
    assert.equal(parsed.error, null);
    assert.deepEqual(
      { ...parsed.connections[0], id: connection.id },
      connection,
    );
  });

  it("preserves stdio profiles the visual editor does not own", () => {
    const unmanaged = {
      name: "local",
      command: "/opt/local-mcp",
      args: [],
    };
    const env = {
      [MCP_PROFILE_ENV_KEY]: JSON.stringify([unmanaged]),
      BUZZ_MCP_LOCAL_AUTH_HEADER: "Bearer unmanaged",
    };
    const parsed = parseAgentMcpConnections(env);
    const written = writeAgentMcpConnections({
      baseEnvVars: env,
      connections: [connection],
      previous: parsed,
    });
    assert.deepEqual(JSON.parse(written[MCP_PROFILE_ENV_KEY])[0], unmanaged);
    assert.equal(written.BUZZ_MCP_LOCAL_AUTH_HEADER, "Bearer unmanaged");
  });

  it("uses distinct credential keys for names that differ only by case", () => {
    const written = writeAgentMcpConnections({
      baseEnvVars: {},
      connections: [connection, { ...connection, id: "two", name: "CRM" }],
      previous: parseAgentMcpConnections({}),
    });
    assert.equal(written.BUZZ_MCP_CRM_1_AUTH_HEADER, "Bearer account-token");
    assert.equal(written.BUZZ_MCP_CRM_2_AUTH_HEADER, "Bearer account-token");
  });

  it("rejects unsafe URLs, duplicate names, and malformed allowlists", () => {
    assert.match(
      validateAgentMcpConnections([
        { ...connection, url: "http://mcp.example" },
      ]),
      /HTTPS/,
    );
    assert.match(
      validateAgentMcpConnections([connection, { ...connection, id: "two" }]),
      /duplicated/,
    );
    assert.match(
      validateAgentMcpConnections([
        { ...connection, allowedTools: "bad__tool" },
      ]),
      /Allowed tool/,
    );
    assert.match(
      validateAgentMcpConnections([
        {
          ...connection,
          name: "a".repeat(32),
          allowedTools: "b".repeat(32),
        },
      ]),
      /too long together/,
    );
  });
});
