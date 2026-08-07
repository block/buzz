import { useQuery } from "@tanstack/react-query";

import { getLongFormNote } from "@/shared/api/social";

import type { LongFormAddress } from "./lib/nostrAddress";

export const longFormQueryKeys = {
  note: (address: LongFormAddress) =>
    ["long-form-note", address.pubkey, address.identifier] as const,
};

export function useLongFormNoteQuery(
  address: LongFormAddress,
  enabled: boolean,
) {
  return useQuery({
    queryKey: longFormQueryKeys.note(address),
    queryFn: () => getLongFormNote(address.pubkey, address.identifier),
    enabled,
    retry: false,
    staleTime: 60_000,
  });
}
