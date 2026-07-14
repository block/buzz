import { useQuery } from "@tanstack/react-query";

import { getIdentity } from "@/shared/api/tauriIdentity";

export function useIdentityQuery() {
  return useQuery({
    queryKey: ["identity"],
    queryFn: getIdentity,
    staleTime: Number.POSITIVE_INFINITY,
  });
}
