import {
  CausalLedger,
  type CausalExperiment,
  type LedgerEntry,
} from "@/features/agents/lib/causalLedger";
import type { CausalLedgerPersistence } from "@/features/agents/lib/liveCausalLedger";
import { invokeTauri } from "./tauri";

export async function readCausalLedger(): Promise<LedgerEntry[]> {
  const entries = await invokeTauri<string[]>("read_causal_ledger");
  return entries.map((entry) => JSON.parse(entry) as LedgerEntry);
}

export async function appendCausalExperiment(
  experiment: CausalExperiment,
): Promise<LedgerEntry> {
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const entries = await readCausalLedger();
    const ledger = await CausalLedger.fromJournal(
      entries.map((entry) => JSON.stringify(entry)).join("\n"),
    );
    const entry = await ledger.append(experiment);
    try {
      await invokeTauri("append_causal_ledger_entry", {
        entryJson: JSON.stringify(entry),
      });
      return entry;
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      if (attempt === 0 && message.includes("append conflict")) continue;
      throw error;
    }
  }
  throw new Error("Could not append the causal experiment after retrying.");
}

export function createTauriCausalLedgerPersistence(): CausalLedgerPersistence {
  return {
    async loadJournal() {
      const entries = await readCausalLedger();
      return entries.map((entry) => JSON.stringify(entry)).join("\n");
    },
    async appendEntry(entry: LedgerEntry) {
      await invokeTauri("append_causal_ledger_entry", {
        entryJson: JSON.stringify(entry),
      });
    },
  };
}
