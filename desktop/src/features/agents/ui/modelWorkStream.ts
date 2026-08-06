import { getSentMessageLink } from "./AgentSessionToolItem/messageLinks";
import type { TranscriptItem, ToolStatus } from "./agentSessionTypes";

export const MODEL_WORK_PHASES = [
  "context",
  "explore",
  "decide",
  "act",
  "deliver",
] as const;

export type ModelWorkPhase = (typeof MODEL_WORK_PHASES)[number];
export type ModelWorkStepStatus = "active" | "complete" | "failed";

export type ModelWorkStep = {
  detail: string | null;
  finding: string | null;
  id: string;
  item: TranscriptItem;
  label: string;
  phase: ModelWorkPhase;
  signalLabel: "Chose" | "Found" | null;
  status: ModelWorkStepStatus;
  trace: {
    input: string | null;
    name: string;
    output: string | null;
  };
};

export type ModelWorkPhaseState = "active" | "complete" | "idle";

export type ModelWorkStream = {
  activePhase: ModelWorkPhase | null;
  focus: string;
  phaseStates: Record<ModelWorkPhase, ModelWorkPhaseState>;
  steps: ModelWorkStep[];
  totals: {
    actions: number;
    findings: number;
    inputs: number;
  };
};

export function buildModelWorkStream(
  items: readonly TranscriptItem[],
  options: { isWorking: boolean },
): ModelWorkStream {
  const steps = items
    .filter(isWorkStreamItem)
    .map((item) => buildModelWorkStep(item, options.isWorking));
  const latestStep = steps.at(-1) ?? null;
  const activePhase = options.isWorking ? (latestStep?.phase ?? null) : null;
  const visitedPhases = new Set(steps.map((step) => step.phase));
  const phaseStates = Object.fromEntries(
    MODEL_WORK_PHASES.map((phase) => [
      phase,
      phase === activePhase
        ? "active"
        : visitedPhases.has(phase)
          ? "complete"
          : "idle",
    ]),
  ) as Record<ModelWorkPhase, ModelWorkPhaseState>;
  const latestFinding = findLast(steps, (step) => Boolean(step.finding));
  const latestDelivery = findLast(
    steps,
    (step) => step.phase === "deliver" && step.status === "complete",
  );
  const focusStep = options.isWorking
    ? latestStep
    : (latestDelivery ?? latestFinding ?? latestStep);

  return {
    activePhase,
    focus: focusStep
      ? [focusStep.label, focusStep.detail].filter(Boolean).join(" · ")
      : "Waiting for activity",
    phaseStates,
    steps,
    totals: {
      actions: steps.filter(
        (step) => step.phase === "act" || step.phase === "deliver",
      ).length,
      findings: steps.filter((step) => Boolean(step.finding)).length,
      inputs: steps.filter((step) => step.phase === "context").length,
    },
  };
}

function buildModelWorkStep(
  item: TranscriptItem,
  isWorking: boolean,
): ModelWorkStep {
  const phase = phaseForItem(item);
  const isLatestActive = isWorking && isItemActive(item);

  if (item.type === "metadata") {
    const sectionNames = item.sections
      .map((section) => section.title.trim())
      .filter(Boolean);
    return {
      detail:
        sectionNames.length > 0
          ? `${sectionNames.slice(0, 3).join(", ")}${sectionNames.length > 3 ? ` +${sectionNames.length - 3}` : ""}`
          : null,
      finding: null,
      id: item.id,
      item,
      label: "Context received",
      phase,
      signalLabel: null,
      status: "complete",
      trace: {
        input: `${item.sections.length} context ${item.sections.length === 1 ? "section" : "sections"}`,
        name: item.acpSource ?? "session/prompt:context",
        output: null,
      },
    };
  }

  if (item.type === "message") {
    return {
      detail: compactText(item.text, 120),
      finding: null,
      id: item.id,
      item,
      label: item.role === "user" ? "Request received" : "Response prepared",
      phase,
      signalLabel: null,
      status: "complete",
      trace: {
        input: compactText(item.text, 160),
        name:
          item.acpSource ??
          (item.role === "user" ? "session/prompt:user" : "assistant/message"),
        output: null,
      },
    };
  }

  if (item.type === "thought") {
    return {
      detail:
        item.title && !/^(thought|thinking|analysis)$/i.test(item.title)
          ? item.title
          : null,
      finding: null,
      id: item.id,
      item,
      label: "Analyzing available signals",
      phase,
      signalLabel: null,
      status: isLatestActive ? "active" : "complete",
      trace: {
        input: null,
        name: item.acpSource ?? "model/reasoning",
        output: "Reasoning content remains private",
      },
    };
  }

  if (item.type === "plan") {
    return {
      detail: compactText(item.text, 120),
      finding: null,
      id: item.id,
      item,
      label: item.isUpdate ? "Plan adjusted" : "Plan formed",
      phase,
      signalLabel: null,
      status: isLatestActive ? "active" : "complete",
      trace: {
        input: compactText(item.text, 160),
        name: item.acpSource ?? "model/plan",
        output: null,
      },
    };
  }

  if (item.type === "lifecycle") {
    const failed = item.renderClass === "error";
    return {
      detail: compactText(item.text, 120),
      finding: null,
      id: item.id,
      item,
      label: item.title,
      phase,
      signalLabel: null,
      status: failed ? "failed" : isLatestActive ? "active" : "complete",
      trace: {
        input: compactText(item.text, 160),
        name: item.acpSource ?? `session/${item.renderClass}`,
        output: item.outcome ?? null,
      },
    };
  }

  const isDelivery = phase === "deliver";
  const failed = item.isError || item.status === "failed";
  const status = failed
    ? "failed"
    : isToolActive(item.status)
      ? "active"
      : "complete";
  const label = isDelivery
    ? status === "active"
      ? "Delivering response"
      : failed
        ? "Delivery failed"
        : "Response delivered"
    : toolLabel(item);
  const finding =
    (phase === "explore" || phase === "decide") && status === "complete"
      ? summarizeToolResult(item.result)
      : null;

  return {
    detail: item.descriptor.object ?? item.descriptor.preview ?? null,
    finding,
    id: item.id,
    item,
    label,
    phase,
    signalLabel:
      finding && phase === "explore"
        ? "Found"
        : finding && phase === "decide"
          ? "Chose"
          : null,
    status,
    trace: {
      input: summarizeToolArguments(item.args),
      name: item.buzzToolName ?? item.toolName,
      output:
        status === "active"
          ? "Awaiting result"
          : summarizeToolResult(item.result),
    },
  };
}

function phaseForItem(item: TranscriptItem): ModelWorkPhase {
  if (item.type === "metadata") return "context";
  if (item.type === "message") {
    return item.role === "user" ? "context" : "deliver";
  }
  if (item.type === "thought" || item.type === "plan") return "decide";
  if (item.type === "lifecycle") {
    if (item.renderClass === "permission" || item.renderClass === "error") {
      return "act";
    }
    return "context";
  }

  if (
    item.descriptor.renderClass === "message" ||
    getSentMessageLink(item) !== null
  ) {
    return "deliver";
  }

  const canonicalToolName = (item.buzzToolName ?? item.toolName).toLowerCase();
  if (
    canonicalToolName === "sample_model" ||
    canonicalToolName === "reason_with_model" ||
    canonicalToolName === "query_model"
  ) {
    return "decide";
  }

  const verb = item.descriptor.action?.verb.toLowerCase() ?? "";
  if (
    item.descriptor.tone === "read" ||
    item.renderClass === "file-read" ||
    item.renderClass === "skill-read" ||
    item.renderClass === "image" ||
    /^(checked|found|listed|read|searched|viewed)$/.test(verb)
  ) {
    return "explore";
  }

  return "act";
}

function isWorkStreamItem(item: TranscriptItem) {
  if (item.renderClass === "suppressed") return false;
  if (item.type === "metadata") {
    return item.acpSource !== "raw_json_rpc";
  }
  if (item.type === "lifecycle") {
    return !/^(session ready|turn started|wire parse error)$/i.test(item.title);
  }
  return true;
}

function isItemActive(item: TranscriptItem) {
  if (item.type === "tool") return isToolActive(item.status);
  if (item.type === "lifecycle") {
    return item.renderClass === "permission" && !item.outcome;
  }
  return true;
}

function isToolActive(status: ToolStatus) {
  return status === "executing" || status === "pending";
}

function toolLabel(item: Extract<TranscriptItem, { type: "tool" }>) {
  const action = item.descriptor.action;
  if (action) {
    return [action.verb, action.object].filter(Boolean).join(" ");
  }
  return item.descriptor.label || item.title || "Tool call";
}

function summarizeToolResult(result: string): string | null {
  const trimmed = result.trim();
  if (!trimmed) return null;

  try {
    return compactText(summarizeStructuredValue(JSON.parse(trimmed)), 150);
  } catch {
    return compactText(trimmed, 150);
  }
}

function summarizeToolArguments(args: Record<string, unknown>): string | null {
  const entries = Object.entries(args);
  if (entries.length === 0) return null;

  const summary = entries
    .slice(0, 4)
    .map(([key, value]) => `${key}=${summarizeArgumentValue(value)}`)
    .join(" · ");
  return compactText(
    `${summary}${entries.length > 4 ? ` · +${entries.length - 4} more` : ""}`,
    180,
  );
}

function summarizeArgumentValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (
    typeof value === "number" ||
    typeof value === "boolean" ||
    value == null
  ) {
    return String(value);
  }
  if (Array.isArray(value)) return `[${value.length}]`;
  try {
    return JSON.stringify(value);
  } catch {
    return String(value);
  }
}

function summarizeStructuredValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (Array.isArray(value)) {
    if (value.length === 0) return "No results returned";
    const text = value
      .map((entry) => summarizeStructuredValue(entry))
      .filter(Boolean)
      .slice(0, 2)
      .join(" · ");
    return text || `${value.length} results returned`;
  }
  if (!value || typeof value !== "object") return String(value ?? "");

  const record = value as Record<string, unknown>;
  for (const key of ["messages", "items", "results", "events"]) {
    const entries = record[key];
    if (Array.isArray(entries)) {
      const noun = key === "items" || key === "results" ? "results" : key;
      return `${entries.length} ${noun} returned`;
    }
  }
  for (const key of ["summary", "text", "content", "output", "message"]) {
    if (record[key] !== undefined) {
      const summary = summarizeStructuredValue(record[key]);
      if (summary) return summary;
    }
  }

  const scalarEntries = Object.entries(record).filter(
    ([, entry]) =>
      typeof entry === "string" ||
      typeof entry === "number" ||
      typeof entry === "boolean",
  );
  return scalarEntries
    .slice(0, 3)
    .map(([key, entry]) => `${key.replaceAll("_", " ")}: ${String(entry)}`)
    .join(" · ");
}

function compactText(text: string, maxLength: number): string | null {
  const compact = text.replace(/\s+/g, " ").trim();
  if (!compact) return null;
  return compact.length > maxLength
    ? `${compact.slice(0, maxLength - 1).trimEnd()}…`
    : compact;
}

function findLast<T>(items: readonly T[], predicate: (item: T) => boolean) {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const item = items[index];
    if (item !== undefined && predicate(item)) return item;
  }
  return undefined;
}
