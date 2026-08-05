export const CAUSAL_EXPERIMENT_SCHEMA = "causal-experiment/v1" as const;

export type ExperimentOutcome =
  | "validated"
  | "rejected"
  | "inconclusive"
  | "untested";

export type EvidenceCoverage = "observed" | "missing";

export type CausalExperiment = {
  schema: typeof CAUSAL_EXPERIMENT_SCHEMA;
  experimentId: string;
  recordedAt: string;
  task: { description: string; sourceMessageId: string | null };
  execution: { sessionId: string; turnId: string; replayOf: string | null };
  failureFingerprint: string;
  context: {
    codeVersion: string;
    policyVersion: string;
    modelVersion: string;
    toolVersion: string;
    environmentVersion: string;
  };
  hypothesis: { cause: string; evidenceIds: string[] };
  intervention: { remedyId: string; changedVariable: string };
  result: { outcome: ExperimentOutcome; evidenceIds: string[] };
  coverage: Record<string, EvidenceCoverage>;
  relations: {
    supports: string[];
    contradicts: string[];
    invalidates: string[];
  };
};

export type LedgerEntry = {
  sequence: number;
  previousHash: string;
  hash: string;
  experiment: CausalExperiment;
};

export type CausalFinding = {
  comparableExperiments: number;
  validated: number;
  rejected: number;
  inconclusive: number;
  confidence: number;
  experimentIds: string[];
};

const GENESIS_HASH = "0".repeat(64);

function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (value && typeof value === "object") {
    return `{${Object.entries(value as Record<string, unknown>)
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([key, entry]) => `${JSON.stringify(key)}:${canonical(entry)}`)
      .join(",")}}`;
  }
  return JSON.stringify(value);
}

async function sha256(value: string): Promise<string> {
  const bytes = new TextEncoder().encode(value);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
}

function sameContext(
  left: CausalExperiment["context"],
  right: CausalExperiment["context"],
): boolean {
  return canonical(left) === canonical(right);
}

function hasCompleteCoverage(experiment: CausalExperiment): boolean {
  return Object.values(experiment.coverage).every(
    (value) => value === "observed",
  );
}

function immutableClone<T>(value: T): T {
  if (Array.isArray(value)) {
    return Object.freeze(value.map(immutableClone)) as T;
  }
  if (value && typeof value === "object") {
    const clone = Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([key, entry]) => [
        key,
        immutableClone(entry),
      ]),
    );
    return Object.freeze(clone) as T;
  }
  return value;
}

export class CausalLedger {
  readonly #entries: LedgerEntry[] = [];
  readonly #ids = new Set<string>();

  get size(): number {
    return this.#entries.length;
  }

  entries(): readonly LedgerEntry[] {
    return this.#entries;
  }

  toJournal(): string {
    return this.#entries.map((entry) => canonical(entry)).join("\n");
  }

  static async fromJournal(journal: string): Promise<CausalLedger> {
    const ledger = new CausalLedger();
    const lines = journal.split("\n").filter((line) => line.trim());
    for (const line of lines) {
      const persisted = JSON.parse(line) as LedgerEntry;
      const restored = await ledger.append(persisted.experiment);
      if (
        restored.sequence !== persisted.sequence ||
        restored.previousHash !== persisted.previousHash ||
        restored.hash !== persisted.hash
      ) {
        throw new Error(
          `Causal ledger integrity failure at sequence ${persisted.sequence}`,
        );
      }
    }
    return ledger;
  }

  async append(experiment: CausalExperiment): Promise<LedgerEntry> {
    if (experiment.schema !== CAUSAL_EXPERIMENT_SCHEMA) {
      throw new Error(
        `Unsupported causal experiment schema: ${experiment.schema}`,
      );
    }
    if (this.#ids.has(experiment.experimentId)) {
      throw new Error(
        `Duplicate causal experiment: ${experiment.experimentId}`,
      );
    }
    const sequence = this.#entries.length + 1;
    const previousHash = this.#entries.at(-1)?.hash ?? GENESIS_HASH;
    const frozenExperiment = immutableClone(experiment);
    const hash = await sha256(
      canonical({ sequence, previousHash, experiment: frozenExperiment }),
    );
    const entry = Object.freeze({
      sequence,
      previousHash,
      hash,
      experiment: frozenExperiment,
    });
    this.#entries.push(entry);
    this.#ids.add(experiment.experimentId);
    return entry;
  }

  async verify(): Promise<boolean> {
    let previousHash = GENESIS_HASH;
    for (const entry of this.#entries) {
      if (entry.previousHash !== previousHash) return false;
      const expected = await sha256(
        canonical({
          sequence: entry.sequence,
          previousHash: entry.previousHash,
          experiment: entry.experiment,
        }),
      );
      if (entry.hash !== expected) return false;
      previousHash = entry.hash;
    }
    return true;
  }

  findingFor(target: CausalExperiment): CausalFinding {
    const comparable = this.#entries
      .map((entry) => entry.experiment)
      .filter(
        (experiment) =>
          experiment.failureFingerprint === target.failureFingerprint &&
          experiment.intervention.remedyId === target.intervention.remedyId &&
          sameContext(experiment.context, target.context),
      );
    const validated = comparable.filter(
      (experiment) => experiment.result.outcome === "validated",
    ).length;
    const rejected = comparable.filter(
      (experiment) => experiment.result.outcome === "rejected",
    ).length;
    const inconclusive = comparable.length - validated - rejected;
    const complete = comparable.filter(hasCompleteCoverage).length;
    const decisive = validated + rejected;
    const evidenceFactor = comparable.length ? complete / comparable.length : 0;
    const confidence = decisive ? (validated / decisive) * evidenceFactor : 0;
    return {
      comparableExperiments: comparable.length,
      validated,
      rejected,
      inconclusive,
      confidence,
      experimentIds: comparable.map((experiment) => experiment.experimentId),
    };
  }
}
