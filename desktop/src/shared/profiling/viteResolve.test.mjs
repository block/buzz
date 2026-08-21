/**
 * Real Vite resolve-graph test for the temporary renderer profiling branch
 * (never merges). The unit suite for `resolveTauriCore` proves the decision in
 * isolation, but the coverage contract depends on how Vite actually resolves
 * imports through the shipped config — the exact gap a bare-specifier alias
 * hid. This drives Vite's own plugin container against `vite.config.ts` so the
 * three call shapes are proven end-to-end, not modeled.
 */

import assert from "node:assert/strict";
import path from "node:path";
import { after, before, describe, it } from "node:test";
import { fileURLToPath } from "node:url";

const desktopDir = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "../../..",
);
const abs = (p) => path.resolve(desktopDir, p);
const PROXY = abs("src/shared/profiling/tauriCoreProxy.ts");
const REAL_CORE = abs("node_modules/@tauri-apps/api/core.js");

describe("Vite resolves every Tauri core import through the proxy", () => {
  let server;
  const resolve = (source, importer) =>
    server.pluginContainer
      .resolveId(source, importer)
      .then((r) => r?.id ?? null);

  before(async () => {
    const { createServer } = await import("vite");
    server = await createServer({
      configFile: abs("vite.config.ts"),
      server: { middlewareMode: true },
      logLevel: "silent",
    });
  });

  after(async () => {
    await server?.close();
  });

  it("routes a raw @tauri-apps/api/core import to the proxy", async () => {
    assert.equal(
      await resolve("@tauri-apps/api/core", abs("src/App.tsx")),
      PROXY,
    );
  });

  it("routes a built-in submodule's relative ./core.js to the proxy", async () => {
    // event.js/app.js/window.js import core relatively — the bypass being fixed.
    assert.equal(
      await resolve("./core.js", abs("node_modules/@tauri-apps/api/event.js")),
      PROXY,
    );
  });

  it("routes an external plugin's core import to the proxy", async () => {
    assert.equal(
      await resolve(
        "@tauri-apps/api/core",
        abs("node_modules/@tauri-apps/plugin-opener/dist-js/index.js"),
      ),
      PROXY,
    );
  });

  it("resolves the proxy's escape import to the real core (no loop)", async () => {
    assert.equal(await resolve("@tauri-core-impl", PROXY), REAL_CORE);
  });
});
