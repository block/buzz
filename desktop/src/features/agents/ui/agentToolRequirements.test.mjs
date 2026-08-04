import assert from "node:assert/strict";
import test from "node:test";

import { agentToolRequirementsValid } from "./agentToolRequirements.ts";

test("accepts a complete stable tool requirement", () => {
  assert.equal(
    agentToolRequirementsValid([
      {
        id: "analytics",
        label: "Analytics reports",
        capability: "mcp.tool.run_report",
        required: true,
      },
    ]),
    true,
  );
});

test("rejects malformed capabilities and duplicate requirement identifiers", () => {
  assert.equal(
    agentToolRequirementsValid([
      {
        id: "analytics",
        label: "Analytics reports",
        capability: "run_report",
        required: true,
      },
    ]),
    false,
  );
  assert.equal(
    agentToolRequirementsValid([
      {
        id: "analytics",
        label: "Analytics reports",
        capability: "mcp.tool.run_report",
        required: true,
      },
      {
        id: "analytics",
        label: "Analytics export",
        capability: "mcp.tool.export_report",
        required: false,
      },
    ]),
    false,
  );
});

test("rejects requirement identifiers that are unsafe as binding keys", () => {
  for (const id of ["__proto__", "constructor", "prototype"]) {
    assert.equal(
      agentToolRequirementsValid([
        {
          id,
          label: "Analytics reports",
          capability: "mcp.tool.run_report",
          required: true,
        },
      ]),
      false,
    );
  }
});
