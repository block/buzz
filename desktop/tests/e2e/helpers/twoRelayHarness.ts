import { spawn, type ChildProcess } from "node:child_process";
import { createWriteStream } from "node:fs";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";

export type RelayPorts = { main: number; health: number; metrics: number };
export type RelaySpec = {
  name: string;
  ports: RelayPorts;
  databaseUrl: string;
  redisUrl: string;
};

type OwnedProcess = { name: string; child: ChildProcess; logPath: string };

export class TwoRelayHarness {
  readonly root: string;
  readonly relays: readonly [RelaySpec, RelaySpec];
  private readonly processes: OwnedProcess[] = [];

  get ownedPids(): number[] {
    return this.processes.flatMap(({ child }) =>
      child.pid ? [child.pid] : [],
    );
  }

  private constructor(root: string, relays: readonly [RelaySpec, RelaySpec]) {
    this.root = root;
    this.relays = relays;
  }

  static async create(relays: readonly [RelaySpec, RelaySpec]) {
    return new TwoRelayHarness(
      await mkdtemp(join(tmpdir(), "buzz-ae-e2e-")),
      relays,
    );
  }

  async startRelays(binary = process.env.BUZZ_E2E_RELAY_BIN) {
    if (!binary)
      throw new Error("BUZZ_E2E_RELAY_BIN is required for the live gate");
    await Promise.all(
      this.relays.map((relay) => this.startRelay(binary, relay)),
    );
  }

  async startAcp(
    name: string,
    relayWsUrl: string,
    privateKey: string,
    extraEnv: NodeJS.ProcessEnv = {},
  ) {
    const binary = process.env.BUZZ_E2E_ACP_BIN;
    if (!binary)
      throw new Error("BUZZ_E2E_ACP_BIN is required for the live gate");
    return this.spawnOwned(name, binary, [], {
      BUZZ_RELAY_URL: relayWsUrl,
      BUZZ_PRIVATE_KEY: privateKey,
      BUZZ_AUTH_TAG: "",
      BUZZ_ACP_LAZY_POOL: "true",
      BUZZ_ACP_AGENT_COMMAND: process.execPath,
      BUZZ_ACP_AGENT_ARGS: resolve("tests/e2e/fixtures/fake-acp-agent.mjs"),
      BUZZ_E2E_CLI_BIN: process.env.BUZZ_E2E_CLI_BIN,
      ...extraEnv,
    });
  }

  async logs(): Promise<string> {
    const chunks = await Promise.all(
      this.processes.map(async ({ name, logPath }) => {
        const body = await readFile(logPath, "utf8").catch(() => "<no log>");
        return `===== ${name} =====\n${body}`;
      }),
    );
    return chunks.join("\n");
  }

  private signal(child: ChildProcess, signal: NodeJS.Signals) {
    if (child.exitCode !== null || child.signalCode !== null || !child.pid)
      return;
    if (process.platform === "win32") {
      child.kill(signal);
      return;
    }
    try {
      process.kill(-child.pid, signal);
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      if (code !== "ESRCH") throw error;
    }
  }

  private async stopChild(child: ChildProcess) {
    if (child.exitCode !== null || child.signalCode !== null) return;
    const exited = new Promise<void>((resolveExit) =>
      child.once("exit", () => resolveExit()),
    );
    this.signal(child, "SIGTERM");
    if (await Promise.race([exited.then(() => true), delay(5_000, false)]))
      return;
    this.signal(child, "SIGKILL");
    if (!(await Promise.race([exited.then(() => true), delay(2_000, false)]))) {
      throw new Error(`child process ${child.pid ?? "unknown"} did not exit`);
    }
  }

  async stop() {
    await Promise.all(
      [...this.processes].reverse().map(({ child }) => this.stopChild(child)),
    );
    await rm(this.root, { recursive: true, force: true });
  }

  private async startRelay(binary: string, relay: RelaySpec) {
    const child = this.spawnOwned(relay.name, binary, [], {
      DATABASE_URL: relay.databaseUrl,
      REDIS_URL: relay.redisUrl,
      RELAY_URL: `ws://127.0.0.1:${relay.ports.main}`,
      BUZZ_BIND_ADDR: `127.0.0.1:${relay.ports.main}`,
      BUZZ_HEALTH_PORT: String(relay.ports.health),
      BUZZ_METRICS_PORT: String(relay.ports.metrics),
      BUZZ_REQUIRE_AUTH_TOKEN: "false",
      BUZZ_RECONCILE_CHANNELS: "true",
    });
    await this.waitForHealth(relay, child);
  }

  private spawnOwned(
    name: string,
    command: string,
    args: string[],
    env: NodeJS.ProcessEnv,
  ) {
    const logPath = join(this.root, `${name}.log`);
    const child = spawn(command, args, {
      cwd: resolve(".."),
      env: { ...process.env, ...env, RUST_LOG: process.env.RUST_LOG ?? "info" },
      stdio: ["ignore", "pipe", "pipe"],
      detached: process.platform !== "win32",
    });
    const log = createWriteStream(logPath, { flags: "a" });
    child.stdout?.pipe(log, { end: false });
    child.stderr?.pipe(log, { end: false });
    child.on("exit", () => log.end());
    this.processes.push({ name, child, logPath });
    return child;
  }

  private async waitForHealth(relay: RelaySpec, child: ChildProcess) {
    const deadline = Date.now() + 30_000;
    while (Date.now() < deadline) {
      if (child.exitCode !== null || child.signalCode !== null) {
        throw new Error(`${relay.name} exited before readiness`);
      }
      try {
        const response = await fetch(
          `http://127.0.0.1:${relay.ports.health}/_readiness`,
        );
        if (response.ok) return;
      } catch {}
      await delay(100);
    }
    throw new Error(`${relay.name} was not ready within 30s`);
  }
}
