import { AnimatePresence, motion, useReducedMotion } from "motion/react";

import {
  type BackgroundMediaUploadPhase,
  backgroundMediaUploadPhaseLabel,
} from "@/features/messages/lib/backgroundMediaUploadPhase";
import { cn } from "@/shared/lib/cn";
import { Spinner } from "@/shared/ui/spinner";

export function ComposerUploadProgressPill({
  canCancel,
  isUploading,
  onCancel,
  phase,
  percentage,
}: {
  canCancel: boolean;
  isUploading: boolean;
  onCancel: () => void;
  phase: BackgroundMediaUploadPhase;
  percentage: number;
}) {
  const reducedMotion = useReducedMotion();
  const phaseLabel = backgroundMediaUploadPhaseLabel(phase);
  const isTransferring = phase === "uploading";
  const phaseTransition = reducedMotion
    ? { duration: 0 }
    : { duration: 0.18, ease: [0.77, 0, 0.175, 1] as const };

  return (
    <AnimatePresence initial={false}>
      {isUploading ? (
        <motion.div
          animate="visible"
          className="relative z-0 flex justify-center pb-2"
          data-testid="composer-upload-progress-motion"
          exit="hidden"
          initial="hidden"
          variants={{
            hidden: {
              opacity: reducedMotion ? 1 : 0,
              transition: reducedMotion
                ? { duration: 0 }
                : { duration: 0.18, ease: "easeOut" },
              y: reducedMotion ? 0 : 28,
            },
            visible: {
              opacity: 1,
              transition: reducedMotion
                ? { duration: 0 }
                : { duration: 0.22, ease: "easeIn" },
              y: 0,
            },
          }}
        >
          <div
            aria-label={
              isTransferring ? `${phaseLabel} ${percentage}%` : phaseLabel
            }
            aria-live="polite"
            className="relative h-9 w-[18.75rem] max-w-full overflow-hidden rounded-full border border-primary bg-primary text-primary-foreground"
            data-testid="composer-upload-progress"
            role="status"
          >
            <motion.div
              animate={{ width: `${percentage}%` }}
              className="absolute inset-y-0 left-0 bg-primary-foreground/20"
              data-testid="composer-upload-progress-fill"
              initial={false}
              transition={
                reducedMotion
                  ? { duration: 0 }
                  : { duration: 0.15, ease: "easeOut" }
              }
            />
            <div className="relative flex h-full items-center gap-2 pl-3 pr-1">
              <span className="min-w-0 flex-1 truncate text-sm font-semibold text-primary-foreground">
                <span className="inline-flex items-baseline">
                  <AnimatePresence initial={false} mode="popLayout">
                    <motion.span
                      animate={{ opacity: 1, y: 0 }}
                      exit={{
                        opacity: reducedMotion ? 1 : 0,
                        y: reducedMotion ? 0 : -2,
                      }}
                      initial={{
                        opacity: reducedMotion ? 1 : 0,
                        y: reducedMotion ? 0 : 2,
                      }}
                      data-testid="composer-upload-phase"
                      key={phase}
                      layout="position"
                      transition={phaseTransition}
                    >
                      {phaseLabel}
                    </motion.span>
                  </AnimatePresence>
                  <motion.span
                    className="ml-1 inline-flex items-center text-primary-foreground/80"
                    data-testid="composer-upload-status"
                    layout="position"
                    transition={phaseTransition}
                  >
                    <span aria-hidden="true">·</span>
                    <AnimatePresence initial={false} mode="popLayout">
                      {isTransferring ? (
                        <motion.span
                          animate={{ opacity: 1, y: 0 }}
                          className="ml-1"
                          data-testid="composer-upload-percentage"
                          exit={{
                            opacity: reducedMotion ? 1 : 0,
                            y: reducedMotion ? 0 : -2,
                          }}
                          initial={{
                            opacity: reducedMotion ? 1 : 0,
                            y: reducedMotion ? 0 : 2,
                          }}
                          key="percentage"
                          layout="position"
                          transition={phaseTransition}
                        >
                          {percentage}%
                        </motion.span>
                      ) : (
                        <motion.span
                          animate={{ opacity: 1, y: 0 }}
                          className="ml-1 inline-flex"
                          data-testid="composer-upload-spinner"
                          exit={{
                            opacity: reducedMotion ? 1 : 0,
                            y: reducedMotion ? 0 : -2,
                          }}
                          initial={{
                            opacity: reducedMotion ? 1 : 0,
                            y: reducedMotion ? 0 : 2,
                          }}
                          key="spinner"
                          layout="position"
                          transition={phaseTransition}
                        >
                          <Spinner
                            aria-hidden="true"
                            className="size-3.5 border-2"
                          />
                        </motion.span>
                      )}
                    </AnimatePresence>
                  </motion.span>
                </span>
              </span>
              {canCancel ? (
                <button
                  className={cn(
                    "shrink-0 rounded-full bg-transparent px-2 py-1 text-sm font-semibold text-primary-foreground",
                    "transition-colors hover:bg-primary-foreground/20 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-primary-foreground",
                  )}
                  data-testid="composer-upload-cancel"
                  onClick={onCancel}
                  type="button"
                >
                  Cancel
                </button>
              ) : null}
            </div>
          </div>
        </motion.div>
      ) : null}
    </AnimatePresence>
  );
}
