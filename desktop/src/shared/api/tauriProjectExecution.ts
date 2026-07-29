import type { TaskOutputType } from "../../features/plans/domain/extendedContracts.ts";
import { invokeTauri } from "./tauri.ts";

type Invoke = (
  command: string,
  input: Record<string, unknown>,
) => Promise<unknown>;

export type ArtifactWriteResult = Readonly<{
  fileName: string;
  path: string;
  format: Exclude<TaskOutputType, "response">;
  storageState: "icloud" | "local_pending_icloud";
  sha256: string;
  sizeBytes: number;
}>;

export type TaskExecutionResult = Readonly<{
  summary: string;
  body: string;
  missingInputs: readonly string[];
  assumptions: readonly string[];
  provider: string | null;
  model: string | null;
  outputType: TaskOutputType;
}>;

function invalid(): never {
  throw new Error("Project Execution returned an invalid native response.");
}

function exact(value: unknown, keys: readonly string[]) {
  if (!value || typeof value !== "object" || Array.isArray(value)) invalid();
  const record = value as Record<string, unknown>;
  if (
    Object.keys(record).length !== keys.length ||
    Object.keys(record).some((key) => !keys.includes(key))
  )
    invalid();
  return record;
}

function text(value: unknown, maximum = 512 * 1024): string {
  if (
    typeof value !== "string" ||
    !value.trim() ||
    value.length > maximum ||
    value.includes("\0")
  )
    invalid();
  return value;
}

function nullableText(value: unknown): string | null {
  return value === null ? null : text(value);
}

function strings(value: unknown): readonly string[] {
  if (
    !Array.isArray(value) ||
    value.length > 128 ||
    value.some((item) => typeof item !== "string" || !item.trim())
  )
    invalid();
  return Object.freeze([...value]) as readonly string[];
}

export function parseArtifactWriteResult(value: unknown): ArtifactWriteResult {
  const object = exact(value, [
    "fileName",
    "path",
    "format",
    "storageState",
    "sha256",
    "sizeBytes",
  ]);
  const path = text(object.path, 4096);
  const sha256 = text(object.sha256, 64);
  if (
    !path.startsWith("/") ||
    !["docx", "pptx", "xlsx", "pdf"].includes(object.format as string) ||
    !["icloud", "local_pending_icloud"].includes(
      object.storageState as string,
    ) ||
    !/^[a-f0-9]{64}$/.test(sha256) ||
    !Number.isSafeInteger(object.sizeBytes) ||
    Number(object.sizeBytes) < 0 ||
    Number(object.sizeBytes) > 25 * 1024 * 1024
  )
    invalid();
  return Object.freeze({
    fileName: text(object.fileName, 512),
    path,
    format: object.format as ArtifactWriteResult["format"],
    storageState: object.storageState as ArtifactWriteResult["storageState"],
    sha256,
    sizeBytes: Number(object.sizeBytes),
  });
}

export function parseTaskExecutionResult(value: unknown): TaskExecutionResult {
  const object = exact(value, [
    "summary",
    "body",
    "missingInputs",
    "assumptions",
    "provider",
    "model",
    "outputType",
  ]);
  if (
    !["response", "docx", "pptx", "xlsx", "pdf"].includes(
      object.outputType as string,
    )
  )
    invalid();
  return Object.freeze({
    summary: text(object.summary, 16 * 1024),
    body: text(object.body),
    missingInputs: strings(object.missingInputs),
    assumptions: strings(object.assumptions),
    provider: nullableText(object.provider),
    model: nullableText(object.model),
    outputType: object.outputType as TaskOutputType,
  });
}

export async function generateTaskArtifact(
  input: Readonly<{
    projectTitle: string;
    taskTitle: string;
    format: ArtifactWriteResult["format"];
    title: string;
    body: string;
  }>,
  invoke: Invoke = invokeTauri,
) {
  return parseArtifactWriteResult(
    await invoke("generate_task_artifact", { input }),
  );
}

export async function generateHodSyncPack(
  input: Readonly<{
    projectTitle: string;
    group: string;
    body: string;
  }>,
  invoke: Invoke = invokeTauri,
) {
  return parseArtifactWriteResult(
    await invoke("generate_hod_sync_pack", { input }),
  );
}

export async function executePlanningTask(
  input: Readonly<{
    taskTitle: string;
    instructions: string;
    adviserId: string | null;
    outputType: TaskOutputType;
    dependencies: readonly Readonly<{
      title: string;
      status: string;
      summary: string | null;
    }>[];
    planningContext: unknown;
  }>,
  invoke: Invoke = invokeTauri,
) {
  return parseTaskExecutionResult(
    await invoke("execute_planning_task", { input }),
  );
}
