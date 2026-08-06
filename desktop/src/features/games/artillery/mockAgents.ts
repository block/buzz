import {
  runArtilleryMatch,
  type ArtilleryAction,
  type ArtilleryAgent,
  type ArtilleryMatch,
} from "@/features/games/artillery/referee";

function action(angle: number, power: number, taunt: string): ArtilleryAction {
  return { angle, power, taunt, weapon: "pulse-shell" };
}

export const MOCK_ARTILLERY_AGENTS: {
  red: ArtilleryAgent;
  blue: ArtilleryAgent;
} = {
  red: {
    id: "mock-bumble",
    name: "Bumble",
    side: "red",
    decide: (state) => {
      const moves = [
        action(48, 72, "Opening with a calibrated arc."),
        action(42, 70, "Correcting for that crosswind."),
        action(51, 74, "This one should close it out."),
      ];
      return moves[Math.floor((state.turn - 1) / 2)] ?? moves.at(-1);
    },
  },
  blue: {
    id: "mock-fizz",
    name: "Fizz",
    side: "blue",
    decide: (state) => {
      if (state.turn === 4) {
        return { angle: 110, power: "maximum", weapon: "banana" };
      }
      const moves = [
        action(46, 73, "Returning fire—with interest."),
        action(43, 72, "Fallback accepted. Still dangerous."),
        action(50, 74, "I can still turn this around."),
      ];
      return moves[Math.floor((state.turn - 2) / 2)] ?? moves.at(-1);
    },
  },
};

export function createMockArtilleryMatch(): Promise<ArtilleryMatch> {
  return runArtilleryMatch({
    agents: MOCK_ARTILLERY_AGENTS,
    id: "bumble-vs-fizz-001",
    maxTurns: 8,
  });
}
