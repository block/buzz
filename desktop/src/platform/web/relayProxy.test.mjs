import assert from "node:assert/strict";
import test from "node:test";
import { relayFetchUrl } from "./relayProxy.ts";

const o = { origin: "https://hulabuzz.huladesk.com", defaultRelayHttp: "https://buzz.huladesk.com" };

test("dev: cross-origin relay goes through the vite /__relay proxy", () => {
  assert.equal(
    relayFetchUrl("https://buzz.huladesk.com", "/query", { ...o, dev: true, origin: "http://localhost:5173" }),
    `/__relay/${encodeURIComponent("https://buzz.huladesk.com")}/query`,
  );
});
test("production never emits a relative /__relay path (static host would return index.html)", () => {
  const u = relayFetchUrl("https://buzz.huladesk.com", "/query", { ...o, dev: false, prefix: "" });
  assert.equal(u, "https://buzz.huladesk.com/query");
});
test("production CDN prefix applies only to the default relay", () => {
  assert.equal(relayFetchUrl("https://buzz.huladesk.com", "/events", { ...o, dev: false, prefix: "/relay" }), "/relay/events");
  assert.equal(relayFetchUrl("https://other.example", "/events", { ...o, dev: false, prefix: "/relay" }), "https://other.example/events");
});
test("same-origin relay is fetched directly", () => {
  assert.equal(relayFetchUrl("https://hulabuzz.huladesk.com", "/query", { ...o, dev: true }), "https://hulabuzz.huladesk.com/query");
});
