import assert from "node:assert/strict";
import test from "node:test";

const { fromRawGooseUpdateStatus, shouldRefreshGooseUpdateStatus } =
  await import("./tauriGooseUpdates.ts");

test("maps up-to-date Goose version fields", () => {
  assert.deepStrictEqual(
    fromRawGooseUpdateStatus({
      status: "up_to_date",
      installed_version: "1.45.0",
      latest_version: "1.45.0",
    }),
    {
      status: "up_to_date",
      installedVersion: "1.45.0",
      latestVersion: "1.45.0",
    },
  );
});

test("maps update-available Goose version fields", () => {
  assert.deepStrictEqual(
    fromRawGooseUpdateStatus({
      status: "update_available",
      installed_version: "1.44.0",
      latest_version: "1.45.0",
    }),
    {
      status: "update_available",
      installedVersion: "1.44.0",
      latestVersion: "1.45.0",
    },
  );
});

test("refreshes status only after successful Goose setup", () => {
  assert.equal(shouldRefreshGooseUpdateStatus("goose", true), true);
  assert.equal(shouldRefreshGooseUpdateStatus("goose", false), false);
  assert.equal(shouldRefreshGooseUpdateStatus("claude", true), false);
});
