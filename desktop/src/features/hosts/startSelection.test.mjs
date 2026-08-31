import assert from "node:assert/strict";
import test from "node:test";
import { startStatus, startUnavailable } from "./startSelection.ts";
import { validateHostReport } from "./reportValidation.ts";
import { needsReport } from "./registration.ts";
const agent = "a".repeat(64);
const report = {
  v: 3,
  name: "private name",
  os: "test",
  arch: "test",
  launcher_version: "test",
  runtimes: [
    {
      id: "goose",
      label: "Goose",
      availability: "available",
      auth_status: "logged_in",
    },
  ],
  accepts_start: true,
  provisioned: [{ agent, runtime: "goose", revision: "b".repeat(64) }],
};
test("picker separates reachability, receiver compatibility, provisioning and workload", () => {
  const row = { report };
  assert.match(startUnavailable(row, agent, undefined), /unknown/);
  assert.match(startUnavailable(row, agent, false), /offline/);
  assert.match(
    startUnavailable(
      { report: { ...report, accepts_start: false } },
      agent,
      true,
    ),
    /compatible Start receiver/,
  );
  assert.match(
    startUnavailable(row, "c".repeat(64), true),
    /Set up this same agent identity/,
  );
  assert.equal(startUnavailable(row, agent, true), undefined);
  assert.match(
    startStatus("relay_accepted"),
    /waiting for destination outcome/,
  );
  assert.match(startStatus("spawned"), /readiness not yet confirmed/);
  assert.match(startStatus("unknown"), /not launching a replacement/);
});
test("v3 allows only compatible private provisioned refs, never secrets or legacy Start", () => {
  validateHostReport(report);
  for (const bad of [
    { ...report, v: 2 },
    { ...report, private_key: "never" },
    { ...report, provisioned: [{ ...report.provisioned[0], environment: {} }] },
    { ...report, provisioned: [{ ...report.provisioned[0], revision: "bad" }] },
    { ...report, provisioned: [report.provisioned[0], report.provisioned[0]] },
    {
      ...report,
      runtimes: [{ ...report.runtimes[0], auth_status: "unknown" }],
    },
  ])
    assert.throws(() => validateHostReport(bad));
  const previous = { report, event: { tags: [["l", "profile"]] } };
  assert.equal(needsReport(previous, structuredClone(report), 100), false);
  assert.equal(
    needsReport(previous, { ...report, provisioned: [] }, 100),
    true,
  );
});
