import { chromium } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";
const url = process.argv[2] || "http://127.0.0.1:5199/";
const relay = process.argv[3] || "ws://localhost:3100";
const sk =
  process.env.SK ||
  "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03"; // tyler (repo e2e fixture)
const SHOTS = "/Users/jm/Documents/buzz-pwa/pwa-shots";
const browser = await chromium.launch();
const ctx = await browser.newContext({
  viewport: { width: 1280, height: 800 },
});
const { pk, log } = await installMockExtension(ctx, sk);
await ctx.addInitScript(
  ({ relay, pk }) => {
    if (!localStorage.getItem("buzz-communities")) {
      localStorage.setItem(
        "buzz-communities",
        JSON.stringify([
          {
            id: "c1",
            name: "Local Dev",
            relayUrl: relay,
            pubkey: pk,
            addedAt: new Date().toISOString(),
          },
        ]),
      );
      localStorage.setItem("buzz-active-community-id", "c1");
    }
  },
  { relay, pk },
);
const page = await ctx.newPage();
const seen = new Map();
page.on("console", (m) => {
  const t = m.text();
  const mm = t.match(/^\[invoke\] (\S+)/);
  const nm = t.match(/native-only command: (\S+)/);
  if (mm) seen.set(mm[1], (seen.get(mm[1]) || 0) + 1);
  else if (nm)
    seen.set("NATIVE:" + nm[1], (seen.get("NATIVE:" + nm[1]) || 0) + 1);
  else if (m.type() === "error") console.log("ERR", t.slice(0, 250));
});
page.on("pageerror", (e) => console.log("PAGEERR", String(e).slice(0, 300)));
page.on("websocket", (ws) => {
  console.log("WS open", ws.url());
  ws.on("framesent", (f) =>
    console.log("  >>", String(f.payload).slice(0, 140)),
  );
  ws.on("framereceived", (f) =>
    console.log("  <<", String(f.payload).slice(0, 140)),
  );
});
await page.goto(url);
await page.waitForTimeout(1500);
await page.screenshot({ path: `${SHOTS}/unlock.png` });
await page.click('[data-testid="web-unlock-nip07"]');
await page.waitForTimeout(9000);
await page.screenshot({ path: `${SHOTS}/after.png` });
console.log("pk", pk, "signed kinds", log.signed.join(","));
console.log([...seen].map(([k, v]) => `${k}×${v}`).join("\n"));
await browser.close();
