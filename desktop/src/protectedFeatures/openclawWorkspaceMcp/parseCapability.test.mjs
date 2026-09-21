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
