import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { mkdir, rm, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright";

const root = fileURLToPath(new URL(".", import.meta.url));
const render = process.argv.includes("--render");
const server = spawn(
  "python3",
  ["-u", "-m", "http.server", "0", "--bind", "127.0.0.1", "--directory", root],
  { stdio: ["ignore", "pipe", "pipe"] },
);
let browser;
try {
  const port = await new Promise((resolve, reject) => {
    const timer = setTimeout(
      () => reject(new Error("Preview server did not start")),
      10000,
    );
    server.on("error", reject);
    server.on("exit", (code) => reject(new Error(`Server exited: ${code}`)));
    server.stdout.on("data", (data) => {
      const match = data.toString().match(/port (\d+)/);
      if (match) {
        clearTimeout(timer);
        resolve(Number(match[1]));
      }
    });
  });
  browser = await chromium.launch({
    channel: process.env.PREVIEW_BROWSER || "chrome",
    headless: true,
  });
  const page = await browser.newPage({
    viewport: { width: 1180, height: 1000 },
  });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${port}`);
  await page.waitForFunction(() => window.ready);
  await page.getByRole("button", { name: "Pair crossing" }).click();
  await page.locator("#save-poster").click();
  await page.waitForFunction(
    () => Number(document.querySelector("#seek").value) === 5.9,
  );
  const poster = await page.locator("#poster-image").getAttribute("src");
  assert.ok(poster.startsWith("data:image/png"));
  await page.reload();
  await page.waitForFunction(() => window.ready);
  assert.match(await page.locator("#poster-status").innerText(), /5.90s/);
  await page.locator("#poster").click();
  await page.waitForFunction(
    () => Number(document.querySelector("#seek").value) === 5.9,
  );
  assert.equal(await page.locator("#poster-image").getAttribute("src"), poster);
  await page
    .getByRole("button", { name: "Wordmark only", exact: true })
    .click();
  await page.locator("#save-poster").click();
  await page.evaluate(() => {
    window.captureMode = true;
    window.renderAt(0);
  });
  const frame = (time) =>
    page.evaluate((time) => {
      window.renderAt(time);
      return document.querySelector("#hero").toDataURL("image/png");
    }, time);
  assert.equal(await frame(0), await frame(16), "Loop seam");
  assert.notEqual(await frame(1), await frame(5.9), "Flight frames differ");
  await mkdir(`${root}.output`, { recursive: true });
  await frame(1);
  await page.screenshot({ path: `${root}.output/desktop.png`, fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(
    await page.evaluate(
      () => document.documentElement.scrollWidth > innerWidth,
    ),
    false,
  );
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.reload();
  await page.waitForFunction(() => window.ready);
  assert.equal(await page.locator("#play").innerText(), "Play");
  assert.deepEqual(errors, []);
  if (render) {
    await page.evaluate(() => {
      window.captureMode = true;
    });
    const png = (data) => Buffer.from(data.split(",")[1], "base64");
    await writeFile(`${root}buzz-invite-poster.png`, png(await frame(0)));
    // Clear frames from earlier renders so ffmpeg encodes exactly 480.
    await rm(`${root}.output/frames`, { recursive: true, force: true });
    await mkdir(`${root}.output/frames`, { recursive: true });
    for (let i = 0; i < 480; i++) {
      await writeFile(
        `${root}.output/frames/${String(i).padStart(4, "0")}.png`,
        png(await frame(i / 30)),
      );
    }
    const result = spawnSync(
      "ffmpeg",
      [
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-framerate",
        "30",
        "-i",
        `${root}.output/frames/%04d.png`,
        "-c:v",
        "libx264",
        "-crf",
        "20",
        "-pix_fmt",
        "yuv420p",
        "-movflags",
        "+faststart",
        "-an",
        `${root}buzz-invite-loop.mp4`,
      ],
      { stdio: "inherit" },
    );
    assert.equal(result.status, 0, "Video encoding");
  }
  console.log(
    "PASS: poster selection/export/persistence, loop seam, mobile layout, reduced motion, JavaScript errors" +
      (render ? ", media export" : ""),
  );
} finally {
  await browser?.close();
  server.kill();
}
