import type { LedgerEntry } from "@/features/agents/lib/causalLedger";
import type { CausalLedgerPersistence } from "@/features/agents/lib/liveCausalLedger";
import { invokeTauri } from "./tauri";

export async function readCausalLedger(): Promise<LedgerEntry[]> {
  const entries = await invokeTauri<string[]>("read_causal_ledger");
  return entries.map((entry) => JSON.parse(entry) as LedgerEntry);
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
