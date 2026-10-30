// Buzz source localisation: authored UI strings only; user values stay unchanged.
import { uiT } from "@/shared/i18n/uiTranslation";
import { truncateNpub } from "@/shared/lib/pubkey";
import {
  formatDurationSecondsVerbose,
  parseDurationSeconds,
} from "./workflowDuration";
import { ACTION_LABELS } from "./workflowFormTypes";
import type { StepFormState } from "./workflowFormTypes";

const MAX_DETAIL_LENGTH = 42;

function compact(value: string): string {
  const normalized = value.trim().replaceAll(/\s+/g, " ");
  return normalized.length > MAX_DETAIL_LENGTH
    ? `${normalized.slice(0, MAX_DETAIL_LENGTH - 3)}...`
    : normalized;
}

function quoted(value: string | undefined): string | null {
  const normalized = value ? compact(value) : "";
  return normalized ? `“${normalized}”` : null;
}

// DM/approver references are pubkeys — 64-char hex in any case, or a
// canonical lowercase npub — or freeform role/template text; only a key
// identity renders as a key (an npub-shaped string that fails the shared
// helper's checksum renders the neutral Unavailable, never raw text).
const KEY_REFERENCE = /^(?:[0-9a-f]{64}|npub1[0-9a-z]+)$/i;

function destination(value: string | undefined): string | null {
  const normalized = value?.trim();
  if (!normalized) return null;
  return KEY_REFERENCE.test(normalized)
    ? truncateNpub(normalized)
    : compact(normalized);
}

function configuredStepDetail(
  step: StepFormState,
  channelLabel?: string,
): string | null {
  switch (step.action) {
    case "delay": {
      const duration = step.duration?.trim();
      if (!duration) return null;
      const seconds = parseDurationSeconds(duration);
      return compact(
        seconds === null ? duration : formatDurationSecondsVerbose(seconds),
      );
    }
    case "send_message": {
      const text = quoted(step.text);
      const channel = channelLabel ? `#${channelLabel}` : null;
      if (text && channel) return uiT("{0} in {1}", { p0: text, p1: channel });
      return text ?? channel;
    }
    case "send_dm": {
      const text = quoted(step.text);
      const recipient = destination(step.to);
      if (text && recipient)
        return uiT("{0} to {1}", { p0: text, p1: recipient });
      return text ?? recipient;
    }
    case "call_webhook": {
      const url = step.url?.trim();
      if (!url) return null;
      return `${step.method?.trim() || "POST"} ${compact(url)}`;
    }
    case "request_approval": {
      const message = quoted(step.message);
      const approver = destination(step.from);
      if (message && approver)
        return uiT("{0} from {1}", { p0: message, p1: approver });
      return message ?? approver;
    }
    case "add_reaction":
      return step.emoji?.trim() || null;
    case "set_channel_topic":
      return quoted(step.topic);
  }
}

/** Build the concise action summary rendered on a workflow step node. */
export function workflowStepDescription(
  step: StepFormState,
  options: { channelLabel?: string; includeName?: boolean } = {},
): string {
  const name = options.includeName === false ? undefined : step.name?.trim();
  const detail = configuredStepDetail(step, options.channelLabel);
  if (name && detail) return `${name} · ${detail}`;
  return name || detail || ACTION_LABELS[step.action];
}
