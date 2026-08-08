import type { InstallRuntimeResult } from "@/shared/api/types";

/**
 * Build the user-visible error message for a failed install.
 * When the last step carries an actionable hint, it is shown first,
 * followed by the raw step failure detail.
 *
 * The step detail is truncated for display, so the message ends with a pointer
 * to the install log — which holds every attempt of every step, each record
 * bounded far above the display truncation — when one was written.
 */
export function getInstallErrorMessage(result: InstallRuntimeResult): string {
  const { steps, logPath } = result;
  const lastStep = steps[steps.length - 1];
  if (!lastStep) {
    return withLog("Install failed with no output.", logPath);
  }
  const base = `Step "${lastStep.step}" failed: ${lastStep.stderr || lastStep.stdout || "unknown error"}`;
  const detail = lastStep.hint ? `${lastStep.hint}\n\n${base}` : base;
  return withLog(detail, logPath);
}

function withLog(message: string, logPath: string | null): string {
  return logPath ? `${message}\n\nFull log: ${logPath}` : message;
}
