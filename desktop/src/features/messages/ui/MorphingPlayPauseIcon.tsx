import { motion, useReducedMotion } from "motion/react";

const PLAY_PATHS = [
  "M8 4.75 Q7 4.15 7 5.35 L7 18.65 Q7 19.85 8 19.25 L18.1 12.75 Q19.3 12 18.1 11.25 L8 4.75 Q8 4.75 8 4.75 Z",
  "M12 12 Q12 12 12 12 L12 12 Q12 12 12 12 L12 12 Q12 12 12 12 L12 12 Q12 12 12 12 Z",
] as const;
const PAUSE_PATHS = [
  "M7.5 5 Q6.5 5 6.5 6 L6.5 18 Q6.5 19 7.5 19 L9.5 19 Q10.5 19 10.5 18 L10.5 6 Q10.5 5 9.5 5 Z",
  "M14.5 5 Q13.5 5 13.5 6 L13.5 18 Q13.5 19 14.5 19 L16.5 19 Q17.5 19 17.5 18 L17.5 6 Q17.5 5 16.5 5 Z",
] as const;
const MORPH_TRANSITION = {
  duration: 0.16,
  ease: [0.77, 0, 0.175, 1],
} as const;

export function MorphingPlayPauseIcon({ isPlaying }: { isPlaying: boolean }) {
  const shouldReduceMotion = useReducedMotion();
  const paths = isPlaying ? PAUSE_PATHS : PLAY_PATHS;
  const transition = shouldReduceMotion ? { duration: 0 } : MORPH_TRANSITION;

  return (
    <motion.svg
      aria-hidden="true"
      className="!size-[1.4375rem]"
      data-icon-state={isPlaying ? "pause" : "play"}
      data-testid="voice-note-play-pause-icon"
      fill="currentColor"
      initial={false}
      viewBox="0 0 24 24"
    >
      {paths.map((path, index) => (
        <motion.path
          animate={{ d: path }}
          data-testid={`voice-note-play-pause-path-${index}`}
          initial={false}
          // The two paths stay mounted. In the play state the second one
          // collapses to a point, then expands into the right pause bar.
          key={index === 0 ? "primary" : "secondary"}
          stroke="currentColor"
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={0.8}
          transition={transition}
        />
      ))}
    </motion.svg>
  );
}
