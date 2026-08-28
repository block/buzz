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
 * CLI's stderr (which carries the actionable message) on nonzero exit.
 */
export async function runFoldsCliJson<T>(args: string[]): Promise<T> {
  const result = await runFoldsCli(args);
  if (result.exitCode !== 0) {
    throw new Error(
      result.stderr.trim() || `buzz folds exited with code ${result.exitCode}`,
    );
  }
  return JSON.parse(result.stdout) as T;
}
