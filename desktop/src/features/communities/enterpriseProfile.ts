import type { BuilderlabAuth } from "./hostedCommunityApi";

export type EnterpriseProfileSeed = {
  username: string;
  displayName: string;
};

export function authoritativeEnterpriseProfile(
  auth: BuilderlabAuth | null | undefined,
): EnterpriseProfileSeed | null {
  const username = auth?.username?.trim() ?? "";
  const displayName = auth?.name?.trim() ?? "";
  if (!username || !displayName) return null;
  return { username, displayName };
}
