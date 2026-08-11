export type CloseToTrayBehavior =
  | "keepRunning"
  | "minimizeToTray"
  | "quitWhenClosed";

export const CLOSE_TO_TRAY_DEFAULT: CloseToTrayBehavior = "keepRunning";

export const CLOSE_TO_TRAY_OPTIONS: ReadonlyArray<{
  value: CloseToTrayBehavior;
  label: string;
  description: string;
}> = [
  {
    value: "keepRunning",
    label: "Keep running in background",
    description:
      "Closing the window hides Buzz but keeps it and its local agents running for background work. Reopen from the menu bar tray icon.",
  },
  {
    value: "minimizeToTray",
    label: "Minimize to Dock",
    description:
      "Closing the window minimizes Buzz to the Dock so it stays visible in window switchers that exclude hidden windows, while keeping agents running.",
  },
  {
    value: "quitWhenClosed",
    label: "Quit when window closes",
    description:
      "Closing the window quits Buzz entirely, like a conventional app. Local agents stop until you reopen Buzz.",
  },
];

export function isCloseToTrayBehavior(
  value: unknown,
): value is CloseToTrayBehavior {
  return (
    typeof value === "string" &&
    CLOSE_TO_TRAY_OPTIONS.some((option) => option.value === value)
  );
}

/** Lazily resolve Tauri's invoke so pure validation stays unit-testable in node. */
async function tauriInvoke(): Promise<
  (<T>(cmd: string, args?: Record<string, unknown>) => Promise<T>) | null
> {
  try {
    const { invoke, isTauri } = await import("@tauri-apps/api/core");
    return isTauri() ? invoke : null;
  } catch {
    // No Tauri bridge / dependency unavailable (plain node, web preview).
    return null;
  }
}

export async function loadCloseToTrayBehavior(): Promise<CloseToTrayBehavior> {
  const invoke = await tauriInvoke();
  if (!invoke) {
    return CLOSE_TO_TRAY_DEFAULT;
  }
  try {
    const behavior = await invoke<unknown>("get_close_to_tray_behavior");
    return isCloseToTrayBehavior(behavior) ? behavior : CLOSE_TO_TRAY_DEFAULT;
  } catch {
    return CLOSE_TO_TRAY_DEFAULT;
  }
}

export async function saveCloseToTrayBehavior(
  behavior: CloseToTrayBehavior,
): Promise<void> {
  const invoke = await tauriInvoke();
  if (!invoke) {
    return;
  }
  await invoke("set_close_to_tray_behavior", { behavior });
}
