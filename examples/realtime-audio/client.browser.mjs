// Run with the repository's installed Playwright development dependencies.
import { chromium } from "../../desktop/node_modules/@playwright/test/index.mjs";
import assert from "node:assert/strict";
import { mkdtemp, writeFile, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { once } from "node:events";
import net from "node:net";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const directory = await mkdtemp(path.join(tmpdir(), "frankie-menu-test-"));
const requestFile = path.join(directory, "request.json");
const responseFile = path.join(directory, "response.json");
const agentFile = path.join(directory, "agent.mjs");
// A controlled ACP peer lets the real browser client exercise permission RPCs
// without model weights, inference timing, or executing actual shell commands.
await writeFile(
  agentFile,
  `#!/usr/bin/env node
import {createInterface} from 'node:readline';
import {existsSync,readFileSync,writeFileSync,unlinkSync} from 'node:fs';
const send = value => console.log(JSON.stringify({jsonrpc:'2.0',...value}));
let prompt;
const input = createInterface({input:process.stdin});
input.on('line', line => {
 const e=JSON.parse(line);
 if (!e.method) {writeFileSync(process.env.TEST_RESPONSE,JSON.stringify(e));return;}
 if (e.method==='initialize') send({id:e.id,result:{agentCapabilities:{_meta:{buzz:{realtimeAudio:1}}}}});
 else if(e.method==='session/new') send({id:e.id,result:{sessionId:'session'}});
 else if(e.method==='session/prompt') {
  prompt=e.id;
  send({method:'_buzz/unstable/realtime/update',params:{sessionId:'session',streamId:'stream',update:{type:'ready'}}});
 } else {
  send({id:e.id,result:null});
  if(e.method.endsWith('/close'))send({id:prompt,result:{stopReason:'end_turn'}});
 }
});
const timer=setInterval(()=>{
 if(!existsSync(process.env.TEST_REQUEST))return;
 const request=JSON.parse(readFileSync(process.env.TEST_REQUEST));unlinkSync(process.env.TEST_REQUEST);
 send({id:request.id,method:'session/request_permission',params:{toolCall:{title:'dev__shell',rawInput:{command:'echo example'}},options:[{kind:'allow_once',optionId:'allow'},{kind:'reject_once',optionId:'deny'}]}});
},25);
input.on('close',()=>clearInterval(timer));
`,
  { mode: 0o700 },
);
const socket = net.createServer();
socket.listen(0, "127.0.0.1");
await once(socket, "listening");
const port = socket.address().port;
await new Promise((resolve) => socket.close(resolve));
const server = spawn(
  process.execPath,
  [fileURLToPath(new URL("server.mjs", import.meta.url))],
  {
    env: {
      ...process.env,
      PORT: String(port),
      BUZZ_AGENT_BIN: agentFile,
      PATH: `${path.dirname(process.execPath)}${path.delimiter}${process.env.PATH || ""}`,
      TEST_REQUEST: requestFile,
      TEST_RESPONSE: responseFile,
    },
    stdio: ["ignore", "pipe", "pipe"],
  },
);
let output = "",
  errors = "",
  browser;
server.stdout.on("data", (value) => {
  output += value;
});
server.stderr.on("data", (value) => {
  errors += value;
});
async function until(check) {
  const deadline = Date.now() + 10000;
  while (!(await check())) {
    if (Date.now() > deadline) throw Error(`test deadline: ${errors}`);
    await delay(25);
  }
}
try {
  await until(() => output.includes("/#"));
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage({
    viewport: { width: 1280, height: 800 },
  });
  page.setDefaultTimeout(10000);
  const pageErrors = [];
  page.on("pageerror", (e) => pageErrors.push(e.message));
  await page.addInitScript(() => {
    navigator.mediaDevices.getUserMedia = async () => {
      const context = new AudioContext({ sampleRate: 24000 });
      await context.resume();
      return context.createMediaStreamDestination().stream;
    };
  });
  await page.goto(output.trim());
  const approval = page.getByLabel("Tool approval");
  assert.equal(await approval.inputValue(), "ask");
  await page.locator("#go").click();
  await page.waitForFunction(() =>
    window.realtimeEvidence?.some((e) => e.type === "ready"),
  );
  const ask = async (id) => {
    await rm(responseFile, { force: true });
    await writeFile(requestFile, JSON.stringify({ id }));
  };
  const response = async (id) => {
    let value;
    await until(async () => {
      try {
        value = JSON.parse(await readFile(responseFile));
        return value.id === id;
      } catch {
        return false;
      }
    });
    return value.result.outcome;
  };
  await ask("manual");
  await page.locator("#permission").waitFor({ state: "visible" });
  await assert.rejects(readFile(responseFile), { code: "ENOENT" });
  await page.locator("#deny").press("Enter");
  assert.deepEqual(await response("manual"), {
    outcome: "selected",
    optionId: "deny",
  });
  await approval.focus();
  await approval.press("Tab");
  assert.equal(
    await page.evaluate(() => document.activeElement.id),
    "fullscreen",
  );
  await page.keyboard.press("Shift+Tab");
  assert.equal(
    await page.evaluate(() => document.activeElement.id),
    "tool-approval",
  );
  await approval.selectOption("auto");
  assert.equal(await approval.inputValue(), "auto");
  assert.equal(await page.locator("#approval-note").isVisible(), true);
  await ask("automatic");
  assert.deepEqual(await response("automatic"), {
    outcome: "selected",
    optionId: "allow",
  });
  assert.equal(await page.locator("#permission").isVisible(), false);
  assert.equal(
    await approval.evaluate((el) => document.activeElement === el),
    true,
  );
  assert.match(
    await page.locator("#log").innerText(),
    /Automatically approved/,
  );
  await approval.selectOption("ask");
  await ask("manual-again");
  await page.locator("#permission").waitFor({ state: "visible" });
  await assert.rejects(readFile(responseFile), { code: "ENOENT" });
  await approval.selectOption("auto");
  await assert.rejects(readFile(responseFile), { code: "ENOENT" });
  await page.locator("#allow").click();
  assert.deepEqual(await response("manual-again"), {
    outcome: "selected",
    optionId: "allow",
  });
  await page.locator("#go").click();
  await page.waitForFunction(() =>
    window.realtimeEvidence.some((e) => e.type === "client_closed"),
  );
  assert.equal(await approval.inputValue(), "auto");
  for (const width of [1280, 1024, 768, 650, 390, 320]) {
    await page.setViewportSize({ width, height: 844 });
    for (const id of ["thinking", "tool-approval", "fullscreen", "go"]) {
      const box = await page.locator(`#${id}`).boundingBox();
      assert.ok(
        box &&
          box.x >= 0 &&
          box.x + box.width <= width &&
          box.y + box.height <= 844,
        `${id} clipped at ${width}`,
      );
    }
    assert.ok(
      await page.evaluate(
        () => document.documentElement.scrollWidth <= innerWidth,
      ),
      `horizontal overflow at ${width}`,
    );
  }
  await page.reload();
  assert.equal(await approval.inputValue(), "ask");
  assert.equal(await page.locator("#approval-note").isVisible(), false);
  assert.deepEqual(pageErrors, []);
  console.log(
    "Approval menu: manual, automatic, switch-back, pending decision, keyboard, reload, and six viewport checks passed.",
  );
} finally {
  await browser?.close();
  if (server.exitCode === null && server.signalCode === null) {
    server.kill("SIGTERM");
    const timeout = setTimeout(() => server.kill("SIGKILL"), 8000);
    await once(server, "exit");
    clearTimeout(timeout);
  }
  await rm(directory, { recursive: true, force: true });
}
