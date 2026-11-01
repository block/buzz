/**
 * Classify Share Compute backend errors so the settings card can show a
 * build-unavailable state instead of a dead-end toggle (#3841).
 */
export function isMeshFeatureDisabledError(message: string | null | undefined): boolean {
  if (!message) return false;
  const lower = message.toLowerCase();
  return (
    lower.includes("mesh-llm feature") ||
    lower.includes("not included in this build")
  );
}
