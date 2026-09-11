import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, writeFile, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";
import { once } from "node:events";
import net from "node:net";
import { setTimeout as delay } from "node:timers/promises";

async function freePort() {
  const server = net.createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}
async function fixture(t, body) {
  const dir = await mkdtemp(path.join(tmpdir(), "realtime-adapter-test-"));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const binary = path.join(dir, "fixture.mjs");
  await writeFile(binary, `#!${process.execPath}\n${body}`, { mode: 0o700 });
  return { dir, binary };
}
function run(t, file, args = [], env = {}) {
  const child = spawn(
    process.execPath,
    [new URL(file, import.meta.url).pathname, ...args],
    {
      env: { ...process.env, ...env },
      stdio: ["ignore", "pipe", "pipe"],
    },
  );
  let stdout = "",
    stderr = "";
  child.stdout.on("data", (bytes) => {
    stdout += bytes;
  });
  child.stderr.on("data", (bytes) => {
    stderr += bytes;
  });
  t.after(async () => {
    if (child.exitCode === null && child.signalCode === null) {
      child.kill("SIGTERM");
      const timer = setTimeout(() => child.kill("SIGKILL"), 9000);
      await once(child, "exit");
      clearTimeout(timer);
    }
  });
  return { child, output: () => stdout, errors: () => stderr };
}
async function until(check) {
  const deadline = Date.now() + 10000;
  while (!(await check())) {
    if (Date.now() > deadline) throw Error("test deadline");
    await delay(20);
  }
}

test("adapter fences origin/auth, preserves approval, constrains tools, and reaps on disconnect", {
  timeout: 20000,
}, async (t) => {
  const { dir, binary } = await fixture(
    t,
    `
import {createInterface} from 'node:readline'; import {writeFileSync} from 'node:fs';
const send = value => console.log(JSON.stringify(value));
const lines = createInterface({input:process.stdin});
lines.on('line', line => {
  const event = JSON.parse(line);
  if (event.method === 'session/new') send({jsonrpc:'2.0',id:event.id,result:event.params});
  else if (event.method === 'session/prompt') send({jsonrpc:'2.0',id:'approval',method:'session/request_permission',params:{options:[{kind:'allow_once',optionId:'allow'}]}});
  else send({jsonrpc:'2.0',id:'observed',result:event});
});
lines.on('close',()=>writeFileSync(new URL('eof',import.meta.url),'closed'));
`,
  );
  const port = await freePort();
  const app = run(t, "server.mjs", [], {
    PORT: String(port),
    BUZZ_AGENT_BIN: binary,
    BUZZ_MCP_BIN: binary,
    FRANKIE_TOOLS_PREAUTHORIZED: "1",
  });
  await until(() => app.output().includes("/#"));
  const url = new URL(app.output().trim()),
    origin = url.origin;
  const headers = {
    Authorization: `Bearer ${url.hash.slice(1)}`,
    "Content-Type": "application/json",
  };
  assert.equal((await fetch(origin)).status, 200);
  assert.equal((await fetch(`${origin}/events`)).status, 403);
  assert.equal(
    (
      await fetch(`${origin}/events`, {
        headers: { ...headers, Origin: "https://example.com" },
      })
    ).status,
    403,
  );
  assert.equal(
    (await fetch(`${origin}/events?thinking=invalid`, { headers })).status,
    400,
  );
  const abort = new AbortController();
  t.after(() => abort.abort());
  const events = await fetch(`${origin}/events?thinking=high`, {
    headers,
    signal: abort.signal,
  });
  assert.equal(events.status, 200);
  const reader = events.body.getReader(),
    decoder = new TextDecoder();
  let buffer = "";
  async function next() {
    while (!buffer.includes("\n")) {
      const { value, done } = await reader.read();
      assert.equal(done, false);
      buffer += decoder.decode(value, { stream: true });
    }
    const index = buffer.indexOf("\n"),
      line = buffer.slice(0, index);
    buffer = buffer.slice(index + 1);
    return JSON.parse(line);
  }
  const rpc = (event) =>
    fetch(`${origin}/rpc`, {
      method: "POST",
      headers,
      body: JSON.stringify(event),
    });
  assert.equal((await fetch(`${origin}/events`, { headers })).status, 409);
  assert.equal(
    (await rpc({ jsonrpc: "2.0", id: 1, method: "untrusted" })).status,
    400,
  );
  assert.equal(
    (
      await rpc({
        jsonrpc: "2.0",
        id: 2,
        method: "session/new",
        params: { cwd: "/untrusted", mcpServers: [{ command: "/untrusted" }] },
      })
    ).status,
    204,
  );
  const session = (await next()).result;
  assert.equal(session.cwd, process.cwd());
  assert.equal(session.mcpServers[0].command, binary);
  assert.equal(
    (await rpc({ jsonrpc: "2.0", id: 3, method: "session/prompt" })).status,
    204,
  );
  assert.equal((await next()).method, "session/request_permission");
  const denial = {
    jsonrpc: "2.0",
    id: "approval",
    result: { outcome: { outcome: "cancelled" } },
  };
  await rpc(denial);
  assert.deepEqual((await next()).result, denial);
  abort.abort();
  await until(async () => {
    try {
      return (await readFile(path.join(dir, "eof"), "utf8")) === "closed";
    } catch {
      return false;
    }
  });
});

test("launcher rejects invalid conditioning and occupied ports before model startup", {
  timeout: 10000,
}, async (t) => {
  const { binary, dir } = await fixture(t, "throw Error('must not start');");
  const model = path.join(dir, "model.gguf");
  await writeFile(model, "fixture");
  const common = [
    "--native",
    binary,
    "--agent",
    binary,
    "--mcp",
    binary,
    "--model",
    model,
  ];
  const invalid = run(t, "launch.mjs", [...common, "--voice-codes", model]);
  assert.equal((await once(invalid.child, "exit"))[0], 1);
  assert.match(invalid.errors(), /ICL conditioning requires/);
  const server = net.createServer();
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(() => new Promise((resolve) => server.close(resolve)));
  const occupied = run(t, "launch.mjs", [
    ...common,
    "--port",
    String(server.address().port),
  ]);
  assert.equal((await once(occupied.child, "exit"))[0], 1);
  assert.match(occupied.errors(), /EADDRINUSE/);
  assert.doesNotMatch(occupied.errors(), /must not start/);
});
