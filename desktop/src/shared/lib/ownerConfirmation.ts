export type WorkDriveOwnerConfirmation = {
  type: "switchboard_workdrive_profile";
  tenant_id: string;
  channel_id: string;
  owner_pubkey: string;
  capability_profile: "transcript_upload";
  upgrade_connection_id: string;
  scopes: string[];
  actions: ["workdrive.files.upload"];
};

const FENCE_OPEN = "```buzz:owner-confirmation";
const FENCE_CLOSE = "```";
const EXPECTED_SCOPES = [
  "WorkDrive.files.CREATE",
  "WorkDrive.files.READ",
  "WorkDrive.team.READ",
  "WorkDrive.teamfolders.READ",
  "WorkDrive.users.READ",
  "ZohoFiles.files.READ",
].sort();

function exactKeys(
  value: Record<string, unknown>,
  expected: string[],
): boolean {
  return (
    Object.keys(value).sort().join("\u0000") === expected.sort().join("\u0000")
  );
}

export function extractWorkDriveOwnerConfirmation(
  content: string,
): WorkDriveOwnerConfirmation | null {
  const open = content.indexOf(FENCE_OPEN);
  if (open < 0) return null;
  const start = content.indexOf("\n", open);
  const end = start < 0 ? -1 : content.indexOf(`\n${FENCE_CLOSE}`, start);
  if (start < 0 || end < 0) return null;

  try {
    const parsed: unknown = JSON.parse(content.slice(start + 1, end).trim());
    if (
      typeof parsed !== "object" ||
      parsed === null ||
      Array.isArray(parsed)
    ) {
      return null;
    }
    const value = parsed as Record<string, unknown>;
    if (
      !exactKeys(value, [
        "type",
        "tenant_id",
        "channel_id",
        "owner_pubkey",
        "capability_profile",
        "upgrade_connection_id",
        "scopes",
        "actions",
      ]) ||
      value.type !== "switchboard_workdrive_profile" ||
      value.capability_profile !== "transcript_upload" ||
      typeof value.tenant_id !== "string" ||
      !value.tenant_id ||
      typeof value.channel_id !== "string" ||
      !value.channel_id ||
      typeof value.owner_pubkey !== "string" ||
      !/^[0-9a-f]{64}$/i.test(value.owner_pubkey) ||
      typeof value.upgrade_connection_id !== "string" ||
      !value.upgrade_connection_id ||
      !Array.isArray(value.scopes) ||
      !value.scopes.every((scope) => typeof scope === "string") ||
      [...value.scopes].sort().join("\u0000") !==
        EXPECTED_SCOPES.join("\u0000") ||
      !Array.isArray(value.actions) ||
      value.actions.length !== 1 ||
      value.actions[0] !== "workdrive.files.upload"
    ) {
      return null;
    }
    return parsed as WorkDriveOwnerConfirmation;
  } catch {
    return null;
  }
}
