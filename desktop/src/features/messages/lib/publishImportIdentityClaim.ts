import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { KIND_IMPORT_IDENTITY_CLAIM } from "@/shared/constants/kinds";

/**
 * Publish the current device identity's consent to an imported identity
 * binding. The relay connection must already point at the community that owns
 * the imported history.
 */
export async function publishImportIdentityClaim(
  subject: string,
): Promise<void> {
  const source = subject.split(":", 1)[0] || "slack";
  const event = await signRelayEvent({
    kind: KIND_IMPORT_IDENTITY_CLAIM,
    content: "",
    tags: [
      ["d", subject],
      ["import", source],
    ],
  });
  await relayClient.publishEvent(
    event,
    "Timed out publishing your identity claim.",
    "Failed to publish your identity claim.",
  );
}
