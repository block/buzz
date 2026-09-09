import "./work.css";
import {
  IconCheck,
  IconCircle,
  IconCircleDashed,
  IconExclamationCircle,
  IconMessageCircle,
  IconSparkles,
  IconTerminal2,
} from "@tabler/icons-react";
import { ShimmerText } from "@/shared/ui/ShimmerText";
import { Accordion } from "@/shared/ui/Accordion";
import type { WorkStepPresentation } from "./workPresentation";

/** One prepared activity step with optional, keyboard-accessible tool output. */
export function WorkStep({
  step,
  shimmer = false,
}: {
  step: WorkStepPresentation;
  shimmer?: boolean;
}) {
  const title = shimmer ? <ShimmerText>{step.title}</ShimmerText> : step.title;
  const Icon =
    step.status === "unavailable"
      ? IconCircle
      : step.status === "failed"
        ? IconExclamationCircle
        : step.status === "running"
          ? IconCircleDashed
          : step.kind === "thought"
            ? IconSparkles
            : step.kind === "progress"
              ? IconMessageCircle
              : IconCheck;
  return (
    <li
      className="work-step"
      data-status={step.status}
      aria-label={`${step.title}, ${step.status}`}
    >
      <span className="work-step-mark" aria-hidden="true">
        <Icon size={16} stroke={1.6} />
      </span>
      <div className="work-step-body">
        {step.detail ? (
          <Accordion
            variant="activity"
            items={[
              {
                value: step.id,
                title,
                content: (
                  <div className="work-tool-output">
                    <div className="work-tool-output-label text-body-sm text-secondary">
                      <IconTerminal2 size={14} aria-hidden="true" /> Tool output
                    </div>
                    <pre className="text-mono-sm text-secondary">
                      {step.detail}
                    </pre>
                  </div>
                ),
              },
            ]}
          />
        ) : (
          <p className="work-step-copy text-body text-secondary">{title}</p>
        )}
        {step.status === "unavailable" ? (
          <span className="text-body-sm text-secondary">
            Outcome unconfirmed
          </span>
        ) : null}
        {step.status === "failed" ? (
          <span className="text-body-sm text-red-12">Failed</span>
        ) : null}
      </div>
    </li>
  );
}
