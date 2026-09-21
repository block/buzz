import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { parseOpenClawWorkspaceMcpCapability } from "./parseCapability.ts";

describe("parseOpenClawWorkspaceMcpCapability", () => {
  it("accepts valid capability JSON", () => {
    const cap = parseOpenClawWorkspaceMcpCapability({
      v: 1,
      type: "hula.capability",
      name: "openclaw.workspace_mcp",
      mcp: {
        url: "https://workspace.hulapreview.com/mcp",
        transport: "http",
        authorization: "Bearer abc.def.ghi",
        expiresAt: "2026-09-22T00:00:00Z",
      },
      hulaBuzzOnly: true,
    });
    assert.ok(cap);
    assert.equal(cap.mcp.url, "https://workspace.hulapreview.com/mcp");
    assert.equal(cap.mcp.headers, undefined);
  });

  it("preserves optional mcp.headers string map", () => {
    const cap = parseOpenClawWorkspaceMcpCapability({
      v: 1,
      type: "hula.capability",
      name: "openclaw.workspace_mcp",
      mcp: {
        url: "https://workspace.hulapreview.com/mcp",
        transport: "http",
        authorization: "Bearer abc.def.ghi",
        expiresAt: "2026-09-22T00:00:00Z",
        headers: {
          Authorization: "Bearer abc.def.ghi",
          "CF-Access-Client-Id": "cf-id",
          "CF-Access-Client-Secret": "cf-secret",
        },
      },
      hulaBuzzOnly: true,
    });
    assert.ok(cap);
    assert.equal(cap.mcp.headers?.["CF-Access-Client-Id"], "cf-id");
    assert.equal(cap.mcp.headers?.["CF-Access-Client-Secret"], "cf-secret");
    assert.equal(cap.mcp.headers?.Authorization, "Bearer abc.def.ghi");
  });

  it("ignores non-string header values", () => {
    const cap = parseOpenClawWorkspaceMcpCapability({
      type: "hula.capability",
      name: "openclaw.workspace_mcp",
      mcp: {
        url: "https://x",
        authorization: "Bearer a",
        expiresAt: "t",
        headers: { Authorization: "Bearer a", bad: 1, empty: "" },
      },
    });
    assert.ok(cap);
    assert.deepEqual(cap.mcp.headers, { Authorization: "Bearer a" });
  });

  it("rejects wrong type/name and incomplete mcp", () => {
    assert.equal(
      parseOpenClawWorkspaceMcpCapability({
        type: "other",
        name: "openclaw.workspace_mcp",
        mcp: { url: "x", authorization: "Bearer a", expiresAt: "t" },
      }),
      null,
    );
    assert.equal(
      parseOpenClawWorkspaceMcpCapability({
        type: "hula.capability",
        name: "openclaw.workspace_mcp",
        mcp: { url: "x", authorization: "Bearer a" },
      }),
      null,
    );
  });
});
