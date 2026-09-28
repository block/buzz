import { Plus, Trash2 } from "lucide-react";
import * as React from "react";

import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import {
  addBestieJobForScope,
  removeBestieJobForScope,
  updateBestieJobForScope,
  useBestieJobs,
} from "./bestieJobStore";
import type {
  BestieJob,
  BestieJobScope,
  BestieJobSchedule,
} from "./bestieJobTypes";
import {
  dueAtFromDatetimeLocal,
  presentBestieJobSchedule,
} from "./bestieDmRhsHelpers";
import { BestieDueCountdownChip } from "./BestieDueCountdownChip";

function JobRow({
  job,
  onRemove,
  onToggleEnabled,
}: {
  job: BestieJob;
  onRemove: () => void;
  onToggleEnabled: () => void;
}) {
  return (
    <div
      className={cn(
        "group flex flex-col gap-1 rounded-md border border-border/60 bg-muted/25 px-2 py-1.5",
        !job.enabled && "opacity-60",
      )}
      data-testid={`bestie-job-item-${job.id}`}
    >
      <div className="flex items-start gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex items-start gap-1.5">
            <p className="min-w-0 flex-1 text-sm font-medium leading-snug">
              {job.title}
            </p>
            {job.enabled && job.nextDueAt != null ? (
              <BestieDueCountdownChip
                dueAt={job.nextDueAt}
                testId={`bestie-job-due-chip-${job.id}`}
              />
            ) : null}
          </div>
          <p className="mt-0.5 line-clamp-2 text-2xs text-muted-foreground">
            {job.prompt}
          </p>
          <p className="mt-0.5 text-2xs text-muted-foreground">
            {presentBestieJobSchedule(job.schedule)}
            {job.nextDueAt != null
              ? ` · next ${new Date(job.nextDueAt * 1000).toLocaleString()}`
              : ""}
          </p>
        </div>
        <Button
          aria-label={job.enabled ? "Disable job" : "Enable job"}
          className="mt-0.5 h-7 shrink-0 px-2 text-xs"
          data-testid={`bestie-job-toggle-${job.id}`}
          onClick={onToggleEnabled}
          size="sm"
          type="button"
          variant="ghost"
        >
          {job.enabled ? "On" : "Off"}
        </Button>
        <Button
          aria-label="Remove job"
          className="mt-0.5 size-6 shrink-0 opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
          data-testid={`bestie-job-remove-${job.id}`}
          onClick={onRemove}
          size="icon-xs"
          type="button"
          variant="ghost"
        >
          <Trash2 className="size-3.5" />
        </Button>
      </div>
    </div>
  );
}

function AddJobRow({
  onAdd,
}: {
  onAdd: (input: {
    dueLocal: string;
    everyMinutes: string;
    prompt: string;
    scheduleKind: "once" | "interval" | "daily";
    title: string;
  }) => void;
}) {
  const [title, setTitle] = React.useState("");
  const [prompt, setPrompt] = React.useState("");
  const [scheduleKind, setScheduleKind] = React.useState<
    "once" | "interval" | "daily"
  >("once");
  const [dueLocal, setDueLocal] = React.useState("");
  const [everyMinutes, setEveryMinutes] = React.useState("60");
  const titleRef = React.useRef<HTMLInputElement>(null);
  React.useEffect(() => {
    titleRef.current?.focus();
  }, []);

  const submit = () => {
    if (!title.trim() || !prompt.trim()) return;
    onAdd({ dueLocal, everyMinutes, prompt, scheduleKind, title });
    setTitle("");
    setPrompt("");
    setDueLocal("");
  };

  return (
    <div className="flex flex-col gap-1.5" data-testid="bestie-add-job">
      <Input
        aria-label="Job title"
        className="h-8 text-sm"
        onChange={(event) => setTitle(event.target.value)}
        placeholder="Title"
        ref={titleRef}
        value={title}
      />
      <Input
        aria-label="Job prompt"
        className="h-8 text-sm"
        onChange={(event) => setPrompt(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            submit();
          }
        }}
        placeholder="Prompt Assistant should run"
        value={prompt}
      />
      <div className="flex flex-wrap items-center gap-1.5">
        <select
          aria-label="Schedule kind"
          className="h-8 rounded-md border border-input bg-background px-2 text-xs"
          data-testid="bestie-add-job-schedule-kind"
          onChange={(event) =>
            setScheduleKind(event.target.value as "once" | "interval" | "daily")
          }
          value={scheduleKind}
        >
          <option value="once">One-time</option>
          <option value="interval">Every N min</option>
          <option value="daily">Daily</option>
        </select>
        {scheduleKind === "once" ? (
          <Input
            aria-label="Due date and time"
            className="h-8 flex-1 text-sm"
            data-testid="bestie-add-job-due"
            onChange={(event) => setDueLocal(event.target.value)}
            type="datetime-local"
            value={dueLocal}
          />
        ) : null}
        {scheduleKind === "interval" ? (
          <Input
            aria-label="Interval minutes"
            className="h-8 w-24 text-sm"
            data-testid="bestie-add-job-interval"
            min={1}
            onChange={(event) => setEveryMinutes(event.target.value)}
            type="number"
            value={everyMinutes}
          />
        ) : null}
        {scheduleKind === "daily" ? (
          <Input
            aria-label="Daily time"
            className="h-8 flex-1 text-sm"
            data-testid="bestie-add-job-daily-time"
            onChange={(event) => setDueLocal(event.target.value)}
            type="time"
            value={dueLocal}
          />
        ) : null}
        <Button
          aria-label="Add job"
          className="size-8 shrink-0"
          disabled={!title.trim() || !prompt.trim()}
          onClick={submit}
          size="icon"
          type="button"
          variant="secondary"
        >
          <Plus className="size-4" />
        </Button>
      </div>
    </div>
  );
}

function buildSchedule(input: {
  dueLocal: string;
  everyMinutes: string;
  scheduleKind: "once" | "interval" | "daily";
}): BestieJobSchedule | null {
  if (input.scheduleKind === "once") {
    const dueAt = dueAtFromDatetimeLocal(input.dueLocal);
    if (dueAt == null) {
      return { dueAt: Math.floor(Date.now() / 1000) + 300, kind: "once" };
    }
    return { dueAt, kind: "once" };
  }
  if (input.scheduleKind === "interval") {
    const minutes = Number(input.everyMinutes);
    if (!Number.isFinite(minutes) || minutes < 1) return null;
    return { everySeconds: Math.floor(minutes * 60), kind: "interval" };
  }
  // daily from time input HH:MM
  const match = input.dueLocal.trim().match(/^(\d{1,2}):(\d{2})$/);
  if (!match) {
    return { hour: 9, kind: "daily", minute: 0 };
  }
  return {
    hour: Number(match[1]),
    kind: "daily",
    minute: Number(match[2]),
  };
}

/** Slide sheet for Bestie Jobs — create, enable/disable, remove. */
export function BestieDmJobsSheet({
  adding,
  onRequestAdd,
  scope,
}: {
  adding: boolean;
  onRequestAdd?: () => void;
  scope: BestieJobScope;
}) {
  const state = useBestieJobs(scope);

  return (
    <div
      className="flex flex-col gap-3 py-1"
      data-testid="bestie-dm-category-sheet-job"
    >
      {adding ? (
        <AddJobRow
          onAdd={(input) => {
            const schedule = buildSchedule(input);
            if (!schedule) return;
            addBestieJobForScope(scope, {
              prompt: input.prompt,
              schedule,
              title: input.title,
            });
          }}
        />
      ) : (
        <p className="px-0.5 text-xs text-muted-foreground">
          Use + to add a job, or ask Assistant to schedule one. When due, Assistant
          runs the prompt as a turn (not just a nudge).{" "}
          {onRequestAdd ? (
            <button
              className="underline-offset-2 hover:underline"
              onClick={onRequestAdd}
              type="button"
            >
              Add one
            </button>
          ) : null}
        </p>
      )}
      <div className="space-y-1.5">
        {state.jobs.length === 0 ? (
          <p className="px-0.5 text-xs text-muted-foreground">No jobs yet.</p>
        ) : (
          state.jobs.map((job) => (
            <JobRow
              key={job.id}
              job={job}
              onRemove={() => removeBestieJobForScope(scope, job.id)}
              onToggleEnabled={() =>
                updateBestieJobForScope(scope, {
                  enabled: !job.enabled,
                  id: job.id,
                })
              }
            />
          ))
        )}
      </div>
    </div>
  );
}
