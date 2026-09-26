import assert from "node:assert/strict";
import test from "node:test";

import { installLocalStorage } from "../../playground/lib/testStorage.mjs";

import { siteRunbooksStorageKey } from "./keys.ts";
import {
  loadSiteRunbooksBlob,
  saveSiteRunbooksBlob,
  upsertRunbookInBlob,
} from "./storage.ts";
import {
  configureSiteRunbooksScope,
  getSiteRunbook,
  resetSiteRunbooksStore,
  setSiteRunbook,
} from "./store.ts";
import { emptyRunbook } from "./serialize.ts";
import { pinRunbookRef, sidRunbookRef } from "./keys.ts";

test("persists pin and sid runbooks across configure", () => {
  installLocalStorage();
  resetSiteRunbooksStore();
  configureSiteRunbooksScope("pk", "wss://relay.example/");
  const pinRb = {
    ...emptyRunbook(1),
    agentBrief: "pin brief",
  };
  const sidRb = {
    ...emptyRunbook(2),
    agentBrief: "sid brief",
  };
  setSiteRunbook(pinRunbookRef("pin-1"), pinRb);
  setSiteRunbook(sidRunbookRef("sid-1"), sidRb);

  resetSiteRunbooksStore();
  configureSiteRunbooksScope("pk", "wss://relay.example/");
  assert.equal(getSiteRunbook(pinRunbookRef("pin-1"))?.agentBrief, "pin brief");
  assert.equal(getSiteRunbook(sidRunbookRef("sid-1"))?.agentBrief, "sid brief");
  assert.ok(
    siteRunbooksStorageKey("pk", "wss://relay.example/").includes(
      "buzz-site-runbooks.v1",
    ),
  );
});

test("saveSiteRunbooksBlob round-trip", () => {
  installLocalStorage();
  const blob = upsertRunbookInBlob(
    { version: 1, runbooks: {} },
    "sid:x",
    { agentBrief: "hi", procedures: [], updatedAt: 9 },
  );
  saveSiteRunbooksBlob("a", "wss://r/", blob);
  const loaded = loadSiteRunbooksBlob("a", "wss://r/");
  assert.equal(loaded.runbooks["sid:x"]?.agentBrief, "hi");
});
