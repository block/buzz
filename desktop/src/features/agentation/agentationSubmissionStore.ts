import type { Annotation } from "agentation";
import type { RelayEvent } from "@/shared/api/types";

export type RetainedAgentationSubmission = {
  fingerprint: string;
  submissionId: string;
  annotations: Annotation[];
  channelId: string;
  agentPubkey: string;
  event: RelayEvent;
};

function key(scope: string) {
  return `buzz:agentation:submission:v1:${scope}`;
}

export function readRetainedAgentationSubmission(
  scope: string,
): RetainedAgentationSubmission | null {
  try {
    const raw = localStorage.getItem(key(scope));
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<RetainedAgentationSubmission>;
    return typeof parsed.fingerprint === "string" &&
      typeof parsed.submissionId === "string" &&
      Array.isArray(parsed.annotations) &&
      parsed.annotations.every(
        (annotation) =>
          typeof annotation === "object" &&
          annotation !== null &&
          typeof annotation.id === "string",
      ) &&
      typeof parsed.channelId === "string" &&
      typeof parsed.agentPubkey === "string" &&
      typeof parsed.event?.id === "string"
      ? (parsed as RetainedAgentationSubmission)
      : null;
  } catch {
    return null;
  }
}

export function retainAgentationSubmission(
  scope: string,
  submission: RetainedAgentationSubmission,
) {
  localStorage.setItem(key(scope), JSON.stringify(submission));
}

export function clearRetainedAgentationSubmission(scope: string) {
  localStorage.removeItem(key(scope));
}
