import { sha256 } from "@noble/hashes/sha2.js";
import { bytesToHex } from "@noble/hashes/utils.js";
import { z } from "zod";

import { relayClient } from "@/shared/api/relayClient";
import { signRelayEvent } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_STREAM_DECISION_RESPONSE } from "@/shared/constants/kinds";

export const DECISION_CARD_CHOICES = [
  "approve",
  "redraft",
  "escalate",
  "reject",
] as const;

export type DecisionCardChoice = (typeof DECISION_CARD_CHOICES)[number];

const decisionCardPayloadSchema = z.object({
  schema_version: z.literal(1),
  card_id: z.string().uuid(),
  title: z.string().trim().min(1).max(160),
  situation: z.string().trim().min(1).max(2_000),
  recommendation: z.string().trim().min(1).max(2_000),
  proposed_action: z.string().trim().min(1).max(2_000),
  risk: z.string().trim().min(1).max(2_000),
  record_url: z
    .string()
    .max(2_048)
    .refine((recordUrl) => {
      try {
        return ["http:", "https:"].includes(new URL(recordUrl).protocol);
      } catch {
        return false;
      }
    })
    .optional(),
  choices: z.array(z.enum(DECISION_CARD_CHOICES)).min(1).max(4),
  expires_at: z.number().int().optional(),
  shadow: z.boolean(),
});

const decisionResponsePayloadSchema = z.object({
  schema_version: z.literal(1),
  action_id: z.string().uuid(),
  card_id: z.string().uuid(),
  decision: z.enum(DECISION_CARD_CHOICES),
  payload_hash: z.string().regex(/^[0-9a-f]{64}$/i),
  note: z.string().max(2_000).optional(),
  shadow: z.boolean(),
});

export type DecisionCardPayload = z.infer<typeof decisionCardPayloadSchema>;
export type DecisionResponsePayload = z.infer<
  typeof decisionResponsePayloadSchema
>;

export type ParsedDecisionCard = {
  payload: DecisionCardPayload;
  payloadHash: string;
};

function findTag(tags: string[][], name: string): string | undefined {
  return tags.find((tag) => tag[0] === name)?.[1];
}

export function parseDecisionCard(tags: string[][]): ParsedDecisionCard | null {
  const encoded = findTag(tags, "decision_card");
  const payloadHash = findTag(tags, "payload_hash");
  if (!encoded || !payloadHash || !/^[0-9a-f]{64}$/i.test(payloadHash)) {
    return null;
  }

  try {
    const payload = decisionCardPayloadSchema.parse(JSON.parse(encoded));
    if (new Set(payload.choices).size !== payload.choices.length) return null;
    const encodedHash = bytesToHex(sha256(new TextEncoder().encode(encoded)));
    if (encodedHash !== payloadHash.toLowerCase()) return null;
    return { payload, payloadHash: encodedHash };
  } catch {
    return null;
  }
}

export function parseDecisionResponse(
  tags: string[][],
): DecisionResponsePayload | null {
  const encoded = findTag(tags, "decision_response");
  if (!encoded) return null;

  try {
    return decisionResponsePayloadSchema.parse(JSON.parse(encoded));
  } catch {
    return null;
  }
}

export function buildDecisionResponseContent(
  decision: DecisionCardChoice,
  note?: string,
): string {
  const labels: Record<DecisionCardChoice, string> = {
    approve: "✅ Approved",
    redraft: "✏️ Redraft requested",
    escalate: "↗️ Escalated",
    reject: "⛔ Rejected",
  };
  const base = `${labels[decision]} — SHADOW / NOT DELIVERED`;
  return note?.trim() ? `${base}\n\n> ${note.trim()}` : base;
}

export function buildDecisionResponseTags(input: {
  actionId: string;
  cardEventId: string;
  cardId: string;
  channelId: string;
  decision: DecisionCardChoice;
  note?: string;
  payloadHash: string;
  rootEventId?: string | null;
}): string[][] {
  const payload: DecisionResponsePayload = {
    schema_version: 1,
    action_id: input.actionId,
    card_id: input.cardId,
    decision: input.decision,
    payload_hash: input.payloadHash.toLowerCase(),
    note: input.note?.trim() || undefined,
    shadow: true,
  };
  decisionResponsePayloadSchema.parse(payload);

  const tags: string[][] = [["h", input.channelId]];
  if (input.rootEventId && input.rootEventId !== input.cardEventId) {
    tags.push(["e", input.rootEventId, "", "root"]);
  }
  tags.push(["e", input.cardEventId, "", "reply"]);
  tags.push(["decision_response", JSON.stringify(payload)]);
  tags.push(["payload_hash", payload.payload_hash]);
  tags.push(["shadow", "1"]);
  return tags;
}

export async function publishDecisionResponse(input: {
  cardEventId: string;
  cardId: string;
  channelId: string;
  decision: DecisionCardChoice;
  note?: string;
  payloadHash: string;
  rootEventId?: string | null;
}): Promise<RelayEvent> {
  const event = await signRelayEvent({
    kind: KIND_STREAM_DECISION_RESPONSE,
    content: buildDecisionResponseContent(input.decision, input.note),
    tags: buildDecisionResponseTags({
      ...input,
      actionId: crypto.randomUUID(),
    }),
  });
  return relayClient.publishEvent(
    event,
    "Timed out while recording the decision.",
    "Failed to record the decision.",
  );
}

export function selectDecisionResponse(
  events: RelayEvent[],
  cardId: string,
  payloadHash: string,
): RelayEvent | null {
  return (
    events
      .filter((event) => {
        const payload = parseDecisionResponse(event.tags);
        return (
          payload?.card_id === cardId &&
          payload.payload_hash.toLowerCase() === payloadHash.toLowerCase()
        );
      })
      .sort(
        (left, right) =>
          left.created_at - right.created_at || left.id.localeCompare(right.id),
      )[0] ?? null
  );
}
