export function shouldProvisionLocalStarterTeam(
  inviteCode: string | null | undefined,
) {
  return !inviteCode?.trim();
}
