import type { InstallStepResult } from "@/shared/api/types";

/**
 * Build the user-visible error message for a failed install.
 * When the last step carries an actionable hint, it is shown first,
 * followed by the raw step failure detail.
 */
export function getInstallErrorMessage(steps: InstallStepResult[]): string {
  const lastStep = steps[steps.length - 1];
  if (!lastStep) {
    return "Install failed with no output.";
  }
  const base = `Step "${lastStep.step}" failed: ${lastStep.stderr || lastStep.stdout || "unknown error"}`;
  return lastStep.hint ? `${lastStep.hint}\n\n${base}` : base;
}
