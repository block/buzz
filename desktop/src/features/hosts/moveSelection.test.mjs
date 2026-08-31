import assert from "node:assert/strict";
import test from "node:test";
import { moveStatus, moveUnavailable } from "./moveSelection.ts";

const agent = "a".repeat(64);
const source = {
  run: "b".repeat(32),
  location: { host: "source", label: "Desktop source" },
};
const destination = {
  host: "destination",
  report: { accepts_start: true, provisioned: [{ agent }] },
};
test("Move selects exact run and disables same, offline, unknown and unprovisioned destinations", () => {
  assert.match(
    moveUnavailable(undefined, destination, agent, true),
    /Select an active instance/,
  );
  assert.match(
    moveUnavailable({ ...source, run: "legacy" }, destination, agent, true),
    /Select an active instance/,
  );
  assert.match(
    moveUnavailable(source, { ...destination, host: "source" }, agent, true),
    /Already/,
  );
  assert.match(
    moveUnavailable(source, destination, agent, undefined),
    /unknown/,
  );
  assert.match(moveUnavailable(source, destination, agent, false), /offline/);
  assert.match(
    moveUnavailable(source, destination, "other-agent", true),
    /Set up this same agent/,
  );
  assert.equal(moveUnavailable(source, destination, agent, true), undefined);
});
test("Every supported lifecycle observation has truthful recovery copy", () => {
  assert.match(moveStatus("stopping"), /destination has not started/);
  assert.match(moveStatus("stop_unconfirmed"), /blocked; no destination Start/);
  assert.match(
    moveStatus("stopped_waiting_destination"),
    /Source confirmed stopped.*will not restart/,
  );
  assert.match(moveStatus("starting"), /waiting for its outcome/);
  assert.match(
    moveStatus("stopped_start_rejected"),
    /Source confirmed stopped; destination rejected.*new Start attempt/,
  );
  assert.match(
    moveStatus("destination_spawned"),
    /spawned.*Readiness is not yet confirmed/,
  );
  assert.match(
    moveStatus("stopped_start_unknown"),
    /Retry the saved Start, never a replacement/,
  );
});
