import assert from "node:assert/strict";
import test from "node:test";

import {
  readSettledUiRequest,
  requestSlashCommandUiAndSettle,
} from "./slashUiRequest.ts";

test("resolves when the owner settles without an error", async () => {
  const requestUi = (_name, payload) => {
    readSettledUiRequest(payload)?.settle();
    return true;
  };
  await requestSlashCommandUiAndSettle(requestUi, "ui", {}, "nobody");
});

test("rejects with the error the owner settles with", async () => {
  const requestUi = (_name, payload) => {
    readSettledUiRequest(payload)?.settle(new Error("mic denied"));
    return true;
  };
  await assert.rejects(
    requestSlashCommandUiAndSettle(requestUi, "ui", {}, "nobody"),
    /mic denied/,
  );
});

test("rejects with the unavailable message when nothing owns the UI", async () => {
  await assert.rejects(
    requestSlashCommandUiAndSettle(() => false, "ui", {}, "nobody home"),
    /nobody home/,
  );
});

test("passes the payload through and ignores foreign payloads", async () => {
  let seen = null;
  const requestUi = (_name, payload) => {
    seen = readSettledUiRequest(payload);
    seen?.settle();
    return true;
  };
  await requestSlashCommandUiAndSettle(requestUi, "ui", { query: "cats" }, "x");
  assert.equal(seen.query, "cats");
  assert.equal(readSettledUiRequest({ query: "cats" }), null);
  assert.equal(readSettledUiRequest(undefined), null);
});
