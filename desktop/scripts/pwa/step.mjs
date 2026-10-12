// ad-hoc: sign in, fill the profile name, click through, screenshot each step
import { chromium } from "playwright";
import { installMockExtension } from "./mock-nip07.mjs";
const relay = "ws://localhost:3100",
  sk =
    process.env.SK ||
    "3dbaebadb5dfd777ff25149ee230d907a15a9e1294b40b830661e65bb42f6c03";
const SHOTS = "/Users/jm/Documents/buzz-pwa/pwa-shots";
const browser = await chromium.launch();
const ctx = await browser.newContext({
  viewport: { width: 1280, height: 800 },
});
const { pk } = await installMockExtension(ctx, sk);
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
  const nm = t.match(/native-only command: (\S+)/);
  if (nm) seen.set(nm[1], 1);
  else if (m.type() === "error" || m.type() === "warning")
    console.log(m.type().toUpperCase(), t.slice(0, 400));
});
page.on("pageerror", (e) => console.log("PAGEERR", String(e).slice(0, 300)));
page.on("response", (r) => {
  if (r.status() >= 400)
    console.log(
      "HTTP",
      r.status(),
      r.request().method(),
      r.url().slice(0, 160),
    );
});
await page.goto("http://127.0.0.1:5199/");
await page.click('[data-testid="web-unlock-nip07"]');
await page.waitForTimeout(5000);
await page.screenshot({ path: `${SHOTS}/s0.png` });
if (!(await page.$('input[placeholder="Enter your name"]'))) {
  console.log(await page.evaluate(() => document.body.innerText.slice(0, 400)));
  await browser.close();
  process.exit(0);
}
await page.fill('input[placeholder="Enter your name"]', "Tyler Web");
await page.screenshot({ path: `${SHOTS}/s1.png` });
await page.click('button:has-text("Continue")');
await page.waitForTimeout(3000);
await page.screenshot({ path: `${SHOTS}/s2.png` });
console.log(await page.evaluate(() => document.body.innerText.slice(0, 600)));
console.log("native:", [...seen.keys()].join(","));
await browser.close();
