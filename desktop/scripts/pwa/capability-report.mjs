// Counts every Tauri command the client invokes by capability group. Needs the dev server (reads the live registry).
// Usage: node scripts/pwa/capability-report.mjs [appUrl]
import fs from "node:fs";
import path from "node:path";
import { chromium } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";
import { CAPABILITY_GROUPS } from "../../src/platform/web/capabilities.ts";

const APP = process.argv[2] || "http://127.0.0.1:5199/";
const root = path.resolve(import.meta.dirname, "../../src");
const files = [];
(function walk(d) {
  for (const e of fs.readdirSync(d, { withFileTypes: true })) {
    const p = path.join(d, e.name);
    if (e.isDirectory()) {
      if (!/^(testing|node_modules)$/.test(e.name)) walk(p);
    } else if (
      /\.(ts|tsx)$/.test(e.name) &&
      !/\.(test|jsdom-test)\./.test(e.name)
    )
      files.push(p);
  }
})(root);
const re =
  /(?:invokeTauri|invoke|invokeWithTimeout)\s*(?:<(?:[^<>]|<(?:[^<>]|<[^<>]*>)*>)*>)?\s*\(\s*"([a-z_0-9]+)"/g;
const cmds = new Set();
for (const f of files) {
  const t = fs.readFileSync(f, "utf8");
  for (const m of t.matchAll(re)) cmds.add(m[1]);
}

const browser = await chromium.launch();
const ctx = await browser.newContext();
await installMockExtension(ctx, "11".repeat(32));
const page = await ctx.newPage();
await page.goto(APP);
await page.click('[data-testid="web-unlock-nip07"]');
await page.waitForFunction(() => window.__buzzWebHandlers, null, {
  timeout: 30000,
});
const implemented = new Set(
  await page.evaluate(() => window.__buzzWebHandlers),
);
await browser.close();

const counts = { web_implemented: 0 };
const per = {};
for (const g of CAPABILITY_GROUPS) {
  counts[g.id] = 0;
  per[g.id] = [];
}
const stray = [];
for (const c of [...cmds].sort()) {
  if (implemented.has(c)) {
    counts.web_implemented++;
    continue;
  }
  const g = CAPABILITY_GROUPS.find((x) => x.match.test(c));
  counts[g.id]++;
  per[g.id].push(c);
}
console.log(
  `total distinct commands in src: ${cmds.size}; registry entries: ${implemented.size} (of which ${counts.web_implemented} are invoked by the client)`,
);
console.log("\ngroup                 policy       count");
for (const g of CAPABILITY_GROUPS)
  console.log(`${g.id.padEnd(22)}${g.policy.padEnd(13)}${counts[g.id]}`);
console.log(`${"web_implemented".padEnd(35)}${counts.web_implemented}`);
if (process.env.LIST)
  for (const g of CAPABILITY_GROUPS)
    console.log(`\n[${g.id}]\n${per[g.id].join(" ")}`);
fs.writeFileSync(
  path.resolve(import.meta.dirname, "capability-report.json"),
  JSON.stringify({ total: cmds.size, counts, per }, null, 1),
);
