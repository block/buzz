/**
 * Pure visibility logic for the Settings → Moderation nav entry.
 *
 * Kept free of React and IO so the gate decision is unit-testable in
 * isolation; `hooks.ts` resolves the origin + probe state that feed it.
 */

import type { AdminProbeState } from "./api";

/** Where the admin origin came from for the active identity. */
export type ModerationOriginSource = "saved" | "advertised" | "none";

/**
 * Probe outcome for the resolved origin. `"error"` distinguishes a probe that
 * threw (transport flake) from a definitive relay verdict; `null` means no
 * probe was run (no origin, or a saved origin that wins without probing).
 */
export type ModerationProbeOutcome = AdminProbeState | "error" | null;

export type ModerationNavResolution = {
  originSource: ModerationOriginSource;
  probe: ModerationProbeOutcome;
};

/**
 * Decide whether the Moderation nav entry is visible.
 *
 * - No origin (neither saved-manual nor advertised) → hidden. Ordinary members
 *   never see a dead entry.
 * - A saved manual origin always shows the entry, regardless of probe state:
 *   the Advanced affordance that edits/clears the origin lives inside the
 *   surface, so hiding it would lock a user out of fixing a bad saved URL.
 * - An advertised-only origin shows the entry when the probe plausibly
 *   authorizes (`nip98Authorized`/`disabled`) or is a transport flake
 *   (`networkOrIntercepted`/`error` — never a silent disappear on flake), and
 *   hides it on a definitive non-admin verdict (`nip98Denied`/`tokenMode`/
 *   `notAdminApi`). Fail-closed: any unrecognised outcome hides the entry.
 */
export function shouldShowModerationNav(res: ModerationNavResolution): boolean {
  if (res.originSource === "none") return false;
  if (res.originSource === "saved") return true;
  switch (res.probe) {
    case "nip98Authorized":
    case "disabled":
    case "networkOrIntercepted":
    case "error":
      return true;
    default:
      return false;
  }
}
