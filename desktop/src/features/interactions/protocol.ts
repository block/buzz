import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_INTERACTION_PROMPT,
  KIND_INTERACTION_STATE,
} from "@/shared/constants/kinds";

export type InteractionField = {
  id: string;
  label: string;
  type: "text" | "number" | "select" | "boolean" | "date";
  required: boolean;
  options: { id: string; label: string }[];
};
export type InteractionPrompt = {
  event: RelayEvent;
  type: string;
  options: { id: string; label: string; style: string }[];
  fields: InteractionField[];
  min: number;
  max: number;
  deadline: number;
};
export type InteractionSummary = {
  revision: number;
  status: "open" | "closed";
  close_reason: string | null;
  winner: string | null;
  tally: Record<string, number>;
  responders: {
    pubkey: string;
    event_id: string;
    created_at: number;
    choices: string[];
  }[];
};

export const eventTag = (event: RelayEvent, name: string) =>
  event.tags.find((t) => t[0] === name)?.[1];

/** Fail closed on unsupported schemas; the original message remains available. */
export function parseInteractionPrompt(
  event: RelayEvent,
  id: string,
  channel: string,
): InteractionPrompt {
  if (
    event.id !== id ||
    event.kind !== KIND_INTERACTION_PROMPT ||
    eventTag(event, "h") !== channel ||
    ![undefined, "public"].includes(eventTag(event, "visibility")) ||
    eventTag(event, "expiration") !== undefined
  )
    throw new Error("Invalid interaction prompt.");
  const type = eventTag(event, "itype") ?? "";
  if (!["buttons", "poll", "form"].includes(type))
    throw new Error("This interaction type is not supported.");
  const options = event.tags
    .filter((t) => t[0] === "opt")
    .map((t) => ({ id: t[1], label: t[2], style: t[3] ?? "default" }));
  const fields: InteractionField[] = event.tags
    .filter((t) => t[0] === "field")
    .map((t) => {
      if (!["text", "number", "select", "boolean", "date"].includes(t[3]))
        throw new Error("This field type is not supported.");
      return {
        id: t[1],
        label: t[2],
        type: t[3] as InteractionField["type"],
        required: t[4] === "required",
        options: event.tags
          .filter((o) => o[0] === "optsel" && o[1] === t[1])
          .map((o) => ({ id: o[2], label: o[3] ?? o[2] })),
      };
    });
  if (
    options.length > 12 ||
    fields.length > 12 ||
    options.some((o) => !o.id || !o.label)
  )
    throw new Error("Malformed interaction schema.");
  const min = Number(eventTag(event, "min") ?? (type === "form" ? 0 : 1));
  const max = Number(eventTag(event, "max") ?? (type === "form" ? 0 : 1));
  const deadline = Number(eventTag(event, "deadline"));
  if (
    !Number.isSafeInteger(min) ||
    !Number.isSafeInteger(max) ||
    min < 0 ||
    max < min ||
    max > options.length ||
    !Number.isSafeInteger(deadline) ||
    deadline <= 0
  )
    throw new Error("Malformed interaction limits or deadline.");
  return {
    event,
    type,
    options,
    fields,
    min,
    max,
    deadline,
  };
}

/** Only the active relay's state for this prompt may enable decision controls. */
export function parseInteractionState(
  event: RelayEvent,
  prompt: string,
  channel: string,
  relay: string,
): InteractionSummary | null {
  if (
    event.kind !== KIND_INTERACTION_STATE ||
    event.pubkey !== relay ||
    eventTag(event, "d") !== prompt ||
    eventTag(event, "h") !== channel
  )
    return null;
  try {
    const state = JSON.parse(event.content) as InteractionSummary & {
      version?: number;
    };
    if (
      state.version !== 1 ||
      !Number.isSafeInteger(state.revision) ||
      state.revision < 0 ||
      !["open", "closed"].includes(state.status) ||
      !Array.isArray(state.responders) ||
      !state.tally ||
      typeof state.tally !== "object"
    )
      return null;
    if (
      state.responders.some(
        (r) =>
          typeof r.pubkey !== "string" ||
          !Array.isArray(r.choices) ||
          !Number.isSafeInteger(r.created_at),
      )
    )
      return null;
    if (Object.values(state.tally).some((n) => !Number.isInteger(n) || n < 0))
      return null;
    return state;
  } catch {
    return null;
  }
}
