import { invokeTauri } from "@/shared/api/tauri";

/** Output of one `buzz folds` CLI invocation, verbatim. */
export type FoldsCliOutput = {
  stdout: string;
  stderr: string;
  exitCode: number;
};

/**
 * Run `buzz folds <args…>` (bundled CLI, desktop identity, active workspace
 * relay). Nonzero exits resolve normally with the CLI's stderr message —
 * rejection is reserved for launch failures, timeouts, and arg validation.
 */
export async function runFoldsCli(args: string[]): Promise<FoldsCliOutput> {
  return invokeTauri<FoldsCliOutput>("run_folds_cli", { args });
}

/**
 * Run a folds verb that prints JSON on success and parse it. Throws with the
 * CLI's stderr (which carries the actionable message) on nonzero exit. A
 * failed `run` may put a structured salvage report on stdout (the paid-for
 * model output that was refused or could not be published); its reason is
 * folded into the thrown message so the UI surfaces why.
 */
export async function runFoldsCliJson<T>(args: string[]): Promise<T> {
  const result = await runFoldsCli(args);
  if (result.exitCode !== 0) {
    let detail = result.stderr.trim();
    try {
      const salvage = JSON.parse(result.stdout) as { reason?: string };
      if (salvage.reason && !detail.includes(salvage.reason)) {
        detail = detail ? `${detail} (${salvage.reason})` : salvage.reason;
      }
    } catch {
      // stdout was not a salvage report; stderr already tells the story.
    }
    throw new Error(detail || `buzz folds exited with code ${result.exitCode}`);
  }
  return JSON.parse(result.stdout) as T;
}
