import assert from "node:assert/strict";
import test from "node:test";

import { emptyRunbook } from "@/features/site-runbook/lib/serialize.ts";
import {
  acceptProcedure,
  proposeProcedure,
  setAgentBrief,
} from "@/features/site-runbook/lib/mutations.ts";

import {
  buildBrowserShare,
  browserShareToJson,
  parseBrowserShare,
  parseBrowserShareJson,
  shareRunbookToSiteRunbook,
  suggestBrowserShareFilename,
} from "./serialize.ts";
import { HULA_BROWSER_SHARE_KIND } from "./types.ts";

function sampleRunbook() {
  let runbook = emptyRunbook(100);
  runbook = setAgentBrief(runbook, "Use left nav");
  const proposed = proposeProcedure(runbook, {
    title: "Pending tip",
    steps: "Wait for review",
  });
  runbook = proposed.runbook;
  runbook = acceptProcedure(
    {
      ...runbook,
      procedures: [
        ...runbook.procedures,
        {
          id: "a1",
          title: "Search",
          steps: "1. Click search",
          status: "active",
          createdAt: 1,
          updatedAt: 2,
          acceptedAt: 3,
        },
        {
          id: "arch",
          title: "Old",
          steps: "gone",
          status: "archived",
          createdAt: 1,
          updatedAt: 2,
        },
      ],
    },
    "a1",
  );
  return runbook;
}

test("buildBrowserShare keeps active + pending, drops archived", () => {
  const share = buildBrowserShare({
    url: "https://example.com/app",
    title: "Example",
    source: "session",
    runbook: sampleRunbook(),
    exportedAt: 1234,
  });
  assert.ok(share);
  assert.equal(share.kind, HULA_BROWSER_SHARE_KIND);
  assert.equal(share.url, "https://example.com/app");
  assert.equal(share.title, "Example");
  assert.equal(share.source, "session");
  assert.equal(share.exportedAt, 1234);
  assert.equal(share.runbook.agentBrief, "Use left nav");
  const statuses = share.runbook.procedures.map((p) => p.status).sort();
  assert.deepEqual(statuses, ["active", "pending"]);
  assert.ok(!share.runbook.procedures.some((p) => p.id === "arch"));
});

test("buildBrowserShare rejects bad URL", () => {
  assert.equal(
    buildBrowserShare({
      url: "ftp://example.com",
      source: "pin",
      runbook: emptyRunbook(),
    }),
    null,
  );
});

test("parseBrowserShare roundtrip", () => {
  const share = buildBrowserShare({
    url: "https://example.com",
    title: "Ex",
    source: "pin",
    runbook: sampleRunbook(),
    exportedAt: 99,
  });
  assert.ok(share);
  const json = browserShareToJson(share);
  const parsed = parseBrowserShareJson(json);
  assert.deepEqual(parsed, share);
  assert.equal(parseBrowserShare({ kind: "other" }), null);
  assert.equal(parseBrowserShareJson("{"), null);
});

test("shareRunbookToSiteRunbook preserves pending status", () => {
  const share = buildBrowserShare({
    url: "https://example.com",
    source: "session",
    runbook: sampleRunbook(),
  });
  assert.ok(share);
  const local = shareRunbookToSiteRunbook(share.runbook, 50);
  assert.equal(local.updatedAt, 50);
  assert.ok(local.procedures.some((p) => p.status === "pending"));
  assert.ok(local.procedures.some((p) => p.status === "active"));
});

test("suggestBrowserShareFilename uses host", () => {
  const share = buildBrowserShare({
    url: "https://docs.example.com/path",
    source: "session",
    runbook: emptyRunbook(),
  });
  assert.ok(share);
  assert.equal(
    suggestBrowserShareFilename(share),
    "docs.example.com.hula-browser-share.json",
  );
});
