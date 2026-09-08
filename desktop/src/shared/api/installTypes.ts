/**
 * Result of an ACP runtime install, in both the shape Rust sends and the shape
 * the UI consumes.
 */

export type RawInstallStepResult = {
  step: string;
  command: string;
  success: boolean;
  stdout: string;
  stderr: string;
  exit_code: number | null;
  hint?: string;
};

export type RawInstallRuntimeResult = {
  success: boolean;
  steps: RawInstallStepResult[];
  restarted_count: number;
  failed_restart_count: number;
  /** Absent for a run with no log file — Rust serializes `None` as null. */
  log_path?: string | null;
};

export type InstallStepResult = {
  step: string;
  command: string;
  success: boolean;
  stdout: string;
  stderr: string;
  exitCode: number | null;
  hint?: string;
};

export type InstallRuntimeResult = {
  success: boolean;
  steps: InstallStepResult[];
  restartedCount: number;
  failedRestartCount: number;
  /**
   * Install log file for this run, when one was written. `steps` carries only
   * the last attempt of each step; the log holds every attempt, each record
   * bounded far above the display truncation. Null when no log could be
   * written — nothing then points at a file that does not exist.
   */
  logPath: string | null;
};

export function fromRawInstallRuntimeResult(
  raw: RawInstallRuntimeResult,
): InstallRuntimeResult {
  return {
    success: raw.success,
    steps: raw.steps.map((step) => ({
      step: step.step,
      command: step.command,
      success: step.success,
      stdout: step.stdout,
      stderr: step.stderr,
      exitCode: step.exit_code,
      hint: step.hint,
    })),
    restartedCount: raw.restarted_count,
    failedRestartCount: raw.failed_restart_count,
    logPath: raw.log_path ?? null,
  };
}
