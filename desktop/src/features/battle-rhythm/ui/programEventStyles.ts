import type { BattleRhythmEvent } from "../domain/contracts";
import {
  programEventTone,
  type ProgramEventTone,
} from "../domain/eventPresentation";

const toneClasses: Readonly<Record<ProgramEventTone, string>> = {
  sea: "border-blue-400/50 bg-blue-500/20 text-blue-900 dark:text-blue-100",
  port: "border-amber-400/50 bg-amber-400/20 text-amber-950 dark:text-amber-100",
  neutral: "border-primary/20 bg-primary/10 text-primary",
};

export function programEventToneClasses(tone: ProgramEventTone): string {
  return toneClasses[tone];
}

export function programEventClasses(
  event: Pick<BattleRhythmEvent, "allDay" | "location">,
): string {
  return programEventToneClasses(programEventTone(event));
}
