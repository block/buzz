import "./work.css";
import { IconAlertCircle, IconLock } from "@tabler/icons-react";
import { useState } from "react";
import { Accordion } from "@/shared/ui/Accordion";
import { Avatar } from "@/shared/ui/Avatar";
import { WorkStep } from "./WorkStep";
import type { AgentWorkPresentation } from "./workPresentation";

/**
 * A stable, owner-private place for one agent's work in an exchange.
 * Adapts Berd AgentWorkPanel's active window / previous steps / settled
 * disclosure anatomy. Parsing, ordering and visibility stay with the owner;
 * no ACP decoding, transcript store, or completion-remount effect is copied.
 * Published answers are deliberately not accepted here.
 */
export function AgentWorkArea({ work }: { work: AgentWorkPresentation }) {
  // Remember user disclosure choices within this work identity. A transition
  // to completed defaults closed without synchronizing state through an Effect.
  const [expanded, setExpanded] = useState<{
    status: string;
    values: string[];
  } | null>(null);
  const active = work.status === "running";
  const count = work.earlierSteps.length + work.visibleSteps.length;
  const steps = (items: AgentWorkPresentation["visibleSteps"]) => (
    <ol className="work-steps">
      {items.map((step) => (
        <WorkStep
          key={step.id}
          step={step}
          shimmer={active && step.id === work.visibleSteps.at(-1)?.id}
        />
      ))}
    </ol>
  );
  return (
    <section
      className="agent-work-area"
      aria-label={`${work.agent.name} activity`}
      data-work-id={work.id}
    >
      <header className="agent-work-identity">
        <span aria-hidden="true">
          <Avatar
            size="small"
            src={work.agent.avatar}
            alt=""
            fallback={work.agent.fallback}
          />
        </span>
        <span className="text-body text-primary">{work.agent.name}</span>
        <span className="sr-only" role="status">
          {work.statusLabel}
        </span>
        <span className="agent-work-privacy text-body-sm text-tertiary">
          <IconLock size={12} aria-hidden="true" />
          Only you
        </span>
      </header>
      <div className="agent-work-content">
        {active ? (
          <div className="work-sequence">
            {work.earlierSteps.length ? (
              <div className="work-history">
                <Accordion
                  variant="activity"
                  items={[
                    {
                      value: "earlier",
                      title: `${work.earlierSteps.length} earlier ${work.earlierSteps.length === 1 ? "step" : "steps"}`,
                      content: steps(work.earlierSteps),
                    },
                  ]}
                />
              </div>
            ) : null}
            {steps(work.visibleSteps)}
          </div>
        ) : count ? (
          <Accordion
            variant="activity"
            value={expanded?.status === work.status ? expanded.values : []}
            onValueChange={(values) =>
              setExpanded({ status: work.status, values })
            }
            items={[
              {
                value: "work",
                title: `${count} steps · ${work.summary}`,
                content: (
                  <div className="work-sequence">
                    {steps([...work.earlierSteps, ...work.visibleSteps])}
                  </div>
                ),
              },
            ]}
          />
        ) : (
          <p className="text-body text-secondary">{work.summary}</p>
        )}
        {work.notice ? (
          <div className="work-notice" role="note">
            <IconAlertCircle size={16} aria-hidden="true" />
            <div>
              <p className="text-body text-primary">{work.notice.title}</p>
              <p className="text-body-sm text-secondary">
                {work.notice.description}
              </p>
              {work.notice.actions ? (
                <div className="work-notice-actions">{work.notice.actions}</div>
              ) : null}
            </div>
          </div>
        ) : null}
        {work.artifacts?.length ? (
          <div className="work-artifacts">
            {work.artifacts.map((artifact) => (
              <Accordion
                key={artifact.id}
                variant="activity"
                items={[
                  {
                    value: artifact.id,
                    title: artifact.name,
                    content: artifact.preview,
                  },
                ]}
              />
            ))}
          </div>
        ) : null}
      </div>
    </section>
  );
}
