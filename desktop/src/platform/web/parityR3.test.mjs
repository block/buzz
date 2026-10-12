import test from "node:test";
import assert from "node:assert/strict";

test("permission prompt helpers", async () => {
  const { runningAgents, buttonsFor } = await import("./permissionPrompts.ts");
  assert.deepEqual(runningAgents([{ pubkey: "AA", name: "x", status: "running" }, { pubkey: "b", status: "stopped" }]), [{ pubkey: "aa", name: "x" }]);
  assert.deepEqual(runningAgents(undefined), []);
  const b = buttonsFor({ id: "1", title: "", kind: "", detail: "", createdAt: 0, options: [{ optionId: "r", name: "", kind: "reject_once" }, { optionId: "a", name: "Allow", kind: "allow_once" }] });
  assert.deepEqual(b.map((x) => [x.optionId, x.allow]), [["a", true], ["r", false]]);
  assert.equal(b[1].label, "reject once");
});

test("link preview handlers: session once, bearer reuse, 401 refresh, http(s) only", async () => {
  globalThis.sessionStorage = { getItem: () => null, setItem: () => {} };
  const { makeLinkPreviewHandlers, previewableHref } = await import("./linkPreview.ts");
  assert.equal(previewableHref("javascript:alert(1)"), null);
  assert.equal(previewableHref("https://e.com/x"), "https://e.com/x");
  const reqs = [];
  let first401 = true;
  const fakeFetch = async (url, init) => {
    reqs.push([String(url), init?.headers?.Authorization]);
    if (String(url).endsWith("/preview?url=" + encodeURIComponent("https://e.com/x"))) {
      if (first401 && init.headers.Authorization === "Bearer T1") {
        first401 = false;
        return new Response("", { status: 401 });
      }
      return new Response(JSON.stringify({ title: "E" }), { status: 200 });
    }
    return new Response("", { status: 204 });
  };
  let n = 0;
  const signs = [];
  const fetch2 = async (url, init) => {
    if (String(url).endsWith("/session")) return new Response(JSON.stringify({ token: `T${++n}`, exp: Math.floor(Date.now() / 1000) + 3600 }), { status: 200 });
    return fakeFetch(url, init);
  };
  const h = makeLinkPreviewHandlers("https://lp.example/", fetch2, async (m, u) => (signs.push([m, u]), "Nostr x"));
  assert.equal(await h.fetch_link_preview_metadata({ href: "ftp://x" }), null);
  const r = await h.fetch_link_preview_metadata({ href: "https://e.com/x", requestId: "r1" });
  assert.equal(r.title, "E");
  assert.equal(n, 2, "401 forced exactly one fresh session");
  assert.deepEqual(signs[0], ["POST", "https://lp.example/session"]);
  assert.equal(reqs.at(-1)[1], "Bearer T2");
  await h.fetch_link_preview_metadata({ href: "https://e.com/x" });
  assert.equal(n, 2, "session reused");
  h.cancel_link_preview_metadata({ requestId: "r1" });
  h.release_link_preview_metadata({ requestId: "r1" });
});
