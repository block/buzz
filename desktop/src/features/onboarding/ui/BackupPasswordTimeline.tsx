import { FileKey2, LockKeyhole, LockOpen } from "lucide-react";
import { motion, useReducedMotion } from "motion/react";

import { cn } from "@/shared/lib/cn";

const BACKUP_KEY_DOTS = [
  "key-dot-1",
  "key-dot-2",
  "key-dot-3",
  "key-dot-4",
  "key-dot-5",
  "key-dot-6",
  "key-dot-7",
  "key-dot-8",
  "key-dot-9",
] as const;

const TIMELINE_CONNECTOR_DOTS = [
  "connector-dot-1",
  "connector-dot-2",
  "connector-dot-3",
  "connector-dot-4",
] as const;

const TIMELINE_DOT_INITIAL = { opacity: 0.35, scale: 0.85 };
const TIMELINE_DOT_PULSE = {
  opacity: [0.35, 1, 0.35],
  scale: [0.85, 1.25, 0.85],
};
const TIMELINE_DOT_TRANSITION = {
  duration: 0.7,
  ease: "easeInOut" as const,
  repeat: Number.POSITIVE_INFINITY,
  repeatDelay: 1.2,
};
const TIMELINE_TOP_DOT_TRANSITIONS = TIMELINE_CONNECTOR_DOTS.map(
  (_, index) => ({
    ...TIMELINE_DOT_TRANSITION,
    delay: index * 0.16,
  }),
);
const TIMELINE_BOTTOM_DOT_TRANSITIONS = TIMELINE_CONNECTOR_DOTS.map(
  (_, index) => ({
    ...TIMELINE_DOT_TRANSITION,
    delay: (index + TIMELINE_CONNECTOR_DOTS.length) * 0.16 + 0.24,
  }),
);

export function BackupFileUnlockPreview() {
  const reduceMotion = useReducedMotion() ?? false;

  return (
    <div
      aria-hidden
      className="mx-auto flex w-[min(28rem,calc(100vw-5rem))] flex-col items-center text-foreground/75"
      data-testid="backup-file-unlock-preview"
    >
      <TimelineDots
        reduceMotion={reduceMotion}
        transitions={TIMELINE_TOP_DOT_TRANSITIONS}
      />
      <div
        className="flex items-center justify-center gap-2 py-1"
        data-testid="backup-file-key-dots"
      >
        {BACKUP_KEY_DOTS.map((dot) => (
          <span
            className="block size-2 rounded-full bg-foreground/85"
            key={`preview-${dot}`}
          />
        ))}
      </div>
      <TimelineDots
        reduceMotion={reduceMotion}
        transitions={TIMELINE_BOTTOM_DOT_TRANSITIONS}
      />
      <LockOpen
        className="size-9"
        data-testid="backup-file-unlock-preview-icon"
      />
    </div>
  );
}

function TimelineDots({
  reduceMotion,
  transitions,
}: {
  reduceMotion: boolean;
  transitions: ReadonlyArray<
    typeof TIMELINE_DOT_TRANSITION & { delay: number }
  >;
}) {
  return (
    <div className="my-3 flex h-12 flex-col items-center justify-between">
      {TIMELINE_CONNECTOR_DOTS.map((dot, index) => (
        <motion.span
          animate={reduceMotion ? undefined : TIMELINE_DOT_PULSE}
          className="block size-1.5 rounded-full bg-foreground/65"
          initial={reduceMotion ? false : TIMELINE_DOT_INITIAL}
          key={dot}
          transition={reduceMotion ? undefined : transitions[index]}
        />
      ))}
    </div>
  );
}

/**
 * Decorative timeline shared by backup creation and encrypted-backup restore.
 * Backup creation reads key → password → lock; restore reads encrypted file →
 * password → unlocked account. The password field is layered over the center.
 */
export function BackupPasswordTimeline({
  className,
  mode = "backup",
}: {
  className?: string;
  mode?: "backup" | "restore";
}) {
  const reduceMotion = useReducedMotion() ?? false;

  return (
    <div
      aria-hidden
      className={cn(
        "pointer-events-none absolute inset-0 [@media(max-height:40rem)]:hidden",
        className,
      )}
      data-testid="backup-password-timeline"
    >
      {mode === "restore" ? (
        <div
          className="absolute left-1/2 top-0 -translate-x-1/2 text-foreground/85"
          data-testid="restore-ncryptsec-affordance"
        >
          <FileKey2 className="size-10" />
        </div>
      ) : (
        <div className="absolute inset-x-0 top-4 flex items-center justify-center gap-2">
          {BACKUP_KEY_DOTS.map((dot) => (
            <span
              className="block size-2 rounded-full bg-foreground/85"
              key={dot}
            />
          ))}
        </div>
      )}
      <div
        className={cn(
          "absolute left-1/2 flex h-12 -translate-x-1/2 flex-col justify-between",
          mode === "restore" ? "top-15" : "top-11",
        )}
      >
        {TIMELINE_CONNECTOR_DOTS.map((dot, index) => (
          <motion.span
            animate={reduceMotion ? undefined : TIMELINE_DOT_PULSE}
            className="block size-1.5 rounded-full bg-foreground/65"
            initial={reduceMotion ? false : TIMELINE_DOT_INITIAL}
            key={`top-${dot}`}
            transition={
              reduceMotion ? undefined : TIMELINE_TOP_DOT_TRANSITIONS[index]
            }
          />
        ))}
      </div>
      <div className="absolute bottom-15 left-1/2 flex h-12 -translate-x-1/2 flex-col justify-between">
        {TIMELINE_CONNECTOR_DOTS.map((dot, index) => (
          <motion.span
            animate={reduceMotion ? undefined : TIMELINE_DOT_PULSE}
            className="block size-1.5 rounded-full bg-foreground/65"
            initial={reduceMotion ? false : TIMELINE_DOT_INITIAL}
            key={`bottom-${dot}`}
            transition={
              reduceMotion ? undefined : TIMELINE_BOTTOM_DOT_TRANSITIONS[index]
            }
          />
        ))}
      </div>
      {mode === "restore" ? (
        <LockOpen
          className="absolute bottom-0 left-1/2 size-10 -translate-x-1/2 text-foreground/85"
          data-testid="restore-unlock-icon"
        />
      ) : (
        <LockKeyhole className="absolute bottom-0 left-1/2 size-10 -translate-x-1/2 text-foreground/85" />
      )}
    </div>
  );
}
