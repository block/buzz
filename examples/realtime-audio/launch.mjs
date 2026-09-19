// Start an isolated local demo using the existing native and Buzz binaries.
import { parseArgs } from "node:util";
import { access, mkdtemp, open, stat } from "node:fs/promises";
import { constants } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import net from "node:net";
import { setTimeout as delay } from "node:timers/promises";

const directory = path.dirname(fileURLToPath(import.meta.url));
const { values } = parseArgs({
  options: {
    help: { type: "boolean" },
    native: { type: "string" },
    model: { type: "string" },
    agent: {
      type: "string",
      default: path.resolve(directory, "../../target/release/buzz-agent"),
    },
    mcp: {
      type: "string",
      default: path.resolve(directory, "../../target/release/buzz-dev-mcp"),
    },
    cwd: { type: "string" },
    voice: { type: "string" },
    "voice-text-file": { type: "string" },
    "voice-codes": { type: "string" },
    device: { type: "string", default: "gpu" },
    context: { type: "string", default: "131072" },
    "cache-type": { type: "string", default: "q4_0" },
    threads: { type: "string", default: "4" },
    port: { type: "string", default: "18796" },
    "native-port": { type: "string", default: "18793" },
    hours: { type: "string", default: "24" },
  },
});
if (values.help) {
  console.log(`Usage: node examples/realtime-audio/launch.mjs --native PATH --model GGUF
  --device cpu|gpu                 CPU, or the GPU backend compiled into llama.cpp
  --context 131072 --cache-type q4_0|q8_0|f16 --threads 4
  --voice WAV                     Override the bundled voice with your reference
  --voice-text-file TXT --voice-codes I32   Optional matching ICL conditioning
  --cwd DIR                       Directory tools operate in (default: empty temporary directory)
  --agent PATH --mcp PATH          Override target/release Buzz binaries
  --port 18796 --native-port 18793 --hours 24
Open the printed URL in Chrome. Ctrl-C stops the demo. macOS and Linux only.`);
} else {
  await main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}

async function available(port) {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, "127.0.0.1", resolve);
  });
  await new Promise((resolve, reject) =>
    server.close((error) => (error ? reject(error) : resolve())),
  );
}

async function main() {
  if (process.platform === "win32")
    throw Error("This launcher supports macOS and Linux.");
  if (!values.native || !values.model)
    throw Error("--native and --model are required; see --help.");
  if (!["cpu", "gpu"].includes(values.device))
    throw Error("--device must be cpu or gpu");
  if (!["q4_0", "q8_0", "f16"].includes(values["cache-type"]))
    throw Error("invalid --cache-type");
  for (const [key, min, max] of [
    ["port", 1, 65535],
    ["native-port", 1, 65535],
    ["context", 4096, 262144],
    ["threads", 1, 1024],
  ]) {
    if (
      !Number.isInteger(Number(values[key])) ||
      +values[key] < min ||
      +values[key] > max
    )
      throw Error(`invalid --${key}`);
  }
  if (values.port === values["native-port"])
    throw Error("The two ports must differ.");
  if (!(+values.hours > 0 && +values.hours <= 168))
    throw Error("--hours must be greater than 0 and at most 168");
  if (
    Boolean(values["voice-text-file"]) !== Boolean(values["voice-codes"]) ||
    (values["voice-codes"] && !values.voice)
  ) {
    throw Error(
      "ICL conditioning requires --voice, --voice-text-file and --voice-codes together",
    );
  }
  for (const key of [
    "native",
    "agent",
    "mcp",
    "model",
    "voice",
    "voice-text-file",
    "voice-codes",
  ]) {
    if (!values[key]) continue;
    values[key] = path.resolve(values[key]);
    await access(
      values[key],
      ["native", "agent", "mcp"].includes(key)
        ? constants.X_OK
        : constants.R_OK,
    );
    if (!(await stat(values[key])).isFile())
      throw Error(`--${key} must name a file`);
  }
  if (values.cwd && !(await stat(values.cwd)).isDirectory())
    throw Error("--cwd must be a directory");
  await available(+values.port);
  await available(+values["native-port"]);

  const runtime = await mkdtemp(path.join(tmpdir(), "frankie-demo-"));
  const cwd = values.cwd
    ? path.resolve(values.cwd)
    : await mkdtemp(path.join(runtime, "workspace-"));
  const env = Object.fromEntries(
    Object.entries(process.env).filter(
      ([key]) => !/^(BUZZ_|OPENAI_|ANTHROPIC_|MCP_|FRANKIE_)/.test(key),
    ),
  );
  // The MCP adapter's shebang uses the same Node runtime as this launcher.
  env.PATH = `${path.dirname(process.execPath)}${path.delimiter}${env.PATH || "/usr/bin:/bin"}`;
  const credential = randomBytes(32).toString("hex");
  const children = [],
    logs = [];
  let stopping = false;
  const stop = () => {
    stopping = true;
  };
  process.on("SIGINT", stop);
  process.on("SIGTERM", stop);
  const lifetime = setTimeout(stop, +values.hours * 3600000);
  async function start(
    command,
    args,
    environment,
    logName,
    stdout = "inherit",
  ) {
    const log = await open(path.join(runtime, logName), "wx", 0o600);
    logs.push(log);
    const child = spawn(command, args, {
      cwd,
      env: environment,
      detached: true,
      stdio: ["ignore", stdout, log.fd],
    });
    children.push(child);
    child.on("error", (error) => {
      console.error(error.message);
      process.exitCode = 1;
      stop();
    });
    return child;
  }
  const ended = (child) => child.exitCode !== null || child.signalCode !== null;
  function signal(child, name) {
    if (!child.pid) return;
    try {
      process.kill(-child.pid, name);
    } catch (error) {
      if (error.code !== "ESRCH") console.error(error.message);
    }
  }
  try {
    console.log(
      `Loading Frankie. Runtime logs: ${runtime}\nTool working directory: ${cwd}`,
    );
    const args = [
      values.model,
      values["native-port"],
      "--device",
      values.device,
      "--ctx-size",
      values.context,
      "--cache-type",
      values["cache-type"],
      "--threads",
      values.threads,
      "--thinking",
      "none",
    ];
    for (const key of ["voice", "voice-text-file", "voice-codes"])
      if (values[key]) args.push(`--${key}`, values[key]);
    const native = await start(
      values.native,
      args,
      { ...env, FRANKIE_REALTIME_TOKEN: credential },
      "native.log",
      "ignore",
    );
    const deadline = Date.now() + 300000;
    while (!stopping) {
      if (ended(native))
        throw Error(`Native startup failed; inspect ${runtime}/native.log`);
      try {
        const response = await fetch(
          `http://127.0.0.1:${values["native-port"]}/health`,
          { signal: AbortSignal.timeout(1000) },
        );
        await response.body?.cancel();
        if (response.ok) break;
      } catch {
        /* Loading the GGUF precedes binding the health endpoint. */
      }
      if (Date.now() > deadline)
        throw Error(`Native readiness deadline; inspect ${runtime}/native.log`);
      await delay(200);
    }
    if (stopping) return;
    const web = await start(
      process.execPath,
      [path.join(directory, "server.mjs")],
      {
        ...env,
        PORT: values.port,
        FRANKIE_LIFETIME_HOURS: values.hours,
        BUZZ_AGENT_BIN: values.agent,
        BUZZ_MCP_BIN: path.join(directory, "mcp-shell.mjs"),
        FRANKIE_DEV_MCP_BIN: values.mcp,
        BUZZ_AGENT_PROVIDER: "openai",
        OPENAI_COMPAT_API: "realtime",
        OPENAI_COMPAT_MODEL: "frankie",
        OPENAI_COMPAT_BASE_URL: `ws://127.0.0.1:${values["native-port"]}/v1/realtime`,
        OPENAI_COMPAT_API_KEY: credential,
        BUZZ_AGENT_NO_HINTS: "1",
        BUZZ_AGENT_LLM_TIMEOUT_SECS: "240",
        BUZZ_AGENT_REALTIME_OUTPUT: "audio",
        BUZZ_AGENT_SYSTEM_PROMPT:
          "You are Frankie, a helpful voice assistant. Be concise and conversational. " +
          "For simple questions, answer directly. When asked to run a command, use the shell tool and wait for its result. " +
          "Never claim a tool ran without its result. If permission is denied, acknowledge it and do not retry that action or an equivalent command unless the user asks again. Do not read tool syntax aloud. Speak plain text without Markdown.",
      },
      "web.log",
    );
    console.log(
      "Open the URL below. Choose a thinking level, then connect. Tool calls ask for approval. Ctrl-C stops the demo.",
    );
    while (!stopping) {
      if (ended(native) || ended(web))
        throw Error(`A demo process exited; inspect ${runtime}`);
      await delay(250);
    }
  } finally {
    clearTimeout(lifetime);
    // Let the adapter close ACP stdin first so Buzz can cancel tools and reap MCP groups.
    for (const child of children.toReversed())
      if (!ended(child)) child.kill("SIGTERM");
    const deadline = Date.now() + 8000;
    while (children.some((child) => !ended(child)) && Date.now() < deadline)
      await delay(100);
    for (const child of children) if (!ended(child)) signal(child, "SIGKILL");
    for (const log of logs) await log.close();
    process.off("SIGINT", stop);
    process.off("SIGTERM", stop);
  }
}
