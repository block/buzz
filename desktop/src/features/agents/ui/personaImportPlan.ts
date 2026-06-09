import type { ParsedPersonaPreview } from "@/shared/api/tauriPersonas";
import type { AgentPersona } from "@/shared/api/types";

type LineChangeCounts = {
  addedLines: number;
  removedLines: number;
};

export type PersonaImportFieldChange = {
  field: string;
  label: string;
  existingValue: string;
  importedValue: string;
  addedLines: number;
  removedLines: number;
};

export type PersonaImportPlan = {
  fields: PersonaImportFieldChange[];
};

type BuildPersonaImportPlanInput = {
  persona: AgentPersona;
  preview: ParsedPersonaPreview;
};

function normalizeOptionalText(value: string | null | undefined): string {
  return (value ?? "").trim();
}

function normalizePromptLines(prompt: string): string[] {
  const normalized = prompt.replace(/\r\n/g, "\n");
  const lines = normalized.split("\n").map((line) => line.trimEnd());
  while (lines.length > 0 && lines[lines.length - 1] === "") {
    lines.pop();
  }
  return lines;
}

function countLineChanges(
  previousLines: string[],
  nextLines: string[],
): LineChangeCounts {
  const previousLength = previousLines.length;
  const nextLength = nextLines.length;

  if (previousLength === 0) {
    return { addedLines: nextLength, removedLines: 0 };
  }
  if (nextLength === 0) {
    return { addedLines: 0, removedLines: previousLength };
  }

  const lcs = Array.from({ length: previousLength + 1 }, () =>
    Array<number>(nextLength + 1).fill(0),
  );

  for (let i = previousLength - 1; i >= 0; i -= 1) {
    for (let j = nextLength - 1; j >= 0; j -= 1) {
      if (previousLines[i] === nextLines[j]) {
        lcs[i][j] = lcs[i + 1][j + 1] + 1;
      } else {
        lcs[i][j] = Math.max(lcs[i + 1][j], lcs[i][j + 1]);
      }
    }
  }

  let i = 0;
  let j = 0;
  let addedLines = 0;
  let removedLines = 0;

  while (i < previousLength && j < nextLength) {
    if (previousLines[i] === nextLines[j]) {
      i += 1;
      j += 1;
      continue;
    }
    if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      removedLines += 1;
      i += 1;
    } else {
      addedLines += 1;
      j += 1;
    }
  }

  removedLines += previousLength - i;
  addedLines += nextLength - j;

  return { addedLines, removedLines };
}

function singleLineChanges(
  existing: string,
  imported: string,
): LineChangeCounts {
  return countLineChanges(
    normalizePromptLines(existing),
    normalizePromptLines(imported),
  );
}

function namePoolToString(pool: string[] | undefined): string {
  return (pool ?? []).join(", ");
}

export function buildPersonaImportPlan({
  persona,
  preview,
}: BuildPersonaImportPlanInput): PersonaImportPlan {
  const fields: PersonaImportFieldChange[] = [];

  const existingDisplayName = normalizeOptionalText(persona.displayName);
  const importedDisplayName = normalizeOptionalText(preview.displayName);
  if (existingDisplayName !== importedDisplayName) {
    fields.push({
      field: "displayName",
      label: "Display name",
      existingValue: existingDisplayName,
      importedValue: importedDisplayName,
      ...singleLineChanges(existingDisplayName, importedDisplayName),
    });
  }

  const existingAvatar = normalizeOptionalText(persona.avatarUrl);
  const importedAvatar = normalizeOptionalText(preview.avatarDataUrl);
  if (existingAvatar !== importedAvatar) {
    fields.push({
      field: "avatarUrl",
      label: "Avatar",
      existingValue: existingAvatar,
      importedValue: importedAvatar,
      ...singleLineChanges(existingAvatar, importedAvatar),
    });
  }

  const existingPrompt = normalizeOptionalText(persona.systemPrompt);
  const importedPrompt = normalizeOptionalText(preview.systemPrompt);
  if (existingPrompt !== importedPrompt) {
    fields.push({
      field: "systemPrompt",
      label: "System prompt",
      existingValue: existingPrompt,
      importedValue: importedPrompt,
      ...countLineChanges(
        normalizePromptLines(existingPrompt),
        normalizePromptLines(importedPrompt),
      ),
    });
  }

  const existingRuntime = normalizeOptionalText(persona.runtime);
  const importedRuntime = normalizeOptionalText(preview.runtime);
  if (existingRuntime !== importedRuntime) {
    fields.push({
      field: "runtime",
      label: "Preferred runtime",
      existingValue: existingRuntime,
      importedValue: importedRuntime,
      ...singleLineChanges(existingRuntime, importedRuntime),
    });
  }

  const existingModel = normalizeOptionalText(persona.model);
  const importedModel = normalizeOptionalText(preview.model);
  if (existingModel !== importedModel) {
    fields.push({
      field: "model",
      label: "Preferred model",
      existingValue: existingModel,
      importedValue: importedModel,
      ...singleLineChanges(existingModel, importedModel),
    });
  }

  const existingNamePool = namePoolToString(persona.namePool);
  const importedNamePool = namePoolToString(preview.namePool);
  if (existingNamePool !== importedNamePool) {
    fields.push({
      field: "namePool",
      label: "Instance name pool",
      existingValue: existingNamePool,
      importedValue: importedNamePool,
      ...singleLineChanges(existingNamePool, importedNamePool),
    });
  }

  return { fields };
}

export function hasAnyPersonaImportChanges(
  plan: PersonaImportPlan | null,
): boolean {
  return (plan?.fields.length ?? 0) > 0;
}
