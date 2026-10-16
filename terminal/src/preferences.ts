import { createHash, randomUUID } from "node:crypto";
import {
  mkdirSync,
  readFileSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

/** Remembers one exact agent per community and human identity, without credentials. */
export class AgentPreferences {
  private readonly directory: string;

  constructor(directory = join(homedir(), ".buzz", "terminal", "agents")) {
    this.directory = directory;
  }

  private path(relay: string, human: string): string {
    const scope = createHash("sha256")
      .update(`${new URL(relay).href}\n${human}`)
      .digest("hex");
    return join(this.directory, scope);
  }

  /** A missing or malformed preference falls back to live discovery. */
  read(relay: string, human: string): string | undefined {
    try {
      const key = readFileSync(this.path(relay, human), "utf8").trim();
      return /^[0-9a-f]{64}$/.test(key) ? key : undefined;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
      throw error;
    }
  }

  /** Atomically replace this scope's preference; concurrent clients cannot tear it. */
  write(relay: string, human: string, agent: string): void {
    if (!/^[0-9a-f]{64}$/.test(agent))
      throw new Error("Invalid agent public key.");
    mkdirSync(this.directory, { recursive: true, mode: 0o700 });
    const destination = this.path(relay, human);
    const temporary = `${destination}.${randomUUID()}`;
    try {
      writeFileSync(temporary, `${agent}\n`, { mode: 0o600, flag: "wx" });
      renameSync(temporary, destination);
    } finally {
      rmSync(temporary, { force: true });
    }
  }
}
