import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
  applyImportRevision,
  fetchBattleRhythm,
  publishManualEvent,
  type BattleRhythmRange,
  type ImportRevisionInput,
} from "./data/battleRhythmService";
import type { BattleRhythmEvent } from "./domain/contracts";
export const battleRhythmQueryKey = (
  pubkey: string,
  range: BattleRhythmRange,
) => ["battle-rhythm", pubkey, range.start, range.end] as const;
export function useBattleRhythmQuery(
  pubkey: string | undefined,
  range: BattleRhythmRange,
) {
  return useQuery({
    enabled: Boolean(pubkey),
    queryKey: battleRhythmQueryKey(pubkey ?? "", range),
    queryFn: () => fetchBattleRhythm(pubkey ?? "", range),
    staleTime: 30_000,
  });
}
export function useBattleRhythmMutations(
  pubkey: string,
  range: BattleRhythmRange,
) {
  const client = useQueryClient();
  const invalidate = () =>
    client.invalidateQueries({ queryKey: battleRhythmQueryKey(pubkey, range) });
  return {
    manual: useMutation({
      mutationFn: (input: BattleRhythmEvent) => publishManualEvent(input),
      onSuccess: invalidate,
    }),
    importRevision: useMutation({
      mutationFn: (input: ImportRevisionInput) => applyImportRevision(input),
      onSuccess: invalidate,
    }),
  };
}
