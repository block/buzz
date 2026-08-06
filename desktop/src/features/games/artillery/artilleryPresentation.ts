import type { ArtilleryAnimationPhase } from "@/features/games/artillery/ArtilleryScene";
import type {
  ArtilleryMatch,
  ArtillerySide,
} from "@/features/games/artillery/referee";

export const ARTILLERY_PHASE_LABELS: Record<ArtilleryAnimationPhase, string> = {
  loading: "Loading arena",
  ready: "Arena ready",
  firing: "Projectile in flight",
  impact: "Impact",
  complete: "Turn complete",
};

export function resolveArtilleryWinnerName(
  match: ArtilleryMatch,
  winner: ArtillerySide | "draw",
) {
  return winner === "draw" ? "Nobody" : match.agents[winner].name;
}
