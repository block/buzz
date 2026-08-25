export type BusinessLeaveDecision = "leave" | "confirm";

export function getBusinessLeaveDecision(
  dirty: boolean,
): BusinessLeaveDecision {
  return dirty ? "confirm" : "leave";
}

export function resolveBusinessLeaveConfirmation(
  confirmed: boolean,
  onLeave: () => void,
): boolean {
  if (!confirmed) return false;
  onLeave();
  return true;
}
