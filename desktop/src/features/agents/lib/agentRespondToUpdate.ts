import type {
  RespondToMode,
  UpdateManagedAgentInput,
} from "@/shared/api/types";

type RespondToUpdate = Pick<
  UpdateManagedAgentInput,
  "respondTo" | "respondToAllowlist"
>;

/**
 * Build the authoritative inbound-author policy for an instance edit.
 *
 * The edit dialog stays mounted while managed-agent polling can replace its
 * `agent` prop. Its local form state is therefore the only reliable snapshot
 * of what the user is saving. Always send that state instead of diffing it
 * against a possibly refreshed prop; Rust validates the merged policy and the
 * retention layer suppresses unchanged relay publications.
 */
export function buildAgentRespondToUpdate(
  respondTo: RespondToMode,
  respondToAllowlist: string[],
): RespondToUpdate {
  return {
    respondTo,
    respondToAllowlist:
      respondTo === "allowlist" ? [...respondToAllowlist] : undefined,
  };
}
