import { useIdentityQuery } from "@/shared/api/hooks";
import { RemindersPanel } from "./RemindersPanel";

export function RemindersScreen() {
  const identityQuery = useIdentityQuery();

  if (!identityQuery.data?.pubkey) {
    return null;
  }

  return <RemindersPanel pubkey={identityQuery.data.pubkey} />;
}
