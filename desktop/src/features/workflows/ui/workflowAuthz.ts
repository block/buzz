import { parse as yamlParse } from "yaml";

import type { ChannelMember, ChannelRole } from "@/shared/api/types";

const WEBHOOK_AUTHOR_ROLES = new Set<ChannelRole>(["owner", "admin"]);

export function workflowUsesCallWebhook(yaml: string): boolean {
  try {
    const parsed = yamlParse(yaml);
    if (!parsed || typeof parsed !== "object") return false;

    const steps = (parsed as { steps?: unknown }).steps;
    if (!Array.isArray(steps)) return false;

    return steps.some(
      (step) =>
        step !== null &&
        typeof step === "object" &&
        (step as { action?: unknown }).action === "call_webhook",
    );
  } catch {
    // If the YAML is malformed, let the relay/schema validation report the
    // parse error on submit rather than guessing about webhook permissions.
    return false;
  }
}

export function getChannelRoleForPubkey(
  members: ChannelMember[] | undefined,
  pubkey: string | undefined,
): ChannelRole | null {
  if (!members || !pubkey) return null;

  const normalizedPubkey = pubkey.toLowerCase();
  return (
    members.find((member) => member.pubkey.toLowerCase() === normalizedPubkey)
      ?.role ?? null
  );
}

export function canAuthorWebhookWorkflow(role: ChannelRole | null): boolean {
  return role !== null && WEBHOOK_AUTHOR_ROLES.has(role);
}
