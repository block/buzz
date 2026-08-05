import type { LedgerEntry } from "@/features/agents/lib/causalLedger";
import type { CausalLedgerPersistence } from "@/features/agents/lib/liveCausalLedger";
import { invokeTauri } from "./tauri";

export function createTauriCausalLedgerPersistence(): CausalLedgerPersistence {
  return {
    async loadJournal() {
      const entries = await invokeTauri<string[]>("read_causal_ledger");
      return entries.join("\n");
    },
    async appendEntry(entry: LedgerEntry) {
      await invokeTauri("append_causal_ledger_entry", {
        entryJson: JSON.stringify(entry),
      });
    },
  };
}
