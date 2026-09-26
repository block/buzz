import type { EnterpriseAuth } from "./enterpriseAuthApi";

export type EnterpriseProfileSeed = {
  username: string;
  displayName: string;
};

export function authoritativeEnterpriseProfile(
  auth: EnterpriseAuth | null | undefined,
): EnterpriseProfileSeed | null {
  const username = auth?.profileProjection?.username.trim() ?? "";
  const displayName = auth?.profileProjection?.displayName.trim() ?? "";
  if (!username || !displayName) return null;
  return { username, displayName };
}
