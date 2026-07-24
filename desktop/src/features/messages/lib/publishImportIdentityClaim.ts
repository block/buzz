import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import { KIND_IMPORT_IDENTITY_CLAIM } from "@/shared/constants/kinds";

/**
 * Sign this device identity's consent to an imported identity binding, WITHOUT
 * publishing it. The signed self-claim (kind {@link KIND_IMPORT_IDENTITY_CLAIM})
 * doubles as a proof of key possession: its valid signature is what the
 * migration service binds the attestation to at `/oidc/finalize`.
 */
async function signImportIdentityClaim(subject: string) {
  const source = subject.split(":", 1)[0] || "slack";
  return signRelayEvent({
    kind: KIND_IMPORT_IDENTITY_CLAIM,
    content: "",
    tags: [
      ["d", subject],
      ["import", source],
    ],
  });
}

/**
 * Publish the current device identity's consent to an imported identity
 * binding. The relay connection must already point at the community that owns
 * the imported history.
 */
export async function publishImportIdentityClaim(
  subject: string,
): Promise<void> {
  const event = await signImportIdentityClaim(subject);
  await relayClient.publishEvent(
    event,
    "Timed out publishing your identity claim.",
    "Failed to publish your identity claim.",
  );
}

/**
 * Complete the server side of a Slack OAuth join before connecting: sign a
 * self-claim and hand it to `/oidc/finalize` as proof of possession. The
 * service admits that exact key and publishes the owner/admin attestation.
 * The app publishes its consent claim separately after the target relay is
 * active.
 */
export async function finalizeSlackOidcAttestation(input: {
  service: string;
  code: string;
  subject: string;
}): Promise<void> {
  const event = await signImportIdentityClaim(input.subject);

  const base = input.service.replace(/\/+$/, "");
  const response = await fetch(`${base}/oidc/finalize`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ code: input.code, claim: event }),
    signal: AbortSignal.timeout(30_000),
  });
  const json = (await response.json().catch(() => ({}))) as {
    error?: unknown;
  };
  if (!response.ok) {
    const message =
      typeof json.error === "string" ? json.error : `HTTP ${response.status}`;
    throw new Error(message);
  }
}
