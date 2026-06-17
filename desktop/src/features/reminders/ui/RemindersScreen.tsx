import { useQuery } from "@tanstack/react-query";

import { getIdentity } from "@/shared/api/tauri";
import { RemindersPanel } from "./RemindersPanel";

export function RemindersScreen() {
  const identityQuery = useQuery({
    queryKey: ["identity"],
    queryFn: getIdentity,
  });

  if (!identityQuery.data?.pubkey) {
    return null;
  }

  return <RemindersPanel pubkey={identityQuery.data.pubkey} />;
}
